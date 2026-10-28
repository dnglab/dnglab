// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! HSV (hue, saturation, value) color model
//!
//! The hue is in the range 0.0 .. 6.0, one unit for each 60 degrees: 0.0 is
//! red, 2.0 is green and 4.0 is blue. Saturation and value are relative to the
//! RGB values, value is the largest channel.

/// RGB to hue (0.0 .. 6.0), saturation and value
pub fn rgb_to_hsv([r, g, b]: [f32; 3]) -> (f32, f32, f32) {
  let v = r.max(g).max(b);
  let gap = v - r.min(g).min(b);
  if gap > 0.0 {
    let h = if r == v {
      let h = (g - b) / gap;
      if h < 0.0 { h + 6.0 } else { h }
    } else if g == v {
      2.0 + (b - r) / gap
    } else {
      4.0 + (r - g) / gap
    };
    (h, gap / v, v)
  } else {
    (0.0, 0.0, v)
  }
}

/// Hue (any range, wraps at 6.0), saturation and value to RGB
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
  if s <= 0.0 {
    return [v; 3];
  }
  let h = h.rem_euclid(6.0);
  let sector = (h as usize).min(5);
  let f = h - sector as f32;
  let p = v * (1.0 - s);
  let q = v * (1.0 - s * f);
  let t = v * (1.0 - s * (1.0 - f));
  match sector {
    0 => [v, t, p],
    1 => [q, v, p],
    2 => [p, v, t],
    3 => [p, q, v],
    4 => [t, p, v],
    _ => [v, p, q],
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn assert_rgb_eq(a: [f32; 3], b: [f32; 3]) {
    for (x, y) in a.iter().zip(b) {
      assert!((x - y).abs() < 1e-5, "{:?} != {:?}", a, b);
    }
  }

  #[test]
  fn hsv_roundtrip() {
    for rgb in [
      [0.8, 0.2, 0.1],
      [0.1, 0.7, 0.3],
      [0.2, 0.3, 0.9],
      [0.5, 0.5, 0.5],
      [0.9, 0.1, 0.9],
      [0.0, 0.0, 0.0],
    ] {
      let (h, s, v) = rgb_to_hsv(rgb);
      assert_rgb_eq(hsv_to_rgb(h, s, v), rgb);
    }
  }

  #[test]
  fn primaries_and_hue_wrap() {
    assert_eq!(rgb_to_hsv([1.0, 0.0, 0.0]), (0.0, 1.0, 1.0));
    assert_eq!(rgb_to_hsv([0.0, 0.5, 0.0]), (2.0, 1.0, 0.5));
    assert_eq!(rgb_to_hsv([0.0, 0.0, 0.25]), (4.0, 1.0, 0.25));
    // Hues outside of 0.0 .. 6.0 wrap around
    assert_rgb_eq(hsv_to_rgb(-2.0, 1.0, 1.0), [0.0, 0.0, 1.0]);
    assert_rgb_eq(hsv_to_rgb(8.0, 1.0, 1.0), [0.0, 1.0, 0.0]);
  }
}
