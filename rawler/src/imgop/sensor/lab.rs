// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

use crate::imgop::{
  cielab::XYZ_to_lab,
  matrix::multiply_row1,
  xyz::{CIE_1931_TRISTIMULUS_D65, SRGB_TO_XYZ_D65},
};

/// Convert linear RGB to CIELab.
///
/// Uses a fixed sRGB/D65 matrix for the RGB→XYZ step. Demosaic algorithms
/// (AHD, Markesteijn) use it to compare the homogeneity of interpolations —
/// exact colorimetric accuracy is not required, only consistent relative
/// distances.
#[inline(always)]
pub fn rgb_to_lab(rgb: &[f32; 3]) -> [f32; 3] {
  let xyz = multiply_row1(&SRGB_TO_XYZ_D65, rgb);
  XYZ_to_lab(&xyz, &CIE_1931_TRISTIMULUS_D65)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn white_is_l100() {
    let [l, a, b] = rgb_to_lab(&[1.0, 1.0, 1.0]);
    assert!((l - 100.0).abs() < 0.5, "L={l}");
    assert!(a.abs() < 1.0, "a={a}");
    assert!(b.abs() < 1.0, "b={b}");
  }

  #[test]
  fn black_is_l0() {
    let [l, a, b] = rgb_to_lab(&[0.0, 0.0, 0.0]);
    assert!(l.abs() < 0.5, "L={l}");
    assert!(a.abs() < 0.5, "a={a}");
    assert!(b.abs() < 0.5, "b={b}");
  }
}
