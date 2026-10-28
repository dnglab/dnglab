// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Hue/saturation/value mapping tables (ProfileHueSatMapData, ProfileLookTableData)

use crate::imgop::{
  hsv::{hsv_to_rgb, rgb_to_hsv},
  srgb::{srgb_apply_gamma, srgb_invert_gamma},
};

/// Table encoding of the value axis: 0 = linear, 1 = sRGB
const ENCODING_SRGB: u32 = 1;

/// Hue/saturation/value mapping table
///
/// Each entry holds a hue shift in degrees, a saturation scale and a value
/// scale. The table is indexed by the HSV values of linear ProPhoto RGB and
/// interpolated linearly, the hue axis wraps around.
#[derive(Debug, Clone, PartialEq)]
pub struct HueSatMap {
  hue_divisions: usize,
  sat_divisions: usize,
  val_divisions: usize,
  /// The value axis is sRGB encoded
  srgb_encoding: bool,
  /// Entries with the value divisions in the outer loop, then hue and saturation
  deltas: Vec<[f32; 3]>,
}

impl HueSatMap {
  /// Build a table from the dimensions (hue, saturation and value divisions),
  /// the table data and the encoding tag
  pub fn new(dims: [u32; 3], data: &[f32], encoding: Option<u32>) -> Option<Self> {
    let [hue_divisions, sat_divisions, val_divisions] = dims.map(|value| value as usize);
    if hue_divisions < 1 || sat_divisions < 2 || val_divisions < 1 || data.len() != 3 * hue_divisions * sat_divisions * val_divisions {
      log::warn!(
        "Invalid hue/saturation/value table with dimensions {:?} and {} values, ignoring it",
        dims,
        data.len()
      );
      return None;
    }
    let srgb_encoding = match encoding.unwrap_or(0) {
      0 => false,
      ENCODING_SRGB => true,
      other => {
        log::warn!("Unknown hue/saturation/value table encoding {}, using linear encoding", other);
        false
      }
    };
    Some(Self {
      hue_divisions,
      sat_divisions,
      val_divisions,
      srgb_encoding,
      deltas: data.as_chunks::<3>().0.to_vec(),
    })
  }

