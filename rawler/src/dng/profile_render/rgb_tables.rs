// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! RGBTables: 3D color lookup tables for the regions of semantic masks
//!
//! Each table works in its own RGB space (ColorPrimaries) and gamma encoding
//! (GammaEncoding). The values are mapped from linear ProPhoto RGB to it,
//! looked up by tetrahedral interpolation and mapped back. Colors outside of
//! the table space are clipped for the lookup, with GamutExtension the clipped
//! part is added to the result. The tables are combined by the weights of
//! their semantic masks, see [`RgbTablesRender::apply`].

use std::sync::Arc;

use super::{decode_overrange, encode_overrange};
use crate::camera_profile::{RgbTable, RgbTables, TableData};
use crate::imgop::{
  chromatic_adaption::bradford_adaption_matrix,
  matrix::{multiply, multiply_vector, pseudo_inverse},
  xyz::{ADOBERGB_TO_XYZ_D65, DISPLAYP3_TO_XYZ_D65, Illuminant, REC2020_TO_XYZ_D65, SRGB_TO_XYZ_D65, XYZ_TO_PROFOTORGB_D50},
};
use crate::mask::SemanticMask;

/// Largest number of tables in RGBTables
const MAX_TABLES: usize = 20;

/// CompositeMethod: 0 = weighted sum, 1 = sequential
const COMPOSITE_SEQUENTIAL: u32 = 1;

/// Intervals of the lookup tables of the gamma encodings
const GAMMA_TABLE_SIZE: usize = 4096;

/// Parameters of the Rec. 709 curve of Rec. 2020
#[allow(clippy::excessive_precision)]
const REC709_ALPHA: f64 = 1.0992968268094429;
const REC709_BETA: f64 = 0.0180539685108078;

/// GammaEncoding of a table
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TableGamma {
  Linear,
  Srgb,
  Gamma18,
  Gamma22,
  Rec2020,
}

impl TableGamma {
  fn from_code(code: u8) -> Option<Self> {
    match code {
      0 => Some(Self::Linear),
      1 => Some(Self::Srgb),
      2 => Some(Self::Gamma18),
      3 => Some(Self::Gamma22),
      4 => Some(Self::Rec2020),
      _ => None,
    }
  }

  /// Toe of the power curves 1.8 and 2.2: a cubic segment
  /// from (0, 0) with slope 32 to (x1, y1), with the slope of the power curve
  /// there. Returns the exponent, x1, y1 and the slope at x1.
  fn power_curve(self) -> Option<(f64, f64, f64, f64)> {
    match self {
      Self::Gamma18 => Some((1.0 / 1.8, 8.2118790552e-4, 0.019310851, 13.064306598)),
      Self::Gamma22 => Some((1.0 / 2.2, 0.0034800731, 0.0763027458, 9.9661890075)),
      _ => None,
    }
  }

  /// Encode a linear value
  fn encode(self, x: f64) -> f64 {
    if let Some((exponent, x1, y1, slope1)) = self.power_curve() {
      return if x <= x1 { spline_segment(x, x1, y1, slope1) } else { x.powf(exponent) };
    }
    match self {
      Self::Srgb if x <= 0.0031308 => x * 12.92,
      Self::Srgb => 1.055 * x.powf(1.0 / 2.4) - 0.055,
      Self::Rec2020 if x <= REC709_BETA => 4.5 * x,
      Self::Rec2020 => REC709_ALPHA * x.powf(0.45) - (REC709_ALPHA - 1.0),
      _ => x,
    }
  }

  /// Decode an encoded value, the inverse of [`Self::encode`]
  fn decode(self, y: f64) -> f64 {
    if let Some((exponent, x1, y1, slope1)) = self.power_curve() {
      if y >= y1 {
        return y.powf(1.0 / exponent);
      }
      // The toe is increasing, its inverse is found by bisection
      let (mut low, mut high) = (0.0, x1);
      for _ in 0..60 {
        let mid = 0.5 * (low + high);
        if spline_segment(mid, x1, y1, slope1) < y {
          low = mid;
        } else {
          high = mid;
        }
      }
      return 0.5 * (low + high);
    }
    match self {
      Self::Srgb if y <= 0.0031308 * 12.92 => y / 12.92,
      Self::Srgb => ((y + 0.055) / 1.055).powf(2.4),
      Self::Rec2020 if y <= 4.5 * REC709_BETA => y / 4.5,
      Self::Rec2020 => ((y + (REC709_ALPHA - 1.0)) / REC709_ALPHA).powf(1.0 / 0.45),
      _ => y,
    }
  }
}

