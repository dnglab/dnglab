// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use rayon::prelude::*;

// https://kosinix.github.io/raster/docs/src/raster/filter.rs.html#339-359
pub fn gamma_transform(pixels: &mut Vec<f32>, gamma: f32) {
  pixels.par_iter_mut().for_each(|p| *p = p.powf(1.0 / gamma));
}
