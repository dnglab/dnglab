// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

use std::time::Instant;

use multiversion::multiversion;

use crate::{
  cfa::{CFA, CFA_COLOR_G, PlaneColor},
  imgop::{
    Dim2,
    sensor::{
      Demosaic,
      border::interpolate_border,
      check_bayer_rgb, check_mosaic,
      lab::rgb_to_lab,
      tiles::{HaloTile, halo_tiles, inner_area, par_tiles},
    },
  },
  pixarray::{Color2D, DisjointRegionMut, ImageRegion, ImageRegionMut},
};

/// Pixels at the edges that AHD leaves to [`interpolate_border`], and the halo
/// of its tiles: the reach of its five steps
const HALO: usize = 5;

/// Width and height of the windows of the tiles, with the halo
const TILE: usize = 256;

/// AHD demosaic of Bayer RGB data
///
/// The Adaptive Homogeneity-Directed algorithm of Keigo Hirakawa and Thomas
/// W. Parks ("Adaptive homogeneity-directed demosaicing algorithm", IEEE
/// Transactions on Image Processing 14(3), 2005), in the form of the
/// `ahd_interpolate` of dcraw:
///
/// 1. Green is interpolated horizontally and vertically, so there are two
///    candidate images.
/// 2. Red and blue of each candidate are interpolated from their differences
///    to its green, and the candidates are converted to CIELab.
/// 3. For each candidate and pixel, the homogeneity is the number of its 4
///    neighbours whose distance in lightness and in color are both within the
///    smaller of the distances that the candidates have along their direction.
/// 4. Each pixel is taken from the candidate with the larger homogeneity in
///    its 3x3 neighbourhood, or is the mean of both on a tie.
///
/// So it interpolates along edges, which avoids the zipper artifacts of
/// interpolations across them. Like the color differences it interpolates, it
/// works best on white balanced data.
///
/// The image is processed in tiles with a halo of 5 pixels in parallel, the 5
/// pixels at the edges are interpolated bilinearly ([`interpolate_border`]).
///
/// # Errors
///
/// Fails for CFA patterns that are not RGGB or variants.
pub struct AhdDemosaic {
  /// Size of the cores of the tiles, the area each tile writes
  core: Dim2,
}

impl Default for AhdDemosaic {
  fn default() -> Self {
    Self::new()
  }
}

impl AhdDemosaic {
  pub fn new() -> Self {
    Self::with_tile(TILE)
  }

  /// Demosaic in tiles of `tile` pixels (with the halo)
  fn with_tile(tile: usize) -> Self {
    assert!(tile > 2 * HALO, "Tile of {} pixels has no core", tile);
    Self {
      core: Dim2::new(tile - 2 * HALO, tile - 2 * HALO),
    }
  }
}

impl Demosaic<f32, 3> for AhdDemosaic {
  fn demosaic(&self, pixels: ImageRegion<'_, f32>, cfa: &CFA, colors: &PlaneColor) -> crate::Result<Color2D<f32, 3>> {
    check_mosaic::<3, f32>("AHD", &pixels, cfa, colors)?;
    check_bayer_rgb("AHD", cfa)?;
    let now = Instant::now();
    // The ROI changes the pattern if not perfectly aligned on the origin pattern
    let roi = pixels.area();
    let cfa_roi = cfa.shift(roi.p.x, roi.p.y);

    let mut out = Color2D::<f32, 3>::new(roi.width(), roi.height());
    interpolate_border(pixels, cfa, colors, &mut out, HALO)?;
    let tiles = halo_tiles(roi.d, inner_area(roi.d, HALO), self.core, HALO)?;
    par_tiles(ImageRegionMut::from(out.view_mut()), &tiles, Buffers::default, |buffers, tile, region| {
      ahd_tile(pixels, &cfa_roi, tile, buffers, region);
    })?;
    log::debug!("AHD debayer time: {:.5}s", now.elapsed().as_secs_f32());
    Ok(out)
  }
}

/// Buffers of a tile, reused for the following tiles of a worker
///
/// They are not cleared between the tiles: each step reads only values that
/// the steps before have written for the tile.
#[derive(Default)]
struct Buffers {
  /// Samples of the window
  raw: Vec<f32>,
  /// Horizontal and vertical candidates
  rgb: [Vec<[f32; 3]>; 2],
  /// The candidates in CIELab
  lab: [Vec<[f32; 3]>; 2],
  /// Homogeneity of the candidates
  homo: [Vec<u8>; 2],
}

