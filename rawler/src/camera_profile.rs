// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Camera profiles as defined by the DNG specification
//!
//! A camera profile maps camera native colors to CIE XYZ. Its matrices and
//! tables are stored in slots, each slot belongs to one calibration illuminant
//! (CalibrationIlluminant1..3). Slot 0 holds the values of the tags ending in 1.

use std::collections::HashMap;

use log::debug;

use crate::dng::{DNG_VERSION_V1_0, DNG_VERSION_V1_6, DngVersionNumber};
use crate::imgop::chromatic_adaption::adapt_xyz2cam_bradford;
use crate::imgop::matrix::{FlatMatrix, determinant, multiply, normalize, project_rows_to_simplex, pseudo_inverse, transform_1d, transform_2d};
use crate::imgop::xyz::{
  CIE_1931_WHITE_POINT_A, CIE_1931_WHITE_POINT_B, CIE_1931_WHITE_POINT_C, CIE_1931_WHITE_POINT_D50, CIE_1931_WHITE_POINT_D55, CIE_1931_WHITE_POINT_D65,
  CIE_1931_WHITE_POINT_D75, Illuminant, XYZ_TO_PROFOTORGB_D50,
};

/// Number of calibration illuminant slots of a camera profile
pub const CALIBRATION_SLOTS: usize = 3;

/// Illuminants in order of preference for color calculations based on D65.
/// Color matrices of these illuminants can be adapted to D65.
pub const D65_PREFERENCE: [Illuminant; 9] = [
  Illuminant::D65,
  Illuminant::A,
  Illuminant::B,
  Illuminant::C,
  Illuminant::D50,
  Illuminant::D55,
  Illuminant::D75,
  Illuminant::Daylight,
  Illuminant::Flash,
];

/// Sort key for illuminants in order of [`D65_PREFERENCE`], other illuminants follow by their EXIF value
pub(crate) fn d65_preference(illuminant: &Illuminant) -> (usize, u16) {
  let position = D65_PREFERENCE.iter().position(|x| x == illuminant).unwrap_or(D65_PREFERENCE.len());
  (position, u16::from(*illuminant))
}

/// Correlated color temperature of a standard illuminant, 0.0 if unknown
pub fn illuminant_temperature(illuminant: Illuminant) -> f32 {
  match illuminant {
    Illuminant::A | Illuminant::Tungsten => 2850.0,
    Illuminant::IsoStudioTungsten => 3200.0,
    Illuminant::D50 => 5000.0,
    Illuminant::D55 | Illuminant::Daylight | Illuminant::FineWeather | Illuminant::Flash | Illuminant::B => 5500.0,
    Illuminant::D65 | Illuminant::C | Illuminant::CloudyWeather => 6500.0,
    Illuminant::D75 | Illuminant::Shade => 7500.0,
    Illuminant::DaylightFluorescent => (5700.0 + 7100.0) * 0.5,
    Illuminant::DaylightWhiteFluorescent => (4600.0 + 5500.0) * 0.5,
    Illuminant::CoolWhiteFluorescent | Illuminant::Fluorescent => (3800.0 + 4500.0) * 0.5,
    Illuminant::WhiteFluorescent => (3250.0 + 3800.0) * 0.5,
    Illuminant::Unknown | Illuminant::Other => 0.0,
  }
}

/// Standard illuminant with the white point nearest to x-y chromaticity `(x, y)`
pub fn nearest_standard_illuminant(x: f32, y: f32) -> Illuminant {
  [
    (Illuminant::A, CIE_1931_WHITE_POINT_A),
    (Illuminant::B, CIE_1931_WHITE_POINT_B),
    (Illuminant::C, CIE_1931_WHITE_POINT_C),
    (Illuminant::D50, CIE_1931_WHITE_POINT_D50),
    (Illuminant::D55, CIE_1931_WHITE_POINT_D55),
    (Illuminant::D65, CIE_1931_WHITE_POINT_D65),
    (Illuminant::D75, CIE_1931_WHITE_POINT_D75),
  ]
  .into_iter()
  .min_by(|(_, a), (_, b)| {
    let distance = |(wx, wy): (f32, f32)| (wx - x).powi(2) + (wy - y).powi(2);
    distance(*a).total_cmp(&distance(*b))
  })
  .map(|(illuminant, _)| illuminant)
  .unwrap_or(Illuminant::Unknown)
}

