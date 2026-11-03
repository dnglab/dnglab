// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::borrow::Cow;

use multiversion::multiversion;

use crate::{
  CFA,
  cfa::PlaneColor,
  imgop::{Rect, sensor::Demosaic},
  pixarray::{Color2D, ImageRegion},
};

#[derive(Default)]
pub struct Bilinear4Channel {}

impl Bilinear4Channel {
  pub fn new() -> Self {
    Self {}
  }
}

impl Demosaic<f32, 4> for Bilinear4Channel {
  /// Debayer image by using bilinear method.
  fn demosaic(&self, pixels: ImageRegion<'_, f32>, cfa: &CFA, colors: &PlaneColor) -> Color2D<f32, 4> {
    assert_eq!(pixels.channels(), 1, "Mosaic data must have one channel");
    // Contiguous samples are used in place, others are copied
    let samples = match pixels.image_view() {
      Some(view) => Cow::Borrowed(view.as_slice()),
      None => Cow::Owned(pixels.to_vec()),
    };
    Self::demosaic_4ch(&samples, cfa, colors, pixels.area())
  }
}

impl Bilinear4Channel {
  #[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
  /// `samples` are the pixels of `roi` of the image, row by row
  fn demosaic_4ch(samples: &[f32], cfa: &CFA, colors: &PlaneColor, roi: Rect) -> Color2D<f32, 4> {
    if colors.plane_count() != 4 {
      panic!("Demosaic for 4 channels needs 4 color planes, but {} given", colors.plane_count());
    }
    log::debug!("Bilinear debayer ROI: {:?}", roi);

    let cfa = cfa.shift(roi.p.x, roi.p.y);
    let plane_map = colors.plane_lookup_table();
    let ch = |row: usize, col: usize| -> usize { plane_map[cfa.color_at(row, col)] };

    let dim = roi.d;
    let at = |row: usize, col: usize| -> f32 { samples[row * dim.w + col] };
    let mut out = Color2D::new(dim.w, dim.h);

    // Process edges
    out.at_mut(0, 0)[ch(0, 0)] = at(0, 0);
    out.at_mut(0, 0)[ch(0, 1)] = at(0, 1);
    out.at_mut(0, 0)[ch(1, 0)] = at(1, 0);
    out.at_mut(0, 0)[ch(1, 1)] = at(1, 1);

    out.at_mut(0, dim.w - 1)[ch(0, dim.w - 1)] = at(0, dim.w - 1);
    out.at_mut(0, dim.w - 1)[ch(0, dim.w - 2)] = at(0, dim.w - 2);
    out.at_mut(0, dim.w - 1)[ch(1, dim.w - 1)] = at(1, dim.w - 1);
    out.at_mut(0, dim.w - 1)[ch(1, dim.w - 2)] = at(1, dim.w - 2);

    out.at_mut(dim.h - 1, 0)[ch(dim.h - 1, 0)] = at(dim.h - 1, 0);
    out.at_mut(dim.h - 1, 0)[ch(dim.h - 1, 1)] = at(dim.h - 1, 1);
    out.at_mut(dim.h - 1, 0)[ch(dim.h - 2, 0)] = at(dim.h - 2, 0);
    out.at_mut(dim.h - 1, 0)[ch(dim.h - 2, 1)] = at(dim.h - 2, 1);

    out.at_mut(dim.h - 1, dim.w - 1)[ch(dim.h - 1, dim.w - 1)] = at(dim.h - 1, dim.w - 1);
    out.at_mut(dim.h - 1, dim.w - 1)[ch(dim.h - 1, dim.w - 2)] = at(dim.h - 1, dim.w - 2);
    out.at_mut(dim.h - 1, dim.w - 1)[ch(dim.h - 2, dim.w - 1)] = at(dim.h - 2, dim.w - 1);
    out.at_mut(dim.h - 1, dim.w - 1)[ch(dim.h - 2, dim.w - 2)] = at(dim.h - 2, dim.w - 2);

    // Process borders
    for i in 1..out.width - 1 {
      // Top line
      out.at_mut(0, i)[ch(0, i)] = at(0, i);
      out.at_mut(0, i)[ch(0, i + 1)] = (at(0, i - 1) + at(0, i + 1)) / 2.0;
      out.at_mut(0, i)[ch(1, i)] = at(1, i);
      out.at_mut(0, i)[ch(1, i + 1)] = (at(1, i - 1) + at(1, i + 1)) / 2.0;
      // Bottom line
      out.at_mut(dim.h - 1, i)[ch(dim.h - 1, i)] = at(dim.h - 1, i);
      out.at_mut(dim.h - 1, i)[ch(dim.h - 1, i + 1)] = (at(dim.h - 1, i - 1) + at(dim.h - 1, i + 1)) / 2.0;
      out.at_mut(dim.h - 1, i)[ch(dim.h - 2, i)] = at(dim.h - 2, i);
      out.at_mut(dim.h - 1, i)[ch(dim.h - 2, i + 1)] = (at(dim.h - 2, i - 1) + at(dim.h - 2, i + 1)) / 2.0;
    }

    for i in 1..out.height - 1 {
      // Left
      out.at_mut(i, 0)[ch(i, 0)] = at(i, 0);
      out.at_mut(i, 0)[ch(i + 1, 0)] = (at(i - 1, 0) + at(i + 1, 0)) / 2.0;
      out.at_mut(i, 0)[ch(i, 1)] = at(i, 1);
      out.at_mut(i, 0)[ch(i + 1, 1)] = (at(i - 1, 1) + at(i + 1, 1)) / 2.0;
      // Right
      out.at_mut(i, dim.w - 1)[ch(i, dim.w - 1)] = at(i, dim.w - 1);
      out.at_mut(i, dim.w - 1)[ch(i + 1, dim.w - 1)] = (at(i - 1, dim.w - 1) + at(i + 1, dim.w - 1)) / 2.0;
      out.at_mut(i, dim.w - 1)[ch(i, dim.w - 2)] = at(i, dim.w - 2);
      out.at_mut(i, dim.w - 1)[ch(i + 1, dim.w - 2)] = (at(i - 1, dim.w - 2) + at(i + 1, dim.w - 2)) / 2.0;
    }

    /*
    A B A B
    C D C D
    A B A B
    C D C D
     */
    out.for_each_row(|row, pix| {
      if row == 0 || row == dim.h - 1 {
        return; // Skip border rows
      }
      for col in 1..dim.w - 1 {
        pix[col][ch(row, col)] = at(row, col);
        pix[col][ch(row + 1, col)] = (at(row - 1, col) + at(row + 1, col)) / 2.0;
        pix[col][ch(row, col + 1)] = (at(row, col - 1) + at(row, col + 1)) / 2.0;
        pix[col][ch(row + 1, col + 1)] = (at(row - 1, col - 1) + at(row - 1, col + 1) + at(row + 1, col - 1) + at(row + 1, col + 1)) / 4.0;
      }
    });

    out
  }
}
