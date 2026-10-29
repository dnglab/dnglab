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
use crate::pixarray::ImageViewMut;

/// Rendering tables of a camera profile for one calibration slot
///
/// A negative BaselineExposure is part of the rendering, too: it is applied
/// as a tone curve that keeps white at 1.0. A positive BaselineExposure is a
/// linear gain that is applied to the image before.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileRender {
  pub hue_sat_map: Option<HueSatMap>,
  pub gain_table: Option<GainTable>,
  pub look_table: Option<HueSatMap>,
  /// ProfileToneCurve after the curve of a negative BaselineExposure
  pub tone_curve: Option<ToneCurve>,
  /// Linear gain of a positive BaselineExposure, applied before
  exposure_gain: f32,
  /// Scale of the gain table map input for a negative BaselineExposure
  weight_scale: f32,
}

impl ProfileRender {
  /// Tables of `profile` with the hue/saturation map of calibration slot
  /// `slot`, the gain table map that applies to the profile and the
  /// BaselineExposure `exposure` in EV
  ///
  /// Returns `None` if there is neither a valid table nor a negative exposure.
  pub fn new(profile: Option<(&CameraProfile, usize)>, gain_table_map: Option<&GainTableMap>, exposure: f32) -> Option<Self> {
    let hue_sat_map = profile.and_then(|(profile, slot)| {
      profile
        .hue_sat_map_dims
        .zip(profile.hue_sat_map_data.get(slot).and_then(Option::as_deref))
        .and_then(|(dims, data)| HueSatMap::new(dims, data, profile.hue_sat_map_encoding))
    });
    let look_table = profile.and_then(|(profile, _)| {
      profile
        .look_table_dims
        .zip(profile.look_table_data.as_deref())
        .and_then(|(dims, data)| HueSatMap::new(dims, data, profile.look_table_encoding))
    });
    let profile_curve = profile.and_then(|(profile, _)| profile.tone_curve.as_deref()).and_then(ToneCurve::new);
    let tone_curve = match (ToneCurve::negative_exposure(exposure), profile_curve) {
      (Some(exposure_curve), Some(curve)) => Some(curve.after(&exposure_curve)),
      (exposure_curve, curve) => exposure_curve.or(curve),
    };
    let gain_table = gain_table_map.and_then(GainTable::new);
    if hue_sat_map.is_none() && gain_table.is_none() && look_table.is_none() && tone_curve.is_none() {
      return None;
    }
    Some(Self {
      hue_sat_map,
      gain_table,
      look_table,
      tone_curve,
      exposure_gain: exposure.max(0.0).exp2(),
      weight_scale: exposure.min(0.0).exp2(),
    })
  }

  /// Linear gain of a positive BaselineExposure, which the image must be scaled by
  /// before [`Self::apply`], 1.0 otherwise
  pub fn exposure_gain(&self) -> f32 {
    self.exposure_gain
  }

