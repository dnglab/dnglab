// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

pub mod bilinear;
pub mod ppg;
pub mod superpixel;

use multiversion::multiversion;
use rayon::prelude::*;

use crate::{
  cfa::CFA,
  pixarray::{ImageRegion, RgbF32},
};

/// Extend a single pixel component from bayer pattern to RGB
///
/// The other channels (missing colors) are set to 0.0.
#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn expand_bayer_rgb<'a>(raw: ImageRegion<'a, f32>, cfa: &CFA) -> RgbF32 {
  let roi = raw.area();
  // The ROI changes the pattern if not perfectly aligned on the origin pattern
  let cfa_roi = cfa.shift(roi.x(), roi.y());
  let mut out = RgbF32::new(roi.width(), roi.height());
  out
    .pixels_mut()
    .par_chunks_exact_mut(roi.width())
    .zip(raw.par_rows())
    .enumerate()
    .for_each(|(row, (buf, line))| {
      for (col, (p_out, p_in)) in buf.iter_mut().zip(line).enumerate() {
        p_out[cfa_roi.color_at(row, col)] = *p_in;
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
