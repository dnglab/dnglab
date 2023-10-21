// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::gamma::apply_gamma_inplace;

/// Apply sRGB specific gamma correction
pub fn srgb_gamma_transform(pixels: &mut Vec<f32>) {
  apply_gamma_inplace(pixels, 2.4)
}
