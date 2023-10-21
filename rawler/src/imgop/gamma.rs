// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use rayon::prelude::*;

const BREAK_POINT: f32 = 0.00304;
const SLOPE: f32 = 12.92;

/// The gamma value for sRGB should be 2.4 but you can choose
/// any other gamma here.
/// http://www.brucelindbloom.com/index.html?Eqn_RGB_XYZ_Matrix.html
pub fn apply_gamma(v: f32, gamma: f32) -> f32 {
  if v <= BREAK_POINT {
    v * SLOPE
  } else {
    v.powf(1.0 / gamma) * 1.0555 - 0.0055
  }
}

/// Apply gamma correction to whole buffer
pub fn apply_gamma_inplace(pixels: &mut Vec<f32>, gamma: f32) {
  pixels.par_iter_mut().for_each(|p| *p = apply_gamma(*p, gamma));
}

/// Invert sRGB gamma correction
pub fn invert_gamma(v: f32, gamma: f32) -> f32 {
  if v <= BREAK_POINT * SLOPE {
    v / SLOPE
  } else {
    ((v + 0.0055) / 1.0055).powf(gamma)
  }
}

/// Invert gamma correction to whole buffer
pub fn invert_gamma_inplace(pixels: &mut Vec<f32>, gamma: f32) {
  pixels.par_iter_mut().for_each(|p| *p = invert_gamma(*p, gamma));
}