  /// Apply the table to linear ProPhoto RGB, the value is limited to 1.0
  pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
    let (h, s, v) = rgb_to_hsv(rgb.map(|c| c.max(0.0)));
    let v_encoded = if self.srgb_encoding { srgb_apply_gamma(v.min(1.0)) } else { v };
    let [hue_shift, sat_scale, val_scale] = self.lookup(h, s, v_encoded);
    let h = h + hue_shift * (6.0 / 360.0);
    let s = (s * sat_scale).min(1.0);
    let v_encoded = (v_encoded * val_scale).clamp(0.0, 1.0);
    let v = if self.srgb_encoding { srgb_invert_gamma(v_encoded) } else { v_encoded };
    hsv_to_rgb(h, s, v)
  }

  /// Interpolated table entry for hue (0.0 .. 6.0), saturation and encoded value
  fn lookup(&self, h: f32, s: f32, v: f32) -> [f32; 3] {
    let h_scaled = if self.hue_divisions < 2 { 0.0 } else { h * self.hue_divisions as f32 / 6.0 };
    let s_scaled = s * (self.sat_divisions - 1) as f32;
    // Float to integer casts saturate, NaN becomes 0
    let h0 = (h_scaled as usize).min(self.hue_divisions - 1);
    let h1 = if h0 + 1 < self.hue_divisions { h0 + 1 } else { 0 };
    let s0 = (s_scaled as usize).min(self.sat_divisions - 2);
    let h_fract = h_scaled - h0 as f32;
    let s_fract = s_scaled - s0 as f32;
    let (v0, v1, v_fract) = if self.val_divisions < 2 {
      (0, 0, 0.0)
    } else {
      let v_scaled = v.clamp(0.0, 1.0) * (self.val_divisions - 1) as f32;
      let v0 = (v_scaled as usize).min(self.val_divisions - 2);
      (v0, v0 + 1, v_scaled - v0 as f32)
    };

    let mut result = [0.0; 3];
    for (v_index, v_weight) in [(v0, 1.0 - v_fract), (v1, v_fract)] {
      for (h_index, h_weight) in [(h0, 1.0 - h_fract), (h1, h_fract)] {
        for (s_index, s_weight) in [(s0, 1.0 - s_fract), (s0 + 1, s_fract)] {
          let weight = v_weight * h_weight * s_weight;
          let entry = self.deltas[(v_index * self.hue_divisions + h_index) * self.sat_divisions + s_index];
          for (value, delta) in result.iter_mut().zip(entry) {
            *value += weight * delta;
          }
        }
      }
    }
    result
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

  /// Table of the given dimensions with the same entry everywhere
  fn uniform_table(dims: [u32; 3], entry: [f32; 3], encoding: Option<u32>) -> HueSatMap {
    let count = dims.iter().product::<u32>() as usize;
    HueSatMap::new(dims, &entry.repeat(count), encoding).expect("table")
  }

  #[test]
  fn identity_table() {
    for encoding in [None, Some(ENCODING_SRGB)] {
      for dims in [[90, 30, 1], [6, 4, 8]] {
        let table = uniform_table(dims, [0.0, 1.0, 1.0], encoding);
        for rgb in [[0.8, 0.2, 0.1], [0.1, 0.7, 0.3], [0.05, 0.03, 0.9], [0.4, 0.4, 0.4]] {
          assert_rgb_eq(table.apply(rgb), rgb);
        }
      }
    }
  }

  #[test]
  fn hue_shift_and_saturation() {
    // 120 degrees turn red into green
    assert_rgb_eq(uniform_table([4, 2, 1], [120.0, 1.0, 1.0], None).apply([0.8, 0.0, 0.0]), [0.0, 0.8, 0.0]);
    // Negative shifts wrap around
    assert_rgb_eq(uniform_table([4, 2, 1], [-120.0, 1.0, 1.0], None).apply([0.8, 0.0, 0.0]), [0.0, 0.0, 0.8]);
    assert_rgb_eq(uniform_table([4, 2, 1], [0.0, 0.0, 1.0], None).apply([0.8, 0.2, 0.4]), [0.8; 3]);
    // The value is limited to 1.0
    assert_rgb_eq(uniform_table([4, 2, 1], [0.0, 1.0, 2.0], None).apply([0.8, 0.4, 0.2]), [1.0, 0.5, 0.25]);
  }

  #[test]
  fn hue_axis_wraps_around() {
    // Two hue divisions: red (0 degrees) shifts by 0, cyan (180 degrees) by 60 degrees
    let mut data = Vec::new();
    for shift in [0.0, 60.0] {
      data.extend([shift, 1.0, 1.0, shift, 1.0, 1.0]);
    }
    let table = HueSatMap::new([2, 2, 1], &data, None).expect("table");
    // Hue 270 degrees is between cyan and red (wrapped), the shift is 30 degrees
    let (h, _, _) = rgb_to_hsv(table.apply(hsv_to_rgb(4.5, 1.0, 0.5)));
    assert!((h - 5.0).abs() < 1e-5, "{}", h);
  }

  #[test]
  fn value_axis_is_interpolated() {
    // Value scale 1.0 at value 0.0 and 2.0 at value 1.0
    let mut data = Vec::new();
    for scale in [1.0, 2.0] {
      data.extend([0.0, 1.0, scale].repeat(2));
    }
    let table = HueSatMap::new([1, 2, 2], &data, None).expect("table");
    assert_rgb_eq(table.apply([0.4, 0.2, 0.2]), [0.56, 0.28, 0.28]);
  }

  #[test]
  fn invalid_tables() {
    assert!(HueSatMap::new([2, 1, 1], &[0.0; 6], None).is_none());
    assert!(HueSatMap::new([2, 2, 1], &[0.0; 11], None).is_none());
    assert!(HueSatMap::new([0, 2, 1], &[], None).is_none());
  }
}