/// DNG IlluminantData, describes a custom calibration illuminant ([`Illuminant::Other`])
#[derive(Debug, Clone, PartialEq)]
pub enum IlluminantData {
  /// x-y chromaticity coordinate
  Xy { x: f32, y: f32 },
  /// Spectral power distribution, samples in nanometers
  Spectral { min_lambda: f32, lambda_spacing: f32, values: Vec<f32> },
}

/// DNG ProfileDynamicRange
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProfileDynamicRange {
  pub version: u16,
  /// 0 = Standard Dynamic Range, 1 = High Dynamic Range
  pub dynamic_range: u16,
  /// Intended maximum output value (linear), <= 0.0 if unknown
  pub hint_max_output_value: f32,
}

impl ProfileDynamicRange {
  pub fn is_hdr(&self) -> bool {
    self.dynamic_range == 1
  }
}

/// DNG RGBTables, color lookup tables applied to semantic mask regions
#[derive(Debug, Clone, PartialEq)]
pub struct RgbTables {
  /// 0 = weighted sum, 1 = sequential
  pub composite_method: u32,
  pub tables: Vec<RgbTable>,
}

/// Single table of [`RgbTables`]
#[derive(Debug, Clone, PartialEq)]
pub struct RgbTable {
  /// Name of the semantic mask, `None` for the background table
  pub semantic_name: Option<String>,
  /// The table has divisions³ entries of three values (r, g, b)
  pub divisions: u8,
  pub gamma_encoding: u8,
  pub color_primaries: u8,
  pub gamut_extension: u8,
  pub data: TableData,
}

/// Table values in the stored pixel type
#[derive(Debug, Clone, PartialEq)]
pub enum TableData {
  U8(Vec<u8>),
  U16(Vec<u16>),
  F32(Vec<f32>),
}

impl TableData {
  pub fn len(&self) -> usize {
    match self {
      Self::U8(v) => v.len(),
      Self::U16(v) => v.len(),
      Self::F32(v) => v.len(),
    }
  }

  pub fn is_empty(&self) -> bool {
    self.len() == 0
  }
}

/// DNG ProfileGainTableMap2
#[derive(Debug, Clone, PartialEq)]
pub struct GainTableMap {
  /// MapPointsV, MapPointsH, MapPointsN
  pub points: [u32; 3],
  /// MapSpacingV, MapSpacingH
  pub spacing: [f64; 2],
  /// MapOriginV, MapOriginH
  pub origin: [f64; 2],
  pub input_weights: [f32; 5],
  pub gamma: f32,
  pub gain_min: f32,
  pub gain_max: f32,
  pub gains: GainData,
}

/// Gain values in the stored data type
#[derive(Debug, Clone, PartialEq)]
pub enum GainData {
  U8(Vec<u8>),
  U16(Vec<u16>),
  /// IEEE 754 half precision, stored as raw bits
  F16(Vec<u16>),
  F32(Vec<f32>),
}

impl GainTableMap {
  /// Gains as floating point values, integer gains are scaled to GainMin ..= GainMax
  pub fn gain_values(&self) -> Vec<f32> {
    let scale = |value: f32, max: f32| self.gain_min + value / max * (self.gain_max - self.gain_min);
    match &self.gains {
      GainData::U8(values) => values.iter().map(|value| scale(*value as f32, 255.0)).collect(),
      GainData::U16(values) => values.iter().map(|value| scale(*value as f32, 65535.0)).collect(),
      GainData::F16(values) => values.iter().map(|value| half::f16::from_bits(*value).to_f32()).collect(),
      GainData::F32(values) => values.clone(),
    }
  }
}

impl GainData {
  pub fn len(&self) -> usize {
    match self {
      Self::U8(v) => v.len(),
      Self::U16(v) | Self::F16(v) => v.len(),
      Self::F32(v) => v.len(),
    }
  }

  pub fn is_empty(&self) -> bool {
    self.len() == 0
  }
}

