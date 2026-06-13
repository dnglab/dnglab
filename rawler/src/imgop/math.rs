// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

use num::Zero;

/// Fast cube root approximation
///
/// This implementation is based on [1] and [2]
/// For the calculation of the CUBIC_ROOT_MAGIC constant,
/// see reference [2].
///
/// [1] https://www.romainguy.dev/posts/2024/going-old-school/
/// [2] https://cs.android.com/androidx/platform/frameworks/support/+/androidx-main:compose/ui/ui-util/src/commonMain/kotlin/androidx/compose/ui/util/MathHelpers.kt;l=131?q=fastcbrt
///
#[inline(always)]
pub(crate) fn fast_cbrt(x: f32) -> f32 {
  const CUBIC_ROOT_MAGIC: u32 = 0x2a510554;
  if x.is_zero() {
    return 0.0;
  }
  let x_abs = x.abs();
  let mut approx = f32::from_bits(x_abs.to_bits() / 3 + CUBIC_ROOT_MAGIC);
  // Two Newton-Raphson steps for refinement
  approx -= (approx - x_abs / (approx * approx)) * (1.0 / 3.0);
  approx -= (approx - x_abs / (approx * approx)) * (1.0 / 3.0);
  if x.is_sign_negative() { -approx } else { approx }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn fast_cbrt_approx() {
    for value in -(u16::MAX as i32)..(u16::MAX as i32) {
      let a = fast_cbrt(value as f32);
      let b = (value as f32).cbrt();
      let abs_difference = (a - b).abs();
      assert!(abs_difference < 1e-4, "{} vs. {}, delta: {}", a, b, abs_difference);
    }

    let mut value = -1.0;
    while value <= 1.0 {
      let a = fast_cbrt(value as f32);
      let b = (value as f32).cbrt();
      let abs_difference = (a - b).abs();
      assert!(abs_difference <= 5.967e-7, "{} vs. {}, delta: {}", a, b, abs_difference);
      value += 0.00001;
    }
  }
}
