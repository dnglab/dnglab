// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Rendering tables of DNG camera profiles
//!
//! The tables are applied in this order to linear ProPhoto RGB (D50), after
//! the camera values are mapped to it: ProfileHueSatMap, the gain table map
//! (ProfileGainTableMap2 or ProfileGainTableMap), ProfileLookTable and
//! ProfileToneCurve.

mod gain_table_map;
mod hue_sat_map;
mod tone_curve;

pub use gain_table_map::GainTable;
pub use hue_sat_map::HueSatMap;
pub use tone_curve::ToneCurve;

use rayon::prelude::*;

use crate::camera_profile::{CameraProfile, GainTableMap};
use crate::imgop::{
  Rect,
  chromatic_adaption::bradford_adaption_matrix,
  matrix::{multiply, pseudo_inverse},
  xyz::{Illuminant, SRGB_TO_XYZ_D65, XYZ_TO_PROFOTORGB_D50, XYZ_TO_SRGB_D65},
};
use crate::pixarray::RgbF32;

/// Rendering tables of a camera profile for one calibration slot
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileRender {
  pub hue_sat_map: Option<HueSatMap>,
  pub gain_table: Option<GainTable>,
  pub look_table: Option<HueSatMap>,
  pub tone_curve: Option<ToneCurve>,
}

impl ProfileRender {
  /// Tables of `profile` with the hue/saturation map of calibration slot `slot`
  /// and the gain table map that applies to the profile, `None` without any
  /// valid table
  pub fn new(profile: &CameraProfile, slot: usize, gain_table_map: Option<&GainTableMap>) -> Option<Self> {
    let hue_sat_map = profile
      .hue_sat_map_dims
      .zip(profile.hue_sat_map_data.get(slot).and_then(Option::as_deref))
      .and_then(|(dims, data)| HueSatMap::new(dims, data, profile.hue_sat_map_encoding));
    let look_table = profile
      .look_table_dims
      .zip(profile.look_table_data.as_deref())
      .and_then(|(dims, data)| HueSatMap::new(dims, data, profile.look_table_encoding));
    let tone_curve = profile.tone_curve.as_deref().and_then(ToneCurve::new);
    let gain_table = gain_table_map.and_then(GainTable::new);
    if hue_sat_map.is_none() && gain_table.is_none() && look_table.is_none() && tone_curve.is_none() {
      return None;
    }
    Some(Self {
      hue_sat_map,
      gain_table,
      look_table,
      tone_curve,
    })
  }

  /// Apply the tables to an image of linear ProPhoto RGB (D50)
  ///
  /// The image is already scaled by `exposure_gain` (BaselineExposure).
  /// `area` is the image area the gain table map refers to (the stage 3 image),
  /// relative to `image`.
  pub fn apply(&self, image: &mut RgbF32, exposure_gain: f32, area: Rect) {
    let (left, top) = (area.p.x as f32, area.p.y as f32);
    let (width, height) = (area.d.w.max(1) as f32, area.d.h.max(1) as f32);
    image.data.par_chunks_mut(image.width.max(1)).enumerate().for_each(|(row, pixels)| {
      // Positions of the pixel centers, relative to the image area
      let v = (row as f32 + 0.5 - top) / height;
      for (col, pixel) in pixels.iter_mut().enumerate() {
        let u = (col as f32 + 0.5 - left) / width;
        *pixel = self.apply_pixel(*pixel, exposure_gain, u, v);
      }
    });
  }