/// DNG camera profile
///
/// A slot exists if its color matrix exists. The other per-slot values
/// belong to the calibration illuminant of the same slot.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CameraProfile {
  /// ProfileName
  pub name: Option<String>,
  /// ProfileCalibrationSignature
  pub calibration_signature: Option<String>,
  /// ProfileCopyright
  pub copyright: Option<String>,
  /// ProfileEmbedPolicy
  pub embed_policy: Option<u32>,
  /// ProfileGroupName
  pub group_name: Option<String>,
  /// CalibrationIlluminant1..3, missing values are [`Illuminant::Unknown`]
  pub calibration_illuminant: [Option<Illuminant>; CALIBRATION_SLOTS],
  /// IlluminantData1..3, used for [`Illuminant::Other`]
  pub illuminant_data: [Option<IlluminantData>; CALIBRATION_SLOTS],
  /// ColorMatrix1..3 (XYZ to camera, color planes x 3)
  pub color_matrix: [Option<FlatMatrix>; CALIBRATION_SLOTS],
  /// ForwardMatrix1..3 (white balanced camera to XYZ D50, 3 x color planes)
  pub forward_matrix: [Option<FlatMatrix>; CALIBRATION_SLOTS],
  /// ReductionMatrix1..3 (3 x color planes)
  pub reduction_matrix: [Option<FlatMatrix>; CALIBRATION_SLOTS],
  /// ProfileHueSatMapData1..3
  pub hue_sat_map_data: [Option<Vec<f32>>; CALIBRATION_SLOTS],
  /// ProfileHueSatMapDims (hue, saturation, value divisions)
  pub hue_sat_map_dims: Option<[u32; 3]>,
  pub hue_sat_map_encoding: Option<u32>,
  /// ProfileLookTableDims (hue, saturation, value divisions)
  pub look_table_dims: Option<[u32; 3]>,
  pub look_table_data: Option<Vec<f32>>,
  pub look_table_encoding: Option<u32>,
  /// ProfileToneCurve, pairs of input and output values
  pub tone_curve: Option<Vec<f32>>,
  /// BaselineExposureOffset in EV
  pub baseline_exposure_offset: Option<f32>,
  pub default_black_render: Option<u32>,
  pub dynamic_range: Option<ProfileDynamicRange>,
  pub rgb_tables: Option<RgbTables>,
  /// ProfileGainTableMap2
  pub gain_table_map: Option<GainTableMap>,
}

/// Calibration of the individual camera (CameraCalibration1..3 in IFD 0)
///
/// It is not part of a camera profile, but its slots belong to the
/// calibration illuminants of the profile.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CameraCalibration {
  /// CameraCalibration1..3 (color planes x color planes)
  pub matrices: [Option<FlatMatrix>; CALIBRATION_SLOTS],
  /// CameraCalibrationSignature
  pub signature: Option<String>,
}

impl CameraCalibration {
  pub fn is_empty(&self) -> bool {
    self.matrices.iter().all(Option::is_none) && self.signature.is_none()
  }

  /// The calibration is only used with profiles of the same signature,
  /// a missing signature on both sides is a match.
  pub fn applies_to(&self, profile: &CameraProfile) -> bool {
    self.signature.as_deref().unwrap_or_default() == profile.calibration_signature.as_deref().unwrap_or_default()
  }
}

/// One calibration illuminant of a camera profile, flattened for processing
#[derive(Debug, Clone, Copy)]
pub struct CalibrationSlot<'a> {
  /// Slot index, the tag suffix is `slot + 1`
  pub slot: usize,
  pub illuminant: Illuminant,
  pub illuminant_data: Option<&'a IlluminantData>,
  pub color_matrix: &'a FlatMatrix,
  pub forward_matrix: Option<&'a FlatMatrix>,
  pub reduction_matrix: Option<&'a FlatMatrix>,
  pub hue_sat_map_data: Option<&'a [f32]>,
  /// CameraCalibration of the slot, only if the calibration applies to the profile
  pub camera_calibration: Option<&'a FlatMatrix>,
  pub baseline_exposure_offset: Option<f32>,
  pub default_black_render: Option<u32>,
  /// The profile for all other values
  pub profile: &'a CameraProfile,
}

impl CameraProfile {
  /// Build a profile from color and reduction matrices by illuminant (camera definitions)
  ///
  /// Slots are ordered by increasing color temperature, as required for the
  /// interpolation of two calibration illuminants.
  pub fn from_catalog(color_matrix: &HashMap<Illuminant, FlatMatrix>, reduction_matrix: &HashMap<Illuminant, FlatMatrix>) -> Option<Self> {
    let mut illuminants: Vec<Illuminant> = color_matrix.keys().copied().collect();
    illuminants.sort_by(|a, b| {
      illuminant_temperature(*a)
        .total_cmp(&illuminant_temperature(*b))
        .then(u16::from(*a).cmp(&u16::from(*b)))
    });
    if illuminants.len() > CALIBRATION_SLOTS {
      log::warn!(
        "Camera has {} color matrices, only {} are used: {:?}",
        illuminants.len(),
        CALIBRATION_SLOTS,
        illuminants
      );
    }
    let mut profile = Self::default();
    for (slot, illuminant) in illuminants.into_iter().take(CALIBRATION_SLOTS).enumerate() {
      profile.calibration_illuminant[slot] = Some(illuminant);
      profile.color_matrix[slot] = color_matrix.get(&illuminant).cloned();
      profile.reduction_matrix[slot] = reduction_matrix.get(&illuminant).cloned();
    }
    if profile.slots().next().is_some() { Some(profile) } else { None }
  }