/// Cubic Hermite segment from (0, 0) with slope 32 to (`x1`, `y1`) with
/// slope `slope1`
fn spline_segment(x: f64, x1: f64, y1: f64, slope1: f64) -> f64 {
  let (b, c) = (x / x1, (x1 - x) / x1);
  32.0 * x1 * b * c * c + (y1 * (2.0 - b + c) - slope1 * x1 * c) * b * b
}

/// Function sampled at [`GAMMA_TABLE_SIZE`] intervals of 0.0 ..= 1.0 and
/// interpolated linearly
#[derive(Debug, Clone, PartialEq)]
struct Lookup(Arc<[f32]>);

impl Lookup {
  fn new(function: impl Fn(f64) -> f64) -> Self {
    Self((0..=GAMMA_TABLE_SIZE).map(|i| function(i as f64 / GAMMA_TABLE_SIZE as f64) as f32).collect())
  }

  fn eval(&self, x: f32) -> f32 {
    let scaled = x.clamp(0.0, 1.0) * GAMMA_TABLE_SIZE as f32;
    // Float to integer casts saturate, NaN becomes 0
    let index = (scaled as usize).min(GAMMA_TABLE_SIZE - 1);
    let (y0, y1) = (self.0[index], self.0[index + 1]);
    y0 + (y1 - y0) * (scaled - index as f32)
  }
}

/// Matrix from linear RGB of the ColorPrimaries `code` to linear ProPhoto
/// RGB (D50). The primaries except ProPhoto have a D65 white, which is
/// adapted by Bradford. Returns `Some(None)` for ProPhoto primaries and
/// `None` for an unknown code.
fn primaries_to_prophoto(code: u8) -> Option<Option<[[f32; 3]; 3]>> {
  let to_xyz = match code {
    0 => SRGB_TO_XYZ_D65,
    1 => ADOBERGB_TO_XYZ_D65,
    2 => DISPLAYP3_TO_XYZ_D65,
    3 => REC2020_TO_XYZ_D65,
    4 => return Some(None),
    _ => return None,
  };
  Some(Some(multiply(
    &XYZ_TO_PROFOTORGB_D50,
    &multiply(&bradford_adaption_matrix(&Illuminant::D65, &Illuminant::D50), &to_xyz),
  )))
}

/// One table of RGBTables, prepared for the lookup
#[derive(Debug, Clone, PartialEq)]
pub struct RgbTable3d {
  divisions: usize,
  /// Entries with red in the outer loop, then green and blue, in 0.0 ..= 1.0
  samples: Arc<[[f32; 3]]>,
  /// Encoding and decoding of the gamma of the table, `None` for linear tables
  gamma: Option<(Lookup, Lookup)>,
  /// Matrices from linear ProPhoto RGB to the primaries of the table and
  /// back, `None` for ProPhoto primaries
  matrices: Option<([[f32; 3]; 3], [[f32; 3]; 3])>,
  gamut_extension: bool,
}

