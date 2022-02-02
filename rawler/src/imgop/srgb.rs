// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::gamma::gamma_transform;

/// Apply sRGB specific gamma correction
pub fn srgb_gamma_transform(pixels: &mut Vec<f32>) {
  gamma_transform(pixels, 2.4)
}
