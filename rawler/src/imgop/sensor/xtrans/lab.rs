use crate::imgop::math::fast_cbrt;

/// Convert linear RGB to CIELab.
///
/// Uses a fixed sRGB/D65 matrix for the RGB→XYZ step. This is used by
/// Markesteijn for homogeneity comparison — exact colorimetric accuracy
/// is not required, only consistent relative distances.
#[inline]
pub fn rgb_to_lab(r: f32, g: f32, b: f32) -> [f32; 3] {
  // sRGB linear → XYZ (D65)
  let x = 0.4124564 * r + 0.3575761 * g + 0.1804375 * b;
  let y = 0.2126729 * r + 0.7151522 * g + 0.0721750 * b;
  let z = 0.0193339 * r + 0.1191920 * g + 0.9503041 * b;

  // D65 white point
  let xn = 0.95047;
  let yn = 1.0;
  let zn = 1.08883;

  let fx = lab_f(x / xn);
  let fy = lab_f(y / yn);
  let fz = lab_f(z / zn);

  // In CIELab, the a* and b* channels share a dependency on luminance Y
  // through the definition:
  //   L* = 116 · f(Y/Yn) - 16     → L* depends on Y
  //   a* = 500 · [f(X/Xn) - f(Y/Yn)]  → a* depends on both X and Y
  //   b* = 200 · [f(Y/Yn) - f(Z/Zn)]  → b* depends on both Y and Z
  let l = 116.0 * fy - 16.0;
  let a = 500.0 * (fx - fy);
  let b_val = 200.0 * (fy - fz);

  [l, a, b_val]
}

#[inline(always)]
fn lab_f(t: f32) -> f32 {
  const DELTA_CB: f32 = (6.0 / 29.0) * (6.0 / 29.0) * (6.0 / 29.0);
  const LINEAR_SCALE: f32 = 1.0 / (3.0 * (6.0 / 29.0) * (6.0 / 29.0));
  const LINEAR_OFFSET: f32 = 4.0 / 29.0;

  if t > DELTA_CB { fast_cbrt(t) } else { t * LINEAR_SCALE + LINEAR_OFFSET }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn white_is_l100() {
    let [l, a, b] = rgb_to_lab(1.0, 1.0, 1.0);
    assert!((l - 100.0).abs() < 0.5, "L={l}");
    assert!(a.abs() < 1.0, "a={a}");
    assert!(b.abs() < 1.0, "b={b}");
  }

  #[test]
  fn black_is_l0() {
    let [l, a, b] = rgb_to_lab(0.0, 0.0, 0.0);
    assert!(l.abs() < 0.5, "L={l}");
    assert!(a.abs() < 0.5, "a={a}");
    assert!(b.abs() < 0.5, "b={b}");
  }
}