impl RgbTable3d {
  /// Prepare `table`, `None` if its parameters or its data are invalid
  pub fn new(table: &RgbTable) -> Option<Self> {
    let divisions = table.divisions as usize;
    let gamma = TableGamma::from_code(table.gamma_encoding);
    let primaries = primaries_to_prophoto(table.color_primaries);
    let (Some(gamma), Some(primaries)) = (gamma, primaries) else {
      log::warn!(
        "RGBTables table with unknown GammaEncoding {} or ColorPrimaries {}, ignoring it",
        table.gamma_encoding,
        table.color_primaries
      );
      return None;
    };
    if !(2..=32).contains(&divisions) || table.data.len() != divisions.pow(3) * 3 || table.gamut_extension > 1 {
      log::warn!(
        "Invalid RGBTables table with {} divisions, {} values and GamutExtension {}, ignoring it",
        divisions,
        table.data.len(),
        table.gamut_extension
      );
      return None;
    }
    let values: Vec<f32> = match &table.data {
      TableData::U8(data) => data.iter().map(|v| *v as f32 / 255.0).collect(),
      TableData::U16(data) => data.iter().map(|v| *v as f32 / 65535.0).collect(),
      TableData::F32(data) => data.iter().map(|v| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) }).collect(),
    };
    Some(Self {
      divisions,
      samples: values.as_chunks::<3>().0.into(),
      gamma: (gamma != TableGamma::Linear).then(|| (Lookup::new(|x| gamma.encode(x)), Lookup::new(|y| gamma.decode(y)))),
      matrices: primaries.map(|decode| (pseudo_inverse(decode), decode)),
      gamut_extension: table.gamut_extension == 1,
    })
  }

  /// Apply the table to linear ProPhoto RGB
  ///
  /// High Dynamic Range values (`hdr`) are encoded by [`encode_overrange`]
  /// for the lookup and are not clipped to 1.0. For a table in ProPhoto
  /// primaries, the part of them above the range of the table is kept.
  pub fn apply(&self, rgb: [f32; 3], hdr: bool) -> [f32; 3] {
    let mapped = match &self.matrices {
      Some((encode, _)) => multiply_vector(encode, &rgb),
      None => rgb,
    };
    let mapped = if hdr { mapped.map(encode_overrange) } else { mapped };
    let clipped = mapped.map(|c| c.clamp(0.0, 1.0));
    let encoded = match &self.gamma {
      Some((encode, _)) => clipped.map(|c| encode.eval(c)),
      None => clipped,
    };
    let looked_up = self.lookup(encoded).map(|c| c.clamp(0.0, 1.0));
    let mut result = match &self.gamma {
      Some((_, decode)) => looked_up.map(|c| decode.eval(c)),
      None => looked_up,
    };
    if self.matrices.is_none() && !hdr {
      return result;
    }
    if self.gamut_extension || self.matrices.is_none() {
      for ((value, mapped), clipped) in result.iter_mut().zip(mapped).zip(clipped) {
        *value += mapped - clipped;
      }
    }
    if hdr {
      result = result.map(decode_overrange);
    }
    if let Some((_, decode)) = &self.matrices {
      result = multiply_vector(decode, &result);
    }
    if hdr { result } else { result.map(|c| c.clamp(0.0, 1.0)) }
  }

  /// Tetrahedral interpolation of the entries at the encoded values `rgb`
  /// (0.0 ..= 1.0)
  fn lookup(&self, rgb: [f32; 3]) -> [f32; 3] {
    let scale = (self.divisions - 1) as f32;
    let scaled = rgb.map(|c| c * scale);
    // Float to integer casts saturate, NaN becomes 0
    let index = scaled.map(|c| (c as usize).min(self.divisions - 2));
    let [fr, fg, fb] = [0, 1, 2].map(|i| scaled[i] - index[i] as f32);
    // Entry strides of red, green and blue
    let (sr, sg, sb) = (self.divisions * self.divisions, self.divisions, 1);
    // The tetrahedron is chosen by the order of the fractions: the corners
    // after the origin of the cube and the fractions, largest first
    let (offset1, offset2, f1, f2, f3) = if fg >= fr {
      if fb >= fg {
        (sb, sg + sb, fb, fg, fr)
      } else if fb >= fr {
        (sg, sg + sb, fg, fb, fr)
      } else {
        (sg, sr + sg, fg, fr, fb)
      }
    } else if fb >= fr {
      (sb, sr + sb, fb, fr, fg)
    } else if fb >= fg {
      (sr, sr + sb, fr, fb, fg)
    } else {
      (sr, sr + sg, fr, fg, fb)
    };
    let origin = index[0] * sr + index[1] * sg + index[2] * sb;
    let corners = [origin, origin + offset1, origin + offset2, origin + sr + sg + sb];
    let weights = [1.0 - f1, f1 - f2, f2 - f3, f3];
    let mut result = [0.0; 3];
    for (corner, weight) in corners.into_iter().zip(weights) {
      for (value, sample) in result.iter_mut().zip(self.samples[corner]) {
        *value += weight * sample;
      }
    }
    result
  }
}

