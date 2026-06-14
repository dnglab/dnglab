// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

use crate::imgop::math::fast_cbrt;

/// CIE standard threshold for the XYZ to CIELAB conversion function f(t).
/// Below this value, the linear approximation is used instead of the cube root.
const EPSILON: f32 = 216.0 / 24389.0;

/// CIE standard scaling factor for the linear portion of the XYZ to CIELAB
/// conversion function f(t).
const KAPPA: f32 = 24389.0 / 27.0;

/// Convert from CIE XYZ colorspace to CIELAB (L\*, a\*, b\*).
///
/// Applies the standard CIE forward conversion using the piecewise function
/// f(t), which uses a cube root for values above [`EPSILON`] and a linear
/// approximation below. The input XYZ values are normalized against
/// `white_ref` (the reference white tristimulus values) before conversion.
///
/// Returns `[L*, a*, b*]` where L\* is lightness (0..100), a\* is the
/// green-red axis, and b\* is the blue-yellow axis.
#[allow(non_snake_case)]
#[inline]
pub fn XYZ_to_lab(xyz: &[f32; 3], white_ref: &[f32; 3]) -> [f32; 3] {
  // CIELab function f()
  let f = |t: f32| {
    if t > EPSILON { fast_cbrt(t) } else { (KAPPA * t + 16.0) / 116.0 }
  };

  let fx = f(xyz[0] / white_ref[0]);
  let fy = f(xyz[1] / white_ref[1]);
  let fz = f(xyz[2] / white_ref[2]);

  // In CIELab, the a* and b* channels share a dependency on luminance Y
  // through the definition:
  //   L* = 116 · f(Y) - 16      → L* depends on Y
  //   a* = 500 · [f(X) - f(Y)]  → a* depends on both X and Y
  //   b* = 200 · [f(Y) - f(Z)]  → b* depends on both Y and Z
  let l = 116.0 * fy - 16.0;
  let a = 500.0 * (fx - fy);
  let b = 200.0 * (fy - fz);
  [l, a, b]
}

/// Convert from CIE XYZ colorspace to CIELAB in-place.
///
/// Overwrites the input `xyz` buffer with the resulting `[L*, a*, b*]` values.
/// See [`XYZ_to_lab`] for details on the conversion.
#[allow(non_snake_case)]
#[inline]
pub fn XYZ_to_lab_inplace(xyz: &mut [f32; 3], white_ref: &[f32; 3]) {
  *xyz = XYZ_to_lab(xyz, white_ref);
}

/// Convert from CIELAB (L\*, a\*, b\*) colorspace to CIE XYZ.
///
/// Applies the standard CIE inverse conversion. The piecewise function f(t)
/// uses a cube for values whose cube exceeds [`EPSILON`], and a linear
/// approximation otherwise. The resulting XYZ values are scaled by `white_ref`
/// (the reference white tristimulus values).
///
/// Expects `lab` as `[L*, a*, b*]` and returns `[X, Y, Z]`.
#[allow(non_snake_case)]
#[inline]
pub fn lab_to_XYZ(lab: &[f32; 3], white_ref: &[f32; 3]) -> [f32; 3] {
  // CIELab function f()
  let f = |t: f32| {
    if t.powi(3) > EPSILON { t.powi(3) } else { (116.0 * t - 16.0) / KAPPA }
  };

  let [l, a, b] = lab;

  let yr = (l + 16.0) / 116.0;
  let xr = (a / 500.0) + yr;
  let zr = yr - (b / 200.0);

  let fx = f(xr);
  let fy = if *l > KAPPA * EPSILON { yr.powi(3) } else { l / KAPPA };
  let fz = f(zr);

  [fx * white_ref[0], fy * white_ref[1], fz * white_ref[2]]
}

/// Convert from CIELAB (L\*, a\*, b\*) colorspace to CIE XYZ in-place.
///
/// Overwrites the input buffer with the resulting `[X, Y, Z]` values.
/// See [`lab_to_XYZ`] for details on the conversion.
#[allow(non_snake_case)]
#[inline]
pub fn lab_to_XYZ_inplace(xyz: &mut [f32; 3], white_ref: &[f32; 3]) {
  *xyz = lab_to_XYZ(xyz, white_ref);
}

#[cfg(test)]
mod tests {
  use crate::imgop::{
    matrix::multiply_row1,
    srgb::srgb_invert_gamma,
    xyz::{CIE_1931_TRISTIMULUS_D65, SRGB_TO_XYZ_D65},
  };

  use super::*;

  #[test]
  fn inverse_match() {
    let rgb = [0.18, 0.72, 0.36];
    let xyz = multiply_row1(&SRGB_TO_XYZ_D65, &rgb);
    let lab = XYZ_to_lab(&xyz, &CIE_1931_TRISTIMULUS_D65);
    let xyz_reverse = lab_to_XYZ(&lab, &CIE_1931_TRISTIMULUS_D65);
    for i in 0..3 {
      let delta_e = (xyz_reverse[i] - xyz[i]).abs();
      assert!(delta_e < 5e-7, "Delta is {}", delta_e);
    }
  }

  #[test]
  fn white_is_l100() {
    let rgb = [1.0, 1.0, 1.0]; // sRGB white
    let xyz = multiply_row1(&SRGB_TO_XYZ_D65, &rgb);
    let [l, a, b] = XYZ_to_lab(&xyz, &CIE_1931_TRISTIMULUS_D65);
    assert!((l - 100.0).abs() < 5e-5, "L={l}");
    assert!(a.abs() < 1e-7, "a={a}");
    assert!(b.abs() < 1e-7, "b={b}");
  }

  #[test]
  fn black_is_l0() {
    let rgb = [0.0, 0.0, 0.0]; // sRGB black
    let xyz = multiply_row1(&SRGB_TO_XYZ_D65, &rgb);
    let [l, a, b] = XYZ_to_lab(&xyz, &CIE_1931_TRISTIMULUS_D65);
    assert!(l.abs() < 1e-7, "L={l}");
    assert!(a.abs() < 1e-7, "a={a}");
    assert!(b.abs() < 1e-7, "b={b}");
  }

  #[test]
  fn grey_test() {
    let grey = srgb_invert_gamma(0.5); // linear RGB grey
    let rgb = [grey, grey, grey];
    let xyz = multiply_row1(&SRGB_TO_XYZ_D65, &rgb);
    let [l, a, b] = XYZ_to_lab(&xyz, &CIE_1931_TRISTIMULUS_D65);
    assert!((l - 53.39).abs() < 2e-3, "L={l}");
    assert!(a.abs() < 1e-4, "a={a}");
    assert!(b.abs() < 1e-4, "b={b}");
  }
}
