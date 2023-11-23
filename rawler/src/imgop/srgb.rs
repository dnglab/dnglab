// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use rayon::prelude::*;

// Reference for sRGB:
//
// https://onlinelibrary.wiley.com/doi/pdf/10.1002/9781119021780.app8
//
// http://www.brucelindbloom.com/index.html?Eqn_RGB_XYZ_Matrix.html
// http://www.brucelindbloom.com/index.html?Eqn_XYZ_to_RGB.html
// /// http://www.brucelindbloom.com/index.html?Eqn_RGB_to_XYZ.html
//

/// sRGB gamma
const SRGB_GAMMA: f32 = 2.4;

/// sRGB crossover point between linear and exponential part
const SRGB_CROSSOVER_POINT: f32 = 0.00304;

/// sRGB gain for linear part, specified for gamma 2.4
const LINEAR_GAIN: f32 = 12.92;

/// sRGB gain for exponential part
const SRGB_GAIN: f32 = 1.055;

/// sRGB offset for exponential part
const SRGB_OFFSET: f32 = SRGB_GAIN - 1.0; // 0.055

/// Apply sRGB gamma
pub fn srgb_apply_gamma(v: f32) -> f32 {
  if v <= SRGB_CROSSOVER_POINT {
    v * LINEAR_GAIN
  } else {
    v.powf(1.0 / SRGB_GAMMA) * SRGB_GAIN - SRGB_OFFSET
  }
}

/// Invert sRGB gamma correction
pub fn srgb_invert_gamma(v: f32) -> f32 {
  if v <= SRGB_CROSSOVER_POINT * LINEAR_GAIN {
    v / LINEAR_GAIN
  } else {
    ((v + SRGB_OFFSET) / SRGB_GAIN).powf(SRGB_GAMMA)
  }
}

/// Apply gamma correction to whole buffer
pub fn srgb_apply_gamma_inplace(pixels: &mut [f32]) {
  pixels.par_iter_mut().for_each(|p| *p = srgb_apply_gamma(*p));
}

/// Invert gamma correction to whole buffer
pub fn srgb_invert_gamma_inplace(pixels: &mut [f32]) {
  pixels.par_iter_mut().for_each(|p| *p = srgb_invert_gamma(*p));
}