/// Weights of a semantic mask at positions relative to the stage 3 image,
/// which the mask covers
#[derive(Debug, Clone, PartialEq)]
struct MaskSampler {
  width: usize,
  height: usize,
  /// Opacity of the mask pixels, 0.0 ..= 1.0
  alpha: Arc<[f32]>,
  /// Left and top position of a cropped mask (MaskSubArea) in the uncropped
  /// mask and the width and height of the uncropped mask
  area: [f32; 4],
  cropped: bool,
}

impl MaskSampler {
  fn new(mask: &SemanticMask) -> Option<Self> {
    let (width, height) = (mask.image.width, mask.image.height);
    if width == 0 || height == 0 {
      return None;
    }
    // MaskSubArea: top, left, width and height of the uncropped mask. An
    // area that doesn't contain the mask is ignored.
    let sub_area = mask
      .sub_area
      .map(|area| area.map(|v| v as usize))
      .filter(|[top, left, full_width, full_height]| top + height <= *full_height && left + width <= *full_width);
    let area = match sub_area {
      Some([top, left, full_width, full_height]) => [left, top, full_width, full_height].map(|v| v as f32),
      None => [0.0, 0.0, width as f32, height as f32],
    };
    Some(Self {
      width,
      height,
      alpha: mask.image.alpha().into(),
      area,
      cropped: sub_area.is_some(),
    })
  }

  /// Weight at the relative position (`u`, `v`), interpolated bilinearly
  /// between the pixel centers. Edge pixels are repeated, outside of a
  /// cropped mask the weight is 0.0.
  fn weight(&self, u: f32, v: f32) -> f32 {
    let [left, top, full_width, full_height] = self.area;
    let x = u * full_width - left - 0.5;
    let y = v * full_height - top - 0.5;
    let (last_x, last_y) = ((self.width - 1) as f32, (self.height - 1) as f32);
    if self.cropped && (x < -0.5 || y < -0.5 || x > last_x + 0.5 || y > last_y + 0.5) {
      return 0.0;
    }
    let (x, y) = (x.clamp(0.0, last_x), y.clamp(0.0, last_y));
    // Float to integer casts saturate, NaN becomes 0
    let (x0, y0) = (x as usize, y as usize);
    let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let at = |x: usize, y: usize| self.alpha[y * self.width + x];
    let upper = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * fx;
    let lower = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * fx;
    upper + (lower - upper) * fy
  }
}

/// RGBTables of a camera profile with the semantic masks of their tables
#[derive(Debug, Clone, PartialEq)]
pub struct RgbTablesRender {
  /// Tables in the order of the tag with their mask, `None` for the
  /// background table
  tables: Vec<(RgbTable3d, Option<MaskSampler>)>,
  sequential: bool,
}

impl RgbTablesRender {
  /// Tables of `tables` with the masks of `masks` that match their
  /// SemanticName
  ///
  /// A table whose name matches no mask is ignored, like an invalid table and
  /// a second background table. Returns `None` if no table is left.
  pub fn new(tables: &RgbTables, masks: &[SemanticMask]) -> Option<Self> {
    let mut prepared = Vec::new();
    for table in &tables.tables {
      let Some(table_3d) = RgbTable3d::new(table) else {
        continue;
      };
      let mask = match &table.semantic_name {
        Some(name) => match masks.iter().find(|mask| &mask.name == name).and_then(MaskSampler::new) {
          Some(mask) => Some(mask),
          None => {
            log::debug!("RGBTables table for semantic mask '{}' has no mask, ignoring it", name);
            continue;
          }
        },
        None if prepared.iter().any(|(_, mask): &(RgbTable3d, Option<MaskSampler>)| mask.is_none()) => {
          log::warn!("RGBTables has more than one background table, ignoring it");
          continue;
        }
        None => None,
      };
      prepared.push((table_3d, mask));
    }
    if tables.composite_method > COMPOSITE_SEQUENTIAL {
      log::warn!("Unknown RGBTables CompositeMethod {}, using the weighted sum", tables.composite_method);
    }
    prepared.truncate(MAX_TABLES);
    (!prepared.is_empty()).then_some(Self {
      tables: prepared,
      sequential: tables.composite_method == COMPOSITE_SEQUENTIAL,
    })
  }