  /// Build a profile from a matrix that maps white balanced camera values to
  /// XYZ D50 (like a DNG ForwardMatrix) and the camera neutral of the white balance
  ///
  /// The profile has one ColorMatrix for D50, the white of `neutral` is declared
  /// to be the D50 white. This reproduces a pipeline that applies the white
  /// balance and then a fixed matrix, like the one of Phase One IIQ files.
  pub fn from_forward_matrix_d50(forward_matrix: &[[f32; 3]; 3], neutral: &[f32; 3]) -> Option<Self> {
    if !neutral.iter().all(|value| value.is_finite() && *value > 0.0) {
      return None;
    }
    // XYZ D50 to white balanced camera values, scaled to camera values of the neutral
    let xyz2cam = pseudo_inverse(*forward_matrix);
    let color_matrix: [[f32; 3]; 3] = std::array::from_fn(|row| xyz2cam[row].map(|value| value * neutral[row]));
    if !color_matrix.as_flattened().iter().all(|value| value.is_finite()) {
      return None;
    }
    let mut profile = Self::default();
    profile.calibration_illuminant[0] = Some(Illuminant::D50);
    profile.color_matrix[0] = Some(transform_2d(&color_matrix));
    Some(profile)
  }

  /// Indices of the slots with a color matrix
  pub fn slots(&self) -> impl Iterator<Item = usize> + '_ {
    (0..CALIBRATION_SLOTS).filter(|slot| self.color_matrix[*slot].is_some())
  }

  /// Calibration illuminant of `slot`
  pub fn illuminant(&self, slot: usize) -> Illuminant {
    self.calibration_illuminant[slot].unwrap_or(Illuminant::Unknown)
  }

  /// First slot for `illuminant`
  pub fn slot_of(&self, illuminant: Illuminant) -> Option<usize> {
    self.slots().find(|slot| self.illuminant(*slot) == illuminant)
  }

  /// First slot for the first illuminant of `preference` that has one,
  /// or the first slot if none matches
  pub fn best_slot(&self, preference: &[Illuminant]) -> Option<usize> {
    preference
      .iter()
      .find_map(|illuminant| self.slot_of(*illuminant))
      .or_else(|| self.slots().next())
  }

  /// Best slot for color calculations based on D65, see [`D65_PREFERENCE`]
  pub fn best_d65_slot(&self) -> Option<usize> {
    self.best_slot(&D65_PREFERENCE)
  }

  /// Number of color planes, given by the color matrix size
  pub fn color_planes(&self) -> Option<usize> {
    self
      .slots()
      .next()
      .and_then(|slot| self.color_matrix[slot].as_ref())
      .map(|matrix| matrix.len() / 3)
  }

  /// Flatten `slot`, the camera calibration is only used if it applies to this profile
  pub fn calibration_slot<'a>(&'a self, slot: usize, calibration: Option<&'a CameraCalibration>) -> Option<CalibrationSlot<'a>> {
    let calibration = calibration.filter(|calibration| calibration.applies_to(self));
    Some(CalibrationSlot {
      slot,
      illuminant: self.illuminant(slot),
      illuminant_data: self.illuminant_data[slot].as_ref(),
      color_matrix: self.color_matrix[slot].as_ref()?,
      forward_matrix: self.forward_matrix[slot].as_ref(),
      reduction_matrix: self.reduction_matrix[slot].as_ref(),
      hue_sat_map_data: self.hue_sat_map_data[slot].as_deref(),
      camera_calibration: calibration.and_then(|calibration| calibration.matrices[slot].as_ref()),
      baseline_exposure_offset: self.baseline_exposure_offset,
      default_black_render: self.default_black_render,
      profile: self,
    })
  }

  /// Flatten all slots, see [`Self::calibration_slot`]
  pub fn unpack<'a>(&'a self, calibration: Option<&'a CameraCalibration>) -> Vec<CalibrationSlot<'a>> {
    self.slots().filter_map(|slot| self.calibration_slot(slot, calibration)).collect()
  }

  /// The profile is intended for High Dynamic Range rendering
  pub fn is_hdr(&self) -> bool {
    self.dynamic_range.is_some_and(|range| range.is_hdr())
  }

  /// Minimum DNGBackwardVersion for this profile
  ///
  /// Custom data for illuminant 1 or 2 breaks readers before DNG 1.6.
  pub fn min_backward_version(&self) -> DngVersionNumber {
    let custom = (0..2)
      .any(|slot| self.color_matrix[slot].is_some() && self.calibration_illuminant[slot] == Some(Illuminant::Other) && self.illuminant_data[slot].is_some());
    if custom { DNG_VERSION_V1_6 } else { DNG_VERSION_V1_0 }
  }

  /// Replace the custom illuminants of slot 0 and 1 by standard illuminants,
  /// for readers before DNG 1.6 (see [`Self::min_backward_version`])
  ///
  /// x-y data is replaced by the nearest standard illuminant, spectral data
  /// by [`Illuminant::Unknown`]. The custom illuminant data is dropped.
  pub fn with_standard_illuminants(&self) -> Self {
    let mut profile = self.clone();
    for slot in 0..2 {
      if profile.calibration_illuminant[slot] == Some(Illuminant::Other) {
        let illuminant = match profile.illuminant_data[slot].take() {
          Some(IlluminantData::Xy { x, y }) => nearest_standard_illuminant(x, y),
          _ => Illuminant::Unknown,
        };
        profile.calibration_illuminant[slot] = Some(illuminant);
      }
    }
    profile
  }

  /// Reduction matrix for each slot with a four color planes color matrix
  ///
  /// Matrices of the profile are used as they are, missing ones are
  /// replaced by [`Self::calc_reduction_matrix`].
  pub fn reduction_matrices(&self) -> [Option<FlatMatrix>; CALIBRATION_SLOTS] {
    let calculated = self.calc_reduction_matrix().map(|matrix| matrix.as_flattened().to_vec());
    std::array::from_fn(|slot| {
      self.color_matrix[slot].as_ref().filter(|matrix| matrix.len() == 12)?;
      self.reduction_matrix[slot].clone().filter(|matrix| matrix.len() == 12).or(calculated.clone())
    })
  }

  /// Calculate a reduction matrix (3x4) for four color planes from the color matrix
  /// for D65, or from another color matrix adapted to D65 by Bradford.
  ///
  /// The matrix has non-negative entries and each row sums to 1.0, so the camera
  /// white (1, 1, 1, 1) is mapped to (1, 1, 1), values never leave the range
  /// 0.0 .. 1.0 and clipped channels stay mostly clipped. A matrix with negative
  /// entries turns partially clipped highlights into noisy, wrong colors when the
  /// image data is reduced. The matrix is the projection of the least squares
  /// mapping into a wide gamut RGB space onto such matrices.
  pub fn calc_reduction_matrix(&self) -> Option<[[f32; 4]; 3]> {
    let (illuminant, matrix) = self
      .slots()
      .filter_map(|slot| {
        let matrix = transform_1d::<4, 3>(self.color_matrix[slot].as_ref()?)?;
        Some((self.illuminant(slot), matrix))
      })
      .min_by_key(|(illuminant, _)| d65_preference(illuminant))?;
    let xyz2cam = if illuminant == Illuminant::D65 {
      matrix
    } else if D65_PREFERENCE.contains(&illuminant) {
      adapt_xyz2cam_bradford(&illuminant, &Illuminant::D65, &matrix)
    } else {
      debug!("Color matrix for {:?} can't be adapted to D65, it is used as it is", illuminant);
      matrix
    };
    // The reduced planes are only interpreted by the reduced color matrix, so
    // any RGB space works as target. The primaries of ProPhoto RGB enclose
    // nearly all colors a camera records, so the camera to ProPhoto mapping
    // has small negative entries only and the projection to non-negative rows
    // below changes it little. A camera to sRGB mapping has large negative
    // entries, its projection loses more color separation and is more likely
    // degenerate. Only the primaries are used: normalize() maps RGB white to
    // the camera white (1, 1, 1, 1) instead of the D50 white of ProPhoto RGB.
    let prophoto2xyz = pseudo_inverse(XYZ_TO_PROFOTORGB_D50);
    // Least squares mapping from the camera planes to ProPhoto RGB
    let wide_gamut = pseudo_inverse(normalize(multiply(&xyz2cam, &prophoto2xyz)));
    let projected = project_rows_to_simplex(wide_gamut);
    // The reduced color matrix must stay invertible, compare its determinant
    // to the volume of its rows (Hadamard's inequality).
    let reduced = multiply(&projected, &xyz2cam);
    let volume: f32 = reduced.iter().map(|row| row.iter().map(|v| v * v).sum::<f32>().sqrt()).product();
    if determinant(&reduced).abs() > 0.01 * volume {
      debug!("Reduction matrix {:?} based on color matrix for {:?}", projected, illuminant);
      Some(projected)
    } else {
      log::warn!("Non-negative reduction matrix is degenerate, highlights may clip");
      Some(wide_gamut)
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::imgop::matrix::transform_2d;
  use crate::imgop::xyz::XYZ_TO_SRGB_D65;

  /// XYZ to camera matrix of a CYGM sensor: cyan = G + B, yellow = R + G, green = G, magenta = R + B
  fn cygm_color_matrix() -> FlatMatrix {
    let rgb2cam = [[0.0, 1.0, 1.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0, 1.0]];
    transform_2d(&multiply(&rgb2cam, &XYZ_TO_SRGB_D65))
  }

  fn catalog(color_matrix: &[(Illuminant, FlatMatrix)], reduction_matrix: &[(Illuminant, FlatMatrix)]) -> Option<CameraProfile> {
    CameraProfile::from_catalog(&color_matrix.iter().cloned().collect(), &reduction_matrix.iter().cloned().collect())
  }

  fn illuminants(profile: &CameraProfile) -> Vec<Illuminant> {
    profile.slots().map(|slot| profile.illuminant(slot)).collect()
  }

  #[test]
  fn catalog_slots_by_temperature() {
    let m = || vec![1.0; 9];
    for (input, expected) in [
      (vec![Illuminant::D65, Illuminant::A], vec![Illuminant::A, Illuminant::D65]),
      (vec![Illuminant::D75, Illuminant::D55], vec![Illuminant::D55, Illuminant::D75]),
      (vec![Illuminant::Flash, Illuminant::Tungsten], vec![Illuminant::Tungsten, Illuminant::Flash]),
      (vec![Illuminant::D65, Illuminant::Daylight], vec![Illuminant::Daylight, Illuminant::D65]),
      (vec![Illuminant::D65], vec![Illuminant::D65]),
    ] {
      let input: Vec<_> = input.into_iter().map(|illuminant| (illuminant, m())).collect();
      let profile = catalog(&input, &[]).expect("profile");
      assert_eq!(illuminants(&profile), expected);
    }
    assert!(catalog(&[], &[]).is_none());
  }

  #[test]
  fn best_slot_falls_back_to_first_slot() {
    let mut profile = CameraProfile::default();
    profile.color_matrix[1] = Some(vec![1.0; 9]);
    profile.calibration_illuminant[1] = Some(Illuminant::Other);
    profile.color_matrix[2] = Some(vec![2.0; 9]);
    assert_eq!(profile.slots().collect::<Vec<_>>(), vec![1, 2]);
    // Missing CalibrationIlluminant is Unknown
    assert_eq!(profile.illuminant(2), Illuminant::Unknown);
    assert_eq!(profile.best_d65_slot(), Some(1));
    profile.calibration_illuminant[2] = Some(Illuminant::A);
    assert_eq!(profile.best_d65_slot(), Some(2));
    assert_eq!(profile.slot_of(Illuminant::A), Some(2));
    // A duplicate illuminant resolves to the first slot
    profile.calibration_illuminant[1] = Some(Illuminant::A);
    assert_eq!(profile.slot_of(Illuminant::A), Some(1));
  }

  #[test]
  fn calibration_requires_matching_signature() {
    let mut profile = catalog(&[(Illuminant::A, vec![1.0; 9]), (Illuminant::D65, vec![2.0; 9])], &[]).expect("profile");
    let mut calibration = CameraCalibration::default();
    calibration.matrices[1] = Some(vec![3.0; 9]);
    let d65 = |profile: &CameraProfile, calibration: &CameraCalibration| {
      profile.calibration_slot(1, Some(calibration)).and_then(|slot| slot.camera_calibration.cloned())
    };
    // No signature on both sides is a match
    assert_eq!(d65(&profile, &calibration), Some(vec![3.0; 9]));
    calibration.signature = Some("com.adobe".into());
    assert_eq!(d65(&profile, &calibration), None);
    profile.calibration_signature = Some("com.adobe".into());
    assert_eq!(d65(&profile, &calibration), Some(vec![3.0; 9]));
    // Slot 0 has no calibration
    assert_eq!(profile.calibration_slot(0, Some(&calibration)).and_then(|slot| slot.camera_calibration), None);
    assert_eq!(profile.unpack(Some(&calibration)).len(), 2);
  }

  #[test]
  fn custom_illuminant_requires_dng_1_6_reader() {
    let mut profile = catalog(&[(Illuminant::D65, vec![1.0; 9])], &[]).expect("profile");
    assert_eq!(profile.min_backward_version(), DNG_VERSION_V1_0);
    profile.calibration_illuminant[0] = Some(Illuminant::Other);
    profile.illuminant_data[0] = Some(IlluminantData::Xy { x: 0.3, y: 0.3 });
    assert_eq!(profile.min_backward_version(), DNG_VERSION_V1_6);

    let standard = profile.with_standard_illuminants();
    assert_eq!(standard.calibration_illuminant[0], Some(Illuminant::D75));
    assert_eq!(standard.illuminant_data[0], None);
    assert_eq!(standard.min_backward_version(), DNG_VERSION_V1_0);

    profile.illuminant_data[0] = Some(IlluminantData::Spectral {
      min_lambda: 380.0,
      lambda_spacing: 10.0,
      values: vec![1.0; 46],
    });
    assert_eq!(profile.with_standard_illuminants().calibration_illuminant[0], Some(Illuminant::Unknown));
    // A custom illuminant in slot 2 doesn't break older readers
    let mut profile = catalog(&[(Illuminant::A, vec![1.0; 9]), (Illuminant::D65, vec![1.0; 9])], &[]).expect("profile");
    profile.color_matrix[2] = Some(vec![1.0; 9]);
    profile.calibration_illuminant[2] = Some(Illuminant::Other);
    profile.illuminant_data[2] = Some(IlluminantData::Xy { x: 0.3, y: 0.3 });
    assert_eq!(profile.min_backward_version(), DNG_VERSION_V1_0);
  }

  #[test]
  fn nearest_standard_illuminants() {
    assert_eq!(nearest_standard_illuminant(0.4476, 0.4074), Illuminant::A);
    assert_eq!(nearest_standard_illuminant(0.3127, 0.3290), Illuminant::D65);
    assert_eq!(nearest_standard_illuminant(0.3460, 0.3590), Illuminant::D50);
  }

  #[test]
  fn reduction_matrices_use_profile_matrices() {
    let provided = vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.5, 0.0, 0.0, 1.0, 0.0];
    let profile = catalog(
      &[(Illuminant::A, cygm_color_matrix()), (Illuminant::D65, cygm_color_matrix())],
      &[(Illuminant::A, provided.clone())],
    )
    .expect("profile");
    let matrices = profile.reduction_matrices();
    assert_eq!(matrices[0], Some(provided));
    // Missing matrices are calculated, rows are non-negative and sum to 1.0
    let calculated = profile.calc_reduction_matrix().expect("reduction matrix");
    assert_eq!(matrices[1], Some(transform_2d(&calculated)));
    assert_eq!(matrices[2], None);
    for row in calculated {
      assert!(row.iter().all(|v| *v >= 0.0), "{:?}", calculated);
      assert!((row.iter().sum::<f32>() - 1.0).abs() < 1e-5, "{:?}", calculated);
    }
  }

  #[test]
  fn reduction_matrix_from_bradford_adapted_matrix() {
    let matrix_a = cygm_color_matrix();
    let adapted = adapt_xyz2cam_bradford(&Illuminant::A, &Illuminant::D65, &transform_1d::<4, 3>(&matrix_a).expect("4x3 matrix"));
    let profile_a = catalog(&[(Illuminant::A, matrix_a)], &[]).expect("profile");
    let profile_d65 = catalog(&[(Illuminant::D65, transform_2d(&adapted))], &[]).expect("profile");
    assert_eq!(profile_a.calc_reduction_matrix(), profile_d65.calc_reduction_matrix());
    assert!(profile_a.calc_reduction_matrix().is_some());
  }

  #[test]
  fn no_reduction_matrix_for_three_color_planes() {
    let profile = catalog(&[(Illuminant::D65, transform_2d(&XYZ_TO_SRGB_D65))], &[]).expect("profile");
    assert!(profile.calc_reduction_matrix().is_none());
    assert_eq!(profile.reduction_matrices(), [None, None, None]);
  }

  /// White balanced camera to XYZ D50 of a Phase One IQ3 100MP (IIQ tag 0x106, camera to ROMM)
  fn iiq_forward_matrix() -> [[f32; 3]; 3] {
    let romm = [[1.193, -0.2, 0.007], [-0.018, 1.129, -0.111], [0.028, -0.173, 1.145]];
    multiply(&pseudo_inverse(XYZ_TO_PROFOTORGB_D50), &romm)
  }

  #[test]
  fn forward_matrix_d50_maps_neutral_to_d50() {
    use crate::imgop::{matrix::multiply_row1, xyz::CIE_1931_TRISTIMULUS_D50};

    let neutral = [0.31, 1.0, 0.76];
    let profile = CameraProfile::from_forward_matrix_d50(&iiq_forward_matrix(), &neutral).expect("profile");
    assert_eq!(profile.slots().collect::<Vec<_>>(), vec![0]);
    assert_eq!(profile.illuminant(0), Illuminant::D50);
    let color_matrix = transform_1d::<3, 3>(profile.color_matrix[0].as_ref().expect("color matrix")).expect("3x3 matrix");
    // The neutral is the camera value of the D50 white and vice versa
    let camera = multiply_row1(&color_matrix, &CIE_1931_TRISTIMULUS_D50);
    let white = multiply_row1(&pseudo_inverse(color_matrix), &neutral);
    for i in 0..3 {
      assert!((camera[i] - neutral[i]).abs() < 1e-3, "camera {:?}", camera);
      assert!((white[i] - CIE_1931_TRISTIMULUS_D50[i]).abs() < 1e-3, "white {:?}", white);
    }
  }

  #[test]
  fn forward_matrix_d50_develops_like_forward_matrix() {
    use crate::imgop::{
      chromatic_adaption::bradford_adaption_matrix,
      xyz::{SRGB_TO_XYZ_D65, XYZ_TO_SRGB_D65},
    };

    let forward_matrix = iiq_forward_matrix();
    let profile = CameraProfile::from_forward_matrix_d50(&forward_matrix, &[0.31, 1.0, 0.76]).expect("profile");
    let color_matrix = transform_1d::<3, 3>(profile.color_matrix[0].as_ref().expect("color matrix")).expect("3x3 matrix");
    // Camera to sRGB as calculated by camera_to_output(): Bradford adaption to D65, then normalized
    let xyz2cam = adapt_xyz2cam_bradford(&Illuminant::D50, &Illuminant::D65, &color_matrix);
    let developed = pseudo_inverse(normalize(multiply(&xyz2cam, &SRGB_TO_XYZ_D65)));
    let expected = multiply(
      &XYZ_TO_SRGB_D65,
      &multiply(&bradford_adaption_matrix(&Illuminant::D50, &Illuminant::D65), &forward_matrix),
    );
    for (developed, expected) in developed.as_flattened().iter().zip(expected.as_flattened()) {
      assert!((developed - expected).abs() < 1e-3, "{:?} != {:?}", developed, expected);
    }
  }

  #[test]
  fn gain_values_of_data_types() {
    let map = |gains| GainTableMap {
      points: [1, 1, 3],
      spacing: [1.0, 1.0],
      origin: [0.0, 0.0],
      input_weights: [0.0; 5],
      gamma: 1.0,
      gain_min: 0.5,
      gain_max: 2.5,
      gains,
    };
    assert_eq!(map(GainData::U8(vec![0, 51, 255])).gain_values(), vec![0.5, 0.9, 2.5]);
    assert_eq!(map(GainData::U16(vec![0, 13107, 65535])).gain_values(), vec![0.5, 0.9, 2.5]);
    assert_eq!(map(GainData::F16(vec![0x3800, 0x3c00, 0x4000])).gain_values(), vec![0.5, 1.0, 2.0]);
    assert_eq!(map(GainData::F32(vec![0.25, 1.0, 4.0])).gain_values(), vec![0.25, 1.0, 4.0]);
  }

  #[test]
  fn forward_matrix_d50_requires_valid_neutral() {
    for neutral in [[0.0, 1.0, 0.5], [f32::NAN, 1.0, 0.5], [-1.0, 1.0, 0.5]] {
      assert!(CameraProfile::from_forward_matrix_d50(&iiq_forward_matrix(), &neutral).is_none());
    }
  }
}
