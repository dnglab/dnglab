// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Yongda Fan <fanyongda2012@gmail.com>

//! The Menon demosiac algorithm for Bayer CFA
//!
//! Menon D, Andriani S, Calvagno G. Demosaicing with directional filtering and a posteriori decision.
//! IEEE Trans Image Process. 2007 Jan;16(1):132-41. doi: 10.1109/tip.2006.884928. PMID: 17283772.

use rayon::iter::{IntoParallelIterator, ParallelBridge, ParallelIterator};
use std::time::Instant;

use crate::{
  cfa::{CFA, CFA_COLOR_B, CFA_COLOR_G, CFA_COLOR_R, PlaneColor},
  imgop::{
    Dim2, Rect,
    sensor::{
      Demosaic,
      tiles::{DemosaicTiler, Tile},
    },
  },
  pixarray::{BorderPadding, Color2D, Pix2DView, PixF32, PixU16, RgbF32},
};

const TILE_HALO: Dim2 = Dim2 { w: 8, h: 8 };
const TILE_SIZE: Dim2 = Dim2 { w: 256, h: 256 };

#[derive(Default)]
pub struct MenonDemosaic;

impl MenonDemosaic {
  pub fn new() -> Self {
    MenonDemosaic
  }
}

impl Demosaic<f32, 3> for MenonDemosaic {
  fn demosaic(&self, pixels: &PixF32, cfa: &CFA, _colors: &PlaneColor, roi: Rect) -> Color2D<f32, 3> {
    // Menon can only applied to pure RGGB or variants.
    if !cfa.is_rgb() {
      panic!("CFA pattern '{}' is not a RGB pattern, can not demosaic with Menon", cfa);
    }

    let now = Instant::now();

    let cfa_roi = cfa.shift(roi.p.x, roi.p.y);
    let input = pixels.view(roi);
    let mut rgb = RgbF32::new(roi.width(), roi.height());

    DemosaicTiler::from_view(input, &cfa_roi, &mut rgb, TILE_SIZE, TILE_HALO)
      .par_bridge()
      .into_par_iter()
      .for_each(|mut tile| {
        let raw = tile.raw();
        let cfa = tile.cfa();

        let mut gh = PixF32::new(raw.width(), raw.height());
        let mut gv = PixF32::new(raw.width(), raw.height());
        build_gh_gv(&raw, &cfa, &mut gh, &mut gv);

        let mut dir = PixU16::new(raw.width(), raw.height());
        pick_direction(&raw, &cfa, &gh, &gv, &mut dir);

        let mut rgb = RgbF32::new(raw.width(), raw.height());
        fill_green(&raw, &cfa, &gh, &gv, &dir, &mut rgb);
        fill_chroma(&raw, &cfa, &dir, &mut rgb);
        refine(&cfa, &dir, &mut rgb);

        copy_to_tile(&rgb, &mut tile);
      });

    log::debug!("Menon total debayer time: {:.5}s", now.elapsed().as_secs_f32());
    rgb
  }
}

fn build_gh_gv(raw: &Pix2DView<f32>, cfa: &CFA, gh: &mut PixF32, gv: &mut PixF32) {
  const OFFSETS: [isize; 5] = [-2, -1, 0, 1, 2];
  const WEIGHTS: [f32; 5] = [-0.25, 0.5, 0.5, 0.5, -0.25];

  for y in 0..raw.height() {
    for x in 0..raw.width() {
      if cfa.color_at(y, x) != CFA_COLOR_G {
        let (iy, ix) = (y as isize, x as isize);

        let mut gh_acc = 0.0;
        let mut gv_acc = 0.0;
        for i in 0..5 {
          let value = raw.at_reflect(iy, ix + OFFSETS[i], false);
          gh_acc += WEIGHTS[i] * value;

          let value = raw.at_reflect(iy + OFFSETS[i], ix, false);
          gv_acc += WEIGHTS[i] * value;
        }

        *gh.at_mut(y, x) = gh_acc;
        *gv.at_mut(y, x) = gv_acc;
      }
    }
  }
}