  /// Apply the tables to linear ProPhoto RGB at the position (`u`, `v`)
  /// relative to the stage 3 image, `hdr` for a High Dynamic Range profile
  ///
  /// Each table has the weight of its mask at the position, the background
  /// table the weight that the masks leave: b = 1 − min(1, Σm). The weighted
  /// sum averages the results of the tables by their weights, the unchanged
  /// color takes the place of a missing background table. The sequential
  /// method blends the result of each table into the color by its weight, in
  /// the order of the tag.
  pub fn apply(&self, rgb: [f32; 3], u: f32, v: f32, hdr: bool) -> [f32; 3] {
    let mut weights = [0.0; MAX_TABLES];
    let mut masked = 0.0;
    for (weight, (_, mask)) in weights.iter_mut().zip(&self.tables) {
      if let Some(mask) = mask {
        *weight = mask.weight(u, v);
        masked += *weight;
      }
    }
    let background = 1.0 - f32::min(1.0, masked);
    for (weight, (_, mask)) in weights.iter_mut().zip(&self.tables) {
      if mask.is_none() {
        *weight = background;
      }
    }
    let applied = self.tables.iter().zip(weights).filter(|(_, weight)| *weight > 0.0);
    if self.sequential {
      return applied.fold(rgb, |color, ((table, _), weight)| {
        let result = table.apply(color, hdr);
        [0, 1, 2].map(|i| color[i] + weight * (result[i] - color[i]))
      });
    }
    let mut sum = [0.0; 3];
    let mut total = 0.0;
    for ((table, _), weight) in applied {
      let result = table.apply(rgb, hdr);
      sum = [0, 1, 2].map(|i| sum[i] + weight * result[i]);
      total += weight;
    }
    if !self.tables.iter().any(|(_, mask)| mask.is_none()) {
      sum = [0, 1, 2].map(|i| sum[i] + background * rgb[i]);
      total += background;
    }
    if total > 0.0 { sum.map(|c| c / total) } else { rgb }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::RawImageData;
  use crate::mask::MaskImage;

  /// Table of `divisions`³ entries in the pixel type of `pixel_type` (0 = u8,
  /// 1 = u16, 2 = f32) from `transform` of the encoded grid values
  fn table(name: Option<&str>, divisions: u8, pixel_type: u8, codes: [u8; 3], transform: impl Fn([f32; 3]) -> [f32; 3]) -> RgbTable {
    let n = divisions as usize;
    let mut values = Vec::new();
    for r in 0..n {
      for g in 0..n {
        for b in 0..n {
          values.extend(transform([r, g, b].map(|i| i as f32 / (n - 1) as f32)));
        }
      }
    }
    let data = match pixel_type {
      0 => TableData::U8(values.iter().map(|v| (v * 255.0).round() as u8).collect()),
      1 => TableData::U16(values.iter().map(|v| (v * 65535.0).round() as u16).collect()),
      _ => TableData::F32(values),
    };
    let [gamma_encoding, color_primaries, gamut_extension] = codes;
    RgbTable {
      semantic_name: name.map(String::from),
      divisions,
      gamma_encoding,
      color_primaries,
      gamut_extension,
      data,
    }
  }

  fn mask(name: &str, width: usize, height: usize, alpha: &[u16]) -> SemanticMask {
    SemanticMask {
      name: name.into(),
      instance_id: None,
      sub_area: None,
      image: MaskImage::new(width, height, 8, RawImageData::Integer(alpha.to_vec())),
    }
  }

  fn assert_rgb_eq(a: [f32; 3], b: [f32; 3], tolerance: f32) {
    for (x, y) in a.iter().zip(b) {
      assert!((x - y).abs() <= tolerance, "{:?} != {:?}", a, b);
    }
  }

  #[test]
  fn gamma_lookups_roundtrip() {
    for code in 0..=4 {
      let gamma = TableGamma::from_code(code).expect("gamma");
      for i in 0..=100 {
        let x = i as f64 / 100.0;
        assert!((gamma.decode(gamma.encode(x)) - x).abs() < 1e-9, "{:?}: {}", gamma, x);
      }
      if let Some((_, x1, y1, _)) = gamma.power_curve() {
        // The toe ends on the power curve
        assert!((gamma.encode(x1) - y1).abs() < 1e-6, "{:?}", gamma);
      }
    }
    assert!(TableGamma::from_code(5).is_none());
  }

  #[test]
  fn identity_tables_keep_colors() {
    // With 18 divisions, the grid values are exact in u8 and u16
    let colors = [[0.4, 0.35, 0.3], [0.2, 0.25, 0.3], [0.8, 0.8, 0.8], [0.05, 0.04, 0.06], [0.0; 3], [1.0; 3]];
    for gamma in 0..=4 {
      for primaries in 0..=4 {
        for pixel_type in 0..=2 {
          let table = RgbTable3d::new(&table(None, 18, pixel_type, [gamma, primaries, 1], |rgb| rgb)).expect("table");
          for rgb in colors {
            assert_rgb_eq(table.apply(rgb, false), rgb, 1e-3);
          }
        }
      }
    }
  }

  #[test]
  fn gamut_extension() {
    // Saturated ProPhoto red, outside of sRGB
    let red = [0.8, 0.1, 0.05];
    let extended = RgbTable3d::new(&table(None, 18, 1, [1, 0, 1], |rgb| rgb)).expect("table");
    assert_rgb_eq(extended.apply(red, false), red, 1e-3);
    let clipped = RgbTable3d::new(&table(None, 18, 1, [1, 0, 0], |rgb| rgb)).expect("table");
    let result = clipped.apply(red, false);
    assert!((result[0] - red[0]).abs() > 0.1, "{:?}", result);
  }

  #[test]
  fn tetrahedral_interpolation() {
    // Swapping the channels is linear, the interpolation is exact
    let swap = RgbTable3d::new(&table(None, 5, 2, [0, 4, 0], |[r, g, b]| [g, b, r])).expect("table");
    for [r, g, b] in [[0.25, 0.5, 1.0], [0.3, 0.7, 0.1], [0.9, 0.2, 0.55], [0.0, 1.0, 0.6]] {
      assert_rgb_eq(swap.apply([r, g, b], false), [g, b, r], 1e-6);
    }
  }

  #[test]
  fn hdr_values_above_white() {
    for primaries in [0, 4] {
      let identity = RgbTable3d::new(&table(None, 18, 2, [0, primaries, 1], |rgb| rgb)).expect("table");
      let rgb = [2.0, 1.0, 0.5];
      assert_rgb_eq(identity.apply(rgb, true), rgb, 1e-3);
    }
  }

  #[test]
  fn invalid_tables() {
    assert!(RgbTable3d::new(&table(None, 3, 1, [5, 0, 1], |rgb| rgb)).is_none());
    assert!(RgbTable3d::new(&table(None, 3, 1, [0, 5, 1], |rgb| rgb)).is_none());
    assert!(RgbTable3d::new(&table(None, 3, 1, [0, 0, 2], |rgb| rgb)).is_none());
    let mut short = table(None, 3, 1, [0, 0, 1], |rgb| rgb);
    short.data = TableData::U16(vec![0; 10]);
    assert!(RgbTable3d::new(&short).is_none());
  }

  #[test]
  fn mask_weights() {
    let sampler = MaskSampler::new(&mask("sky", 4, 2, &[0, 255, 255, 0, 0, 0, 255, 255])).expect("mask");
    // Pixel centers
    assert_eq!(sampler.weight(0.375, 0.25), 1.0);
    assert_eq!(sampler.weight(0.125, 0.75), 0.0);
    // Between the centers of the pixels (0, 0) and (1, 0)
    assert!((sampler.weight(0.25, 0.25) - 0.5).abs() < 1e-6);
    // Between both rows of pixel column 2
    assert!((sampler.weight(0.625, 0.5) - 1.0).abs() < 1e-6);
    // Edge pixels are repeated
    assert_eq!(sampler.weight(0.0, 0.0), 0.0);
    assert_eq!(sampler.weight(1.0, 1.0), 1.0);
    // A cropped mask of 2x1 pixels at left 1 in a 4x1 mask
    let mut cropped = mask("sky", 2, 1, &[255, 128]);
    cropped.sub_area = Some([0, 1, 4, 1]);
    let sampler = MaskSampler::new(&cropped).expect("mask");
    assert_eq!(sampler.weight(0.125, 0.5), 0.0);
    assert_eq!(sampler.weight(0.375, 0.5), 1.0);
    assert!((sampler.weight(0.625, 0.5) - 128.0 / 255.0).abs() < 1e-6);
    assert_eq!(sampler.weight(0.875, 0.5), 0.0);
  }

  /// RGBTables with tables that map all colors to the constant `colors`
  fn constant_tables(names: &[Option<&str>], colors: &[[f32; 3]], composite_method: u32) -> RgbTables {
    RgbTables {
      composite_method,
      tables: names
        .iter()
        .zip(colors)
        .map(|(name, color)| table(*name, 2, 2, [0, 4, 0], |_| *color))
        .collect(),
    }
  }

  #[test]
  fn weighted_sum() {
    let colors = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let src = [0.2, 0.4, 0.6];
    let masks = |a: u16, b: u16| [mask("a", 1, 1, &[a]), mask("b", 1, 1, &[b])];
    let tables = constant_tables(&[Some("a"), Some("b"), None], &colors, 0);
    // The masks weigh 0.6 and 0.2, the background 0.2
    let render = RgbTablesRender::new(&tables, &masks(153, 51)).expect("tables");
    assert_rgb_eq(render.apply(src, 0.5, 0.5, false), [0.6, 0.2, 0.2], 1e-6);
    // Masks above 1 in sum leave no weight for the background
    let render = RgbTablesRender::new(&tables, &masks(255, 255)).expect("tables");
    assert_rgb_eq(render.apply(src, 0.5, 0.5, false), [0.5, 0.5, 0.0], 1e-6);
    // Without a background table, the unchanged color has its weight
    let tables = constant_tables(&[Some("a")], &colors, 0);
    let render = RgbTablesRender::new(&tables, &masks(153, 0)).expect("tables");
    assert_rgb_eq(render.apply(src, 0.5, 0.5, false), [0.68, 0.16, 0.24], 1e-6);
  }

  #[test]
  fn sequential() {
    let colors = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let src = [0.2, 0.4, 0.6];
    let masks = [mask("a", 1, 1, &[153]), mask("b", 1, 1, &[51])];
    let render = RgbTablesRender::new(&constant_tables(&[Some("a"), None, Some("b")], &colors, 1), &masks).expect("tables");
    // In the order of the tag, the background with the weight 0.2
    let blend = |p: [f32; 3], t: [f32; 3], w: f32| [0, 1, 2].map(|i| p[i] + w * (t[i] - p[i]));
    let expected = blend(blend(blend(src, colors[0], 0.6), colors[1], 0.2), colors[2], 0.2);
    assert_rgb_eq(render.apply(src, 0.5, 0.5, false), expected, 1e-6);
  }

  #[test]
  fn tables_without_mask_are_ignored() {
    let colors = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    assert!(RgbTablesRender::new(&constant_tables(&[Some("a")], &colors, 0), &[]).is_none());
    // Only the first background table is used
    let render = RgbTablesRender::new(&constant_tables(&[Some("a"), None, None], &colors, 0), &[]).expect("tables");
    assert_eq!(render.tables.len(), 1);
    assert_rgb_eq(render.apply([0.2; 3], 0.5, 0.5, false), colors[1], 1e-6);
  }
}
