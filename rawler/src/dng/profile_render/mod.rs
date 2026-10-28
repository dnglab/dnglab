// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Rendering tables of DNG camera profiles
//!
//! ProfileHueSatMap, ProfileLookTable and ProfileToneCurve are applied in this
//! order to linear ProPhoto RGB (D50), after the camera values are mapped to it.

mod hue_sat_map;
mod tone_curve;

pub use hue_sat_map::HueSatMap;
pub use tone_curve::ToneCurve;

use crate::camera_profile::CameraProfile;
use crate::imgop::{
  chromatic_adaption::bradford_adaption_matrix,
  matrix::{multiply, pseudo_inverse},
  xyz::{Illuminant, SRGB_TO_XYZ_D65, XYZ_TO_PROFOTORGB_D50, XYZ_TO_SRGB_D65},
};

/// Rendering tables of a camera profile for one calibration slot
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileRender {
  pub hue_sat_map: Option<HueSatMap>,
  pub look_table: Option<HueSatMap>,
  pub tone_curve: Option<ToneCurve>,
}

impl ProfileRender {
  /// Tables of `profile` with the hue/saturation map of calibration slot `slot`,
  /// `None` if the profile has no valid table
  pub fn new(profile: &CameraProfile, slot: usize) -> Option<Self> {
    let hue_sat_map = profile
      .hue_sat_map_dims
      .zip(profile.hue_sat_map_data.get(slot).and_then(Option::as_deref))
      .and_then(|(dims, data)| HueSatMap::new(dims, data, profile.hue_sat_map_encoding));
    let look_table = profile
      .look_table_dims
      .zip(profile.look_table_data.as_deref())
      .and_then(|(dims, data)| HueSatMap::new(dims, data, profile.look_table_encoding));
    let tone_curve = profile.tone_curve.as_deref().and_then(ToneCurve::new);
    if hue_sat_map.is_none() && look_table.is_none() && tone_curve.is_none() {
      return None;
    }
    Some(Self {
      hue_sat_map,
      look_table,
      tone_curve,
    })
  }

  /// Apply the tables to linear ProPhoto RGB (D50)
  pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
    let mut rgb = rgb;
    if let Some(table) = &self.hue_sat_map {
      rgb = table.apply(rgb);
    }
    if let Some(table) = &self.look_table {
      rgb = table.apply(rgb);
    }
    if let Some(curve) = &self.tone_curve {
      rgb = curve.apply_rgb(rgb);
    }
    rgb
  }
}

/// Linear sRGB (D65) to linear ProPhoto RGB (D50)
pub fn srgb_to_prophoto() -> [[f32; 3]; 3] {
  multiply(
    &XYZ_TO_PROFOTORGB_D50,
    &multiply(&bradford_adaption_matrix(&Illuminant::D65, &Illuminant::D50), &SRGB_TO_XYZ_D65),
  )
}

/// Linear ProPhoto RGB (D50) to linear sRGB (D65)
pub fn prophoto_to_srgb() -> [[f32; 3]; 3] {
  multiply(
    &XYZ_TO_SRGB_D65,
    &multiply(
      &bradford_adaption_matrix(&Illuminant::D50, &Illuminant::D65),
      &pseudo_inverse(XYZ_TO_PROFOTORGB_D50),
    ),
  )
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::imgop::matrix::multiply_vector;

  #[test]
  fn profile_render_of_slot() {
    let mut profile = CameraProfile::default();
    assert!(ProfileRender::new(&profile, 0).is_none());
    // Hue shift of 120 degrees in slot 1, none in slot 0
    profile.hue_sat_map_dims = Some([2, 2, 1]);
    profile.hue_sat_map_data = [Some([0.0, 1.0, 1.0].repeat(4)), Some([120.0, 1.0, 1.0].repeat(4)), None];
    let render = ProfileRender::new(&profile, 1).expect("render");
    assert_eq!(render.apply([0.5, 0.0, 0.0]), [0.0, 0.5, 0.0]);
    assert_eq!(ProfileRender::new(&profile, 0).expect("render").apply([0.5, 0.0, 0.0]), [0.5, 0.0, 0.0]);
    // Slot 2 has no table
    assert!(ProfileRender::new(&profile, 2).is_none());
    profile.tone_curve = Some(vec![0.0, 0.0, 0.5, 0.25, 1.0, 1.0]);
    let render = ProfileRender::new(&profile, 2).expect("render");
    assert!(render.hue_sat_map.is_none() && render.tone_curve.is_some());
  }

  #[test]
  fn prophoto_conversion_keeps_white() {
    for (matrix, white) in [(srgb_to_prophoto(), [1.0; 3]), (prophoto_to_srgb(), [1.0; 3])] {
      for v in multiply_vector(&matrix, &white) {
        assert!((v - 1.0).abs() < 1e-3, "{:?}", matrix);
      }
    }
    let roundtrip = multiply(&prophoto_to_srgb(), &srgb_to_prophoto());
    for (i, row) in roundtrip.iter().enumerate() {
      for (j, v) in row.iter().enumerate() {
        assert!((v - if i == j { 1.0 } else { 0.0 }).abs() < 1e-4, "{:?}", roundtrip);
      }
    }
  }
}