fn pick_direction(raw: &Pix2DView<f32>, cfa: &CFA, gh: &PixF32, gv: &PixF32, dir: &mut PixU16) {
  static ZERO: f32 = 0.0;

  const KX: [isize; 8] = [0, -2, -1, 0, -2, -1, 0, -2];
  const KY: [isize; 8] = [2, 2, 1, 0, 0, -1, -2, -2];
  const KW: [f32; 8] = [1.0, 1.0, 1.0, 3.0, 3.0, 1.0, 1.0, 1.0];

  let dh = {
    let mut buf1 = PixF32::new(raw.width(), raw.height());
    let mut buf2 = PixF32::new(raw.width(), raw.height());

    for y in 0..raw.height() {
      for x in 0..raw.width() {
        if cfa.color_at(y, x) != CFA_COLOR_G {
          let (iy, ix) = (y as isize, x as isize);

          let delta0 = raw.at_reflect(iy, ix, false) - gh.at_reflect(iy, ix, false);
          let delta2 = raw.at_reflect(iy, ix + 2, false) - gh.at_reflect(iy, ix + 2, false);
          *buf1.at_mut(y, x) = (delta0 - delta2).abs();
        }
      }
    }

    for y in 0..raw.height() {
      for x in 0..raw.width() {
        if cfa.color_at(y, x) != CFA_COLOR_G {
          let (iy, ix) = (y as isize, x as isize);

          let mut acc = 0.0;
          for i in 0..8 {
            let value = *buf1.at_padding(iy + KY[i], ix + KX[i], &ZERO);
            acc += KW[i] * value;
          }
          *buf2.at_mut(y, x) = acc;
        }
      }
    }

    buf2
  };

  let dv = {
    let mut buf1 = PixF32::new(raw.width(), raw.height());
    let mut buf2 = PixF32::new(raw.width(), raw.height());

    for y in 0..raw.height() {
      for x in 0..raw.width() {
        if cfa.color_at(y, x) != CFA_COLOR_G {
          let (iy, ix) = (y as isize, x as isize);

          let delta0 = raw.at_reflect(iy, ix, false) - gv.at_reflect(iy, ix, false);
          let delta2 = raw.at_reflect(iy + 2, ix, false) - gv.at_reflect(iy + 2, ix, false);
          *buf1.at_mut(y, x) = (delta0 - delta2).abs();
        }
      }
    }

    for y in 0..raw.height() {
      for x in 0..raw.width() {
        if cfa.color_at(y, x) != CFA_COLOR_G {
          let (iy, ix) = (y as isize, x as isize);

          let mut acc = 0.0;
          for i in 0..8 {
            let value = *buf1.at_padding(iy + KX[i], ix + KY[i], &ZERO);
            acc += KW[i] * value;
          }
          *buf2.at_mut(y, x) = acc;
        }
      }
    }

    buf2
  };

  for y in 0..raw.height() {
    for x in 0..raw.width() {
      if cfa.color_at(y, x) != CFA_COLOR_G {
        let value = if dv.at(y, x) >= dh.at(y, x) { 1 } else { 0 };
        *dir.at_mut(y, x) = value;
      }
    }
  }
}

fn fill_green(raw: &Pix2DView<f32>, cfa: &CFA, gh: &PixF32, gv: &PixF32, dir: &PixU16, rgb: &mut RgbF32) {
  for y in 0..raw.height() {
    for x in 0..raw.width() {
      if cfa.color_at(y, x) == CFA_COLOR_G {
        rgb.at_mut(y, x)[CFA_COLOR_G] = *raw.at(y, x);
      } else if *dir.at(y, x) == 1 {
        rgb.at_mut(y, x)[CFA_COLOR_G] = *gh.at(y, x);
      } else {
        rgb.at_mut(y, x)[CFA_COLOR_G] = *gv.at(y, x);
      }
    }
  }
}