  /// Apply the tables to one pixel at the relative position `u` (horizontal)
  /// and `v` (vertical)
  ///
  /// Like in a DNG reader, the values are limited to the white point, the
  /// hue/saturation map is applied before the exposure and the values are
  /// clipped to 1.0 after the gain table map.
  pub fn apply_pixel(&self, rgb: [f32; 3], exposure_gain: f32, u: f32, v: f32) -> [f32; 3] {
    let mut rgb = rgb.map(|c| c.clamp(0.0, exposure_gain));
    if let Some(table) = &self.hue_sat_map {
      rgb = table.apply(rgb.map(|c| c / exposure_gain)).map(|c| c * exposure_gain);
    }
    if let Some(table) = &self.gain_table {
      let gain = table.gain(rgb, u, v);
      rgb = rgb.map(|c| c * gain);
    }
    rgb = rgb.map(|c| c.clamp(0.0, 1.0));
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
  use crate::camera_profile::GainData;
  use crate::imgop::{Dim2, Point, matrix::multiply_vector};

  #[test]
  fn profile_render_of_slot() {
    let mut profile = CameraProfile::default();
    assert!(ProfileRender::new(&profile, 0, None).is_none());
    // Hue shift of 120 degrees in slot 1, none in slot 0
    profile.hue_sat_map_dims = Some([2, 2, 1]);
    profile.hue_sat_map_data = [Some([0.0, 1.0, 1.0].repeat(4)), Some([120.0, 1.0, 1.0].repeat(4)), None];
    let render = ProfileRender::new(&profile, 1, None).expect("render");
    assert_eq!(render.apply_pixel([0.5, 0.0, 0.0], 1.0, 0.5, 0.5), [0.0, 0.5, 0.0]);
    assert_eq!(
      ProfileRender::new(&profile, 0, None)
        .expect("render")
        .apply_pixel([0.5, 0.0, 0.0], 1.0, 0.5, 0.5),
      [0.5, 0.0, 0.0]
    );
    // Slot 2 has no table
    assert!(ProfileRender::new(&profile, 2, None).is_none());
    profile.tone_curve = Some(vec![0.0, 0.0, 0.5, 0.25, 1.0, 1.0]);
    let render = ProfileRender::new(&profile, 2, None).expect("render");
    assert!(render.hue_sat_map.is_none() && render.tone_curve.is_some());
  }

  fn assert_rgb_eq(a: [f32; 3], b: [f32; 3]) {
    for (x, y) in a.iter().zip(b) {
      assert!((x - y).abs() < 1e-5, "{:?} != {:?}", a, b);
    }
  }

  fn gain_table_map(gains: Vec<f32>) -> GainTableMap {
    GainTableMap {
      points: [1, 2, 1],
      spacing: [1.0, 1.0],
      origin: [0.0, 0.0],
      input_weights: [0.0, 0.0, 0.0, 0.0, 1.0],
      gamma: 1.0,
      gain_min: 1.0,
      gain_max: 1.0,
      gains: GainData::F32(gains),
    }
  }

  #[test]
  fn gain_table_map_across_image() {
    // Gain 1.0 at the left and 3.0 at the right edge
    let render = ProfileRender::new(&CameraProfile::default(), 0, Some(&gain_table_map(vec![1.0, 3.0]))).expect("render");
    let mut image = RgbF32::new_with(vec![[0.2, 0.1, 0.1]; 4], 2, 2);
    // The map covers the right column only
    render.apply(&mut image, 1.0, Rect::new(Point::new(1, 0), Dim2::new(1, 2)));
    // Left column: the pixel center is outside of the map (left edge), right column: the center
    assert_eq!(image.data[0], [0.2, 0.1, 0.1]);
    assert_eq!(image.data[1], [0.4, 0.2, 0.2]);
    assert_eq!(image.data[3], [0.4, 0.2, 0.2]);
  }

  #[test]
  fn exposure_and_clipping() {
    // A hue/saturation map works on the values before the exposure
    let mut profile = CameraProfile::default();
    profile.hue_sat_map_dims = Some([1, 2, 1]);
    profile.hue_sat_map_data = [Some([0.0, 1.0, 1.0].repeat(2)), None, None];
    let render = ProfileRender::new(&profile, 0, Some(&gain_table_map(vec![0.5, 0.5]))).expect("render");
    // 1.6 is below the white point of 2.0, the gain table map halves it
    assert_rgb_eq(render.apply_pixel([1.6, 0.8, 0.4], 2.0, 0.5, 0.5), [0.8, 0.4, 0.2]);
    // Values are limited to the white point, then clipped to 1.0 after the gain table map
    assert_rgb_eq(render.apply_pixel([3.0, 0.4, 0.4], 1.0, 0.5, 0.5), [0.5, 0.2, 0.2]);
    let without_map = ProfileRender::new(&profile, 0, None).expect("render");
    assert_rgb_eq(without_map.apply_pixel([1.6, 0.8, 0.4], 2.0, 0.5, 0.5), [1.0, 0.8, 0.4]);
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
