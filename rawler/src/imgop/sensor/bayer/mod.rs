// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

pub mod bilinear;
pub mod ppg;
pub mod superpixel;

use multiversion::multiversion;
use rayon::prelude::*;

use crate::{
  cfa::{CFA, PlaneColor},
  imgop::{Dim2, Rect},
  pixarray::{Color2D, Pix2D, RgbF32, SubPixel},
};

pub trait Demosaic<T: SubPixel, const N: usize> {
  fn demosaic(&self, pixels: &Pix2D<T>, cfa: &CFA, colors: &PlaneColor, roi: Rect) -> Color2D<T, N>;
}

/// Extend a single pixel component from bayer pattern to RGB
///
/// The other channels (missing colors) are set to 0.0.
#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
pub fn expand_bayer_rgb(raw: &[f32], dim: Dim2, cfa: &CFA, roi: Rect) -> RgbF32 {
  // The ROI changes the pattern if not perfectly aligned on the origin pattern
  let cfa_roi = cfa.shift(roi.x(), roi.y());
  let mut out = RgbF32::new(roi.width(), roi.height());
  out.pixels_mut().par_chunks_exact_mut(roi.width()).enumerate().for_each(|(row_out, buf)| {
    assert_eq!(roi.width() % cfa.width, 0); // Area must be bound to CFA bounds
    let row_in = roi.p.y + row_out;
    let start_in = row_in * dim.w + roi.p.x;
    let line = &raw[start_in..start_in + roi.width()];
    for (col, (p_out, p_in)) in buf.iter_mut().zip(line.iter()).enumerate() {
      p_out[cfa_roi.color_at(row_out, col)] = *p_in;
    }
  });
  out
}

/// Bayer matrix pattern
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RgbBayerPattern {
  RGGB,
  BGGR,
  GBRG,
  GRBG,
  //ERBG,
  //RGEB,
}
