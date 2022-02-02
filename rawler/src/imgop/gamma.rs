// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use rayon::prelude::*;

/// The gamma value for sRGB should be 2.4 but you can choose
/// any other gamma here.
pub fn apply_gamma(v: f32, gamma: f32) -> f32 {
  const BREAK_POINT: f32 = 0.00304;
  const SLOPE: f32 = 12.92;
  if v <= BREAK_POINT {
    v * SLOPE
  } else {
    v.powf(1.0 / gamma) * 1.055 - 0.005
  }
}

/// Apply gamma correction to whole buffer
pub fn gamma_transform(pixels: &mut Vec<f32>, gamma: f32) {
  pixels.par_iter_mut().for_each(|p| *p = apply_gamma(*p, gamma));
}