  /// Apply the tables to an image of linear ProPhoto RGB (D50)
  ///
  /// The image is already scaled by [`Self::exposure_gain`]. `area` is the
  /// image area the gain table map refers to (the stage 3 image), relative to
  /// `image`. Fails if `image` has not three channels.
  pub fn apply<'a>(&self, image: impl Into<ImageViewMut<'a, f32>>, area: Rect) -> crate::Result<()> {
    self.apply_view(image.into(), area)
  }

  fn apply_view(&self, mut image: ImageViewMut<'_, f32>, area: Rect) -> crate::Result<()> {
    let (row_width, channels) = (image.width().max(1), image.channels());
    let pixels = image
      .as_array_slice_mut::<3>()
      .ok_or_else(|| format!("Rendering tables require three channels, image has {}", channels))?;
    let (left, top) = (area.p.x as f32, area.p.y as f32);
    let (width, height) = (area.d.w.max(1) as f32, area.d.h.max(1) as f32);
    pixels.par_chunks_mut(row_width).enumerate().for_each(|(row, pixels)| {
      // Positions of the pixel centers, relative to the image area
      let v = (row as f32 + 0.5 - top) / height;
      for (col, pixel) in pixels.iter_mut().enumerate() {
        let u = (col as f32 + 0.5 - left) / width;
        *pixel = self.apply_pixel(*pixel, u, v);
      }
    });
    Ok(())
  }

  /// Apply the tables to one pixel at the relative position `u` (horizontal)
  /// and `v` (vertical)
  ///
  /// Like in a DNG reader, the values are limited to the white point, the
  /// hue/saturation map is applied before the exposure and the values are
  /// clipped to 1.0 after the gain table map. The gain table map input
  /// includes the BaselineExposure.
  pub fn apply_pixel(&self, rgb: [f32; 3], u: f32, v: f32) -> [f32; 3] {
    let gain = self.exposure_gain;
    let mut rgb = rgb.map(|c| c.clamp(0.0, gain));
    if let Some(table) = &self.hue_sat_map {
      rgb = table.apply(rgb.map(|c| c / gain)).map(|c| c * gain);
    }
    if let Some(table) = &self.gain_table {
      let map_gain = table.gain(rgb.map(|c| c * self.weight_scale), u, v);
      rgb = rgb.map(|c| c * map_gain);
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
  use crate::pixarray::RgbF32;

  #[test]
  fn profile_render_of_slot() {
    let mut profile = CameraProfile::default();
    assert!(ProfileRender::new(Some((&profile, 0)), None, 0.0).is_none());
    // Hue shift of 120 degrees in slot 1, none in slot 0
    profile.hue_sat_map_dims = Some([2, 2, 1]);
    profile.hue_sat_map_data = [Some([0.0, 1.0, 1.0].repeat(4)), Some([120.0, 1.0, 1.0].repeat(4)), None];
    let render = ProfileRender::new(Some((&profile, 1)), None, 0.0).expect("render");
    assert_eq!(render.apply_pixel([0.5, 0.0, 0.0], 0.5, 0.5), [0.0, 0.5, 0.0]);
    assert_eq!(
      ProfileRender::new(Some((&profile, 0)), None, 0.0)
        .expect("render")
        .apply_pixel([0.5, 0.0, 0.0], 0.5, 0.5),
      [0.5, 0.0, 0.0]
    );
    // Slot 2 has no table
    assert!(ProfileRender::new(Some((&profile, 2)), None, 0.0).is_none());
    profile.tone_curve = Some(vec![0.0, 0.0, 0.5, 0.25, 1.0, 1.0]);
    let render = ProfileRender::new(Some((&profile, 2)), None, 0.0).expect("render");
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
    let render = ProfileRender::new(None, Some(&gain_table_map(vec![1.0, 3.0])), 0.0).expect("render");
    let mut image = RgbF32::new_with(vec![[0.2, 0.1, 0.1]; 4], 2, 2);
    // The map covers the right column only
    render.apply(&mut image, Rect::new(Point::new(1, 0), Dim2::new(1, 2))).expect("three channels");
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
    // BaselineExposure of +1 EV, the white point is 2.0
    let render = ProfileRender::new(Some((&profile, 0)), Some(&gain_table_map(vec![0.5, 0.5])), 1.0).expect("render");
    assert_eq!(render.exposure_gain(), 2.0);
    // 1.6 is below the white point, the gain table map halves it
    assert_rgb_eq(render.apply_pixel([1.6, 0.8, 0.4], 0.5, 0.5), [0.8, 0.4, 0.2]);
    // Values are limited to the white point, then clipped to 1.0 after the gain table map
    let render = ProfileRender::new(Some((&profile, 0)), Some(&gain_table_map(vec![0.5, 0.5])), 0.0).expect("render");
    assert_rgb_eq(render.apply_pixel([3.0, 0.4, 0.4], 0.5, 0.5), [0.5, 0.2, 0.2]);
    let without_map = ProfileRender::new(Some((&profile, 0)), None, 1.0).expect("render");
    assert_rgb_eq(without_map.apply_pixel([1.6, 0.8, 0.4], 0.5, 0.5), [1.0, 0.8, 0.4]);
  }

  #[test]
  fn negative_exposure() {
    // Without any table, a negative exposure is rendered as tone curve
    assert!(ProfileRender::new(None, None, 0.0).is_none());
    let render = ProfileRender::new(None, None, -1.0).expect("render");
    assert_eq!(render.exposure_gain(), 1.0);
    assert_rgb_eq(render.apply_pixel([1.0; 3], 0.5, 0.5), [1.0; 3]);
    assert_rgb_eq(render.apply_pixel([0.2; 3], 0.5, 0.5), [0.1; 3]);
    // The gain table map input includes the exposure: weight 0.6 * 0.5 selects
    // gain 1.4 between 2.0 (dark) and 1.0 (bright)
    let mut map = gain_table_map(vec![2.0, 1.0]);
    map.points = [1, 1, 2];
    let render = ProfileRender::new(None, Some(&map), -1.0).expect("render");
    let curve = ToneCurve::negative_exposure(-1.0).expect("curve");
    assert_rgb_eq(render.apply_pixel([0.6, 0.3, 0.3], 0.5, 0.5), curve.apply_rgb([0.84, 0.42, 0.42]));
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