/// Demosaic `tile` of `pixels` into `region`, its core
///
/// Each step covers the area of the following step and the pixels next to it,
/// so the steps cover the core and 3, 2, 1 and 0 pixels around it. The first
/// step reads 2 pixels beyond that, which is the window.
#[multiversion(targets("x86_64+avx512f+avx512bw+avx512vl", "x86_64+avx+avx2", "x86_64+sse4.1", "aarch64+neon"))]
fn ahd_tile(pixels: ImageRegion<f32>, cfa_roi: &CFA, tile: &HaloTile, buffers: &mut Buffers, region: &mut DisjointRegionMut<f32>) {
  let (tw, th) = (tile.window.d.w, tile.window.d.h);
  let core = tile.core_in_window();
  // halo_tiles over the inner area with the same halo gives the whole halo
  debug_assert!(
    core.p.x == HALO && core.p.y == HALO && core.d.w + 2 * HALO == tw && core.d.h + 2 * HALO == th,
    "Window of {:?} has no halo of {} pixels",
    tile,
    HALO
  );
  let cfa = cfa_roi.shift(tile.window.p.x, tile.window.p.y);
  let pixel_count = tw * th;
  let Buffers { raw, rgb, lab, homo } = buffers;
  pixels.region(tile.window).copy_into(raw);
  for d in 0..2 {
    rgb[d].resize(pixel_count, [0.0; 3]);
    lab[d].resize(pixel_count, [0.0; 3]);
    homo[d].resize(pixel_count, 0);
  }
  // Offsets of the neighbours in the buffers: left, right, above, below
  let dir = [-1, 1, -(tw as isize), tw as isize];
  let at = |i: usize, offset: isize| i.wrapping_add_signed(offset);

  // Interpolate green horizontally and vertically
  for row in 2..th - 2 {
    for col in 2..tw - 2 {
      let i = row * tw + col;
      let sample = raw[i];
      if cfa.color_at(row, col) == CFA_COLOR_G {
        rgb[0][i][CFA_COLOR_G] = sample;
        rgb[1][i][CFA_COLOR_G] = sample;
        continue;
      }
      for d in 0..2 {
        let step = dir[2 * d + 1];
        let (before, after) = (raw[at(i, -step)], raw[at(i, step)]);
        let value = ((before + sample + after) * 2.0 - raw[at(i, -2 * step)] - raw[at(i, 2 * step)]) / 4.0;
        // Limited to the range of the green neighbours
        rgb[d][i][CFA_COLOR_G] = value.max(before.min(after)).min(before.max(after));
      }
    }
  }

  // Interpolate red and blue by the differences to green, convert to CIELab
  for row in 3..th - 3 {
    for col in 3..tw - 3 {
      let i = row * tw + col;
      let own = cfa.color_at(row, col);
      for d in 0..2 {
        let green = |offset: isize| rgb[d][at(i, offset)][CFA_COLOR_G];
        let mut pixel = rgb[d][i];
        if own == CFA_COLOR_G {
          // The neighbours in the row have one color, in the column the other
          let (hc, vc) = (cfa.color_at(row, col + 1), cfa.color_at(row + 1, col));
          let horizontal = (raw[i - 1] + raw[i + 1] - green(-1) - green(1)) / 2.0;
          let vertical = (raw[i - tw] + raw[i + tw] - green(-(tw as isize)) - green(tw as isize)) / 2.0;
          pixel[hc] = (pixel[CFA_COLOR_G] + horizontal).max(0.0);
          pixel[vc] = (pixel[CFA_COLOR_G] + vertical).max(0.0);
        } else {
          // The diagonal neighbours have the other color
          let other = 2 - own;
          let corners = [-(tw as isize) - 1, -(tw as isize) + 1, tw as isize - 1, tw as isize + 1];
          let difference: f32 = corners.iter().map(|&offset| raw[at(i, offset)] - green(offset)).sum();
          pixel[other] = (pixel[CFA_COLOR_G] + difference / 4.0).max(0.0);
        }
        pixel[own] = raw[i];
        rgb[d][i] = pixel;
        lab[d][i] = rgb_to_lab(&pixel);
      }
    }
  }

  // Build the homogeneity maps from the CIELab candidates
  for row in 4..th - 4 {
    for col in 4..tw - 4 {
      let i = row * tw + col;
      let mut ldiff = [[0.0_f32; 4]; 2];
      let mut abdiff = [[0.0_f32; 4]; 2];
      for d in 0..2 {
        let center = lab[d][i];
        for (k, &offset) in dir.iter().enumerate() {
          let neighbour = lab[d][at(i, offset)];
          ldiff[d][k] = (center[0] - neighbour[0]).abs();
          abdiff[d][k] = (center[1] - neighbour[1]).powi(2) + (center[2] - neighbour[2]).powi(2);
        }
      }
      // The smaller of the distances of the candidates along their direction
      let leps = ldiff[0][0].max(ldiff[0][1]).min(ldiff[1][2].max(ldiff[1][3]));
      let abeps = abdiff[0][0].max(abdiff[0][1]).min(abdiff[1][2].max(abdiff[1][3]));
      for d in 0..2 {
        homo[d][i] = (0..4).filter(|&k| ldiff[d][k] <= leps && abdiff[d][k] <= abeps).count() as u8;
      }
    }
  }

  // Combine the most homogeneous candidates for the core
  for (row, line) in (HALO..).zip(region.rows_mut()) {
    for (col, pixel) in (HALO..).zip(line.as_chunks_mut::<3>().0) {
      let i = row * tw + col;
      let mut hm = [0_u32; 2];
      for (d, sum) in hm.iter_mut().enumerate() {
        for y in row - 1..=row + 1 {
          *sum += homo[d][y * tw + col - 1..=y * tw + col + 1].iter().map(|&h| h as u32).sum::<u32>();
        }
      }
      *pixel = if hm[0] != hm[1] {
        rgb[(hm[1] > hm[0]) as usize][i]
      } else {
        let (h, v) = (rgb[0][i], rgb[1][i]);
        [(h[0] + v[0]) / 2.0, (h[1] + v[1]) / 2.0, (h[2] + v[2]) / 2.0]
      };
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::pixarray::ImageView;

  const PATTERNS: [&str; 4] = ["RGGB", "BGGR", "GRBG", "GBRG"];

  fn mosaic(width: usize, height: usize) -> Vec<f32> {
    (0..width * height).map(|i| (i * 7919 % 1000) as f32 / 1000.0).collect()
  }

  fn demosaic(algorithm: &AhdDemosaic, samples: &[f32], width: usize, height: usize, pattern: &str) -> Color2D<f32, 3> {
    algorithm
      .demosaic(ImageView::new(samples, width, height, 1).into(), &CFA::new(pattern), &PlaneColor::new("RGB"))
      .expect("demosaic")
  }

  #[test]
  fn tiling_doesnt_change_the_result() {
    // Tiles with cores of 1, 7 and 22 pixels against one tile for the image
    let (width, height) = (61, 43);
    let samples = mosaic(width, height);
    for pattern in PATTERNS {
      let expected = demosaic(&AhdDemosaic::with_tile(100), &samples, width, height, pattern);
      for tile in [11, 17, 32] {
        let demosaiced = demosaic(&AhdDemosaic::with_tile(tile), &samples, width, height, pattern);
        assert!(demosaiced.data == expected.data, "{} with tiles of {} pixels", pattern, tile);
      }
    }
  }

  #[test]
  fn small_images_keep_their_samples() {
    // Images up to 10 pixels high or wide are border only
    for pattern in PATTERNS {
      let cfa = CFA::new(pattern);
      for (width, height) in (1..=12).flat_map(|width| (1..=12).map(move |height| (width, height))) {
        let samples = mosaic(width, height);
        let rgb = demosaic(&AhdDemosaic::new(), &samples, width, height, pattern);
        assert_eq!((rgb.width, rgb.height), (width, height), "{}", pattern);
        for (row, col) in (0..height).flat_map(|row| (0..width).map(move |col| (row, col))) {
          assert_eq!(
            rgb.at(row, col)[cfa.color_at(row, col)],
            samples[row * width + col],
            "{} {}x{} at {}:{}",
            pattern,
            width,
            height,
            col,
            row
          );
        }
      }
    }
  }

  #[test]
  fn flat_colors_stay_flat() {
    let (width, height) = (40, 30);
    let color = [0.2, 0.5, 0.3];
    for pattern in PATTERNS {
      let cfa = CFA::new(pattern);
      let samples: Vec<f32> = (0..width * height).map(|i| color[cfa.color_at(i / width, i % width)]).collect();
      let rgb = demosaic(&AhdDemosaic::new(), &samples, width, height, pattern);
      for (i, pixel) in rgb.pixels().iter().enumerate() {
        for c in 0..3 {
          assert!((pixel[c] - color[c]).abs() < 1e-6, "{} {:?} at {}:{}", pattern, pixel, i % width, i / width);
        }
      }
    }
  }

  #[test]
  fn edges_stay_neutral() {
    // Neutral data with a horizontal or vertical edge: the interpolation
    // along the edge is neutral, across it there are false colors
    let (width, height) = (40, 30);
    for pattern in PATTERNS {
      for horizontal in [true, false] {
        let value = |row: usize, col: usize| {
          let across = if horizontal { row } else { col };
          if across < 15 { 0.2 } else { 0.8 }
        };
        let samples: Vec<f32> = (0..width * height).map(|i| value(i / width, i % width)).collect();
        let rgb = demosaic(&AhdDemosaic::new(), &samples, width, height, pattern);
        let inner = inner_area(Dim2::new(width, height), HALO);
        for row in inner.p.y..inner.p.y + inner.d.h {
          for col in inner.p.x..inner.p.x + inner.d.w {
            let pixel = rgb.at(row, col);
            assert!(
              (pixel[0] - pixel[1]).abs() < 1e-6 && (pixel[2] - pixel[1]).abs() < 1e-6,
              "{} {:?} at {}:{}",
              pattern,
              pixel,
              col,
              row
            );
          }
        }
      }
    }
  }
}