fn fill_chroma(raw: &Pix2DView<f32>, cfa: &CFA, dir: &PixU16, rgb: &mut RgbF32) {
  for y in 0..raw.height() {
    for x in 0..raw.width() {
      if cfa.color_at(y, x) != CFA_COLOR_G {
        rgb.at_mut(y, x)[cfa.color_at(y, x)] = *raw.at(y, x);
      } else {
        let (iy, ix) = (y as isize, x as isize);
        let cfa_h = cfa.color_at(y, x + 1);
        let cfa_v = cfa.color_at(y + 1, x);

        let mut acc_h = rgb.at(y, x)[CFA_COLOR_G];
        let mut acc_v = rgb.at(y, x)[CFA_COLOR_G];
        for i in [-1, 1] {
          acc_h += (*raw.at_reflect(iy, ix + i, false) - rgb.at_reflect(iy, ix + i, false)[CFA_COLOR_G]) / 2.0;
          acc_v += (*raw.at_reflect(iy + i, ix, false) - rgb.at_reflect(iy + i, ix, false)[CFA_COLOR_G]) / 2.0;
        }

        rgb.at_mut(y, x)[cfa_h] = acc_h;
        rgb.at_mut(y, x)[cfa_v] = acc_v;
      }
    }
  }

  for y in 0..raw.height() {
    for x in 0..raw.width() {
      if cfa.color_at(y, x) != CFA_COLOR_G {
        let (iy, ix) = (y as isize, x as isize);
        let cfa_src = cfa.color_at(y, x);
        let cfa_trg = if cfa_src == CFA_COLOR_R { CFA_COLOR_B } else { CFA_COLOR_R };

        let mut acc = rgb.at(y, x)[cfa_src];
        for i in [-1, 1] {
          if *dir.at(y, x) == 1 {
            acc += (rgb.at_reflect(iy, ix + i, false)[cfa_trg] - rgb.at_reflect(iy, ix + i, false)[cfa_src]) / 2.0;
          } else {
            acc += (rgb.at_reflect(iy + i, ix, false)[cfa_trg] - rgb.at_reflect(iy + i, ix, false)[cfa_src]) / 2.0;
          }
        }

        rgb.at_mut(y, x)[cfa_trg] = acc;
      }
    }
  }
}

fn refine(cfa: &CFA, dir: &PixU16, rgb: &mut RgbF32) {
  for y in 0..rgb.height {
    for x in 0..rgb.width {
      if cfa.color_at(y, x) != CFA_COLOR_G {
        let (iy, ix) = (y as isize, x as isize);
        let cfa_src = cfa.color_at(y, x);

        let mut acc = rgb.at(y, x)[cfa_src];
        for i in [-1, 0, 1] {
          if *dir.at(y, x) == 1 {
            acc += (rgb.at_reflect(iy, ix + i, false)[CFA_COLOR_G] - rgb.at_reflect(iy, ix + i, false)[cfa_src]) / 3.0;
          } else {
            acc += (rgb.at_reflect(iy + i, ix, false)[CFA_COLOR_G] - rgb.at_reflect(iy + i, ix, false)[cfa_src]) / 3.0;
          }
        }
        rgb.at_mut(y, x)[CFA_COLOR_G] = acc;
      }
    }
  }

  for y in 0..rgb.height {
    for x in 0..rgb.width {
      if cfa.color_at(y, x) == CFA_COLOR_G {
        let (iy, ix) = (y as isize, x as isize);
        let cfa_h = cfa.color_at(y, x + 1);
        let cfa_v = cfa.color_at(y + 1, x);

        let mut acc_h = rgb.at(y, x)[CFA_COLOR_G];
        let mut acc_v = rgb.at(y, x)[CFA_COLOR_G];
        for i in [-1, 1] {
          acc_h += (rgb.at_reflect(iy, ix + i, false)[cfa_h] - rgb.at_reflect(iy, ix + i, false)[CFA_COLOR_G]) / 2.0;
          acc_v += (rgb.at_reflect(iy + i, ix, false)[cfa_v] - rgb.at_reflect(iy + i, ix, false)[CFA_COLOR_G]) / 2.0;
        }

        rgb.at_mut(y, x)[cfa_h] = acc_h;
        rgb.at_mut(y, x)[cfa_v] = acc_v;
      }
    }
  }

  for y in 0..rgb.height {
    for x in 0..rgb.width {
      if cfa.color_at(y, x) != CFA_COLOR_G {
        let (iy, ix) = (y as isize, x as isize);
        let cfa_src = cfa.color_at(y, x);
        let cfa_trg = if cfa_src == CFA_COLOR_R { CFA_COLOR_B } else { CFA_COLOR_R };

        let mut acc = rgb.at(y, x)[cfa_src];
        for i in [-1, 0, 1] {
          if *dir.at(y, x) == 1 {
            acc += (rgb.at_reflect(iy, ix + i, false)[cfa_trg] - rgb.at_reflect(iy, ix + i, false)[cfa_src]) / 3.0;
          } else {
            acc += (rgb.at_reflect(iy + i, ix, false)[cfa_trg] - rgb.at_reflect(iy + i, ix, false)[cfa_src]) / 3.0;
          }
        }
        rgb.at_mut(y, x)[cfa_trg] = acc;
      }
    }
  }
}

fn copy_to_tile(rgb: &RgbF32, tile: &mut Tile) {
  for y in 0..rgb.height {
    for x in 0..rgb.width {
      *tile.rgb_mut(y, x) = *rgb.at(y, x);
    }
  }
}
