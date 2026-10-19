// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! DNG opcode GainMap (DNG 1.3)

use serde::{Deserialize, Serialize};

use crate::{RawImage, RawImageData, RawlerError, Result, imgop::Rect, pixarray::SubPixel};

use super::{
  OpcodeStage,
  helpers::{AreaSpec, ParamReader, apply_roi_threaded, put_f32, put_f64, put_u32, stage_bounds},
};

#[derive(Debug, Clone, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct GainMap {
  pub version: u32,
  pub flags: u32,
  pub area: AreaSpec,
  pub map_points_v: u32,
  pub map_points_h: u32,

  pub map_spacing_v: f64,
  pub map_spacing_h: f64,
  pub map_origin_v: f64,
  pub map_origin_h: f64,
  pub map_planes: u32,
  pub gain_map: Vec<f32>,
}

impl GainMap {
  pub(super) fn parse(reader: &mut ParamReader, version: u32, flags: u32) -> Result<Self> {
    let area = AreaSpec::parse(reader)?;
    let map_points_v = reader.u32()?;
    let map_points_h = reader.u32()?;
    let map_spacing_v = reader.f64()?;
    let map_spacing_h = reader.f64()?;
    let map_origin_v = reader.f64()?;
    let map_origin_h = reader.f64()?;
    let map_planes = reader.u32()?;

    let count = (map_points_v as usize)
      .checked_mul(map_points_h as usize)
      .and_then(|n| n.checked_mul(map_planes as usize))
      .unwrap_or(usize::MAX);
    reader.check_count(count, 4, "GainMap")?;
    let mut gain_map = Vec::with_capacity(count);
    for _ in 0..count {
      gain_map.push(reader.f32()?);
    }

    let gain_map = Self {
      version,
      flags,
      area,
      map_points_v,
      map_points_h,
      map_spacing_v,
      map_spacing_h,
      map_origin_v,
      map_origin_h,
      map_planes,
      gain_map,
    };
    gain_map.validate()?;
    Ok(gain_map)
  }

  /// Validate the parameters. Spacing and origin
  /// are ignored for a map axis with a single point.
  fn validate(&self) -> Result<()> {
    let invalid = |what: &str| Err(RawlerError::DecoderFailed(format!("DNG opcode GainMap is invalid: {}", what)));
    if self.map_points_v < 1 || self.map_points_h < 1 || self.map_planes < 1 {
      return invalid("map points and planes must be at least 1");
    }
    for (points, spacing, origin) in [
      (self.map_points_v, self.map_spacing_v, self.map_origin_v),
      (self.map_points_h, self.map_spacing_h, self.map_origin_h),
    ] {
      if points > 1 && !(spacing.is_finite() && spacing > 0.0 && origin.is_finite()) {
        return invalid("map spacing must be positive and map origin finite");
      }
    }
    let count = (self.map_points_v as usize)
      .checked_mul(self.map_points_h as usize)
      .and_then(|n| n.checked_mul(self.map_planes as usize));
    if count != Some(self.gain_map.len()) {
      return invalid("number of map gains does not match map size");
    }
    Ok(())
  }

  /// Gain for map point (v, h) and map plane
  fn gain(&self, v: usize, h: usize, plane: usize) -> f32 {
    self.gain_map[(v * self.map_points_h as usize + h) * self.map_planes as usize + plane]
  }

  pub fn apply(&self, rawimage: &mut RawImage, stage: OpcodeStage) -> Result<()> {
    self.validate()?;
    let Some(rect) = self.area.buffer_rect(rawimage, stage) else {
      return Ok(());
    };
    let bounds = stage_bounds(rawimage, stage);
    let (width, cpp) = (rawimage.width, rawimage.cpp);
    match (&mut rawimage.data, stage) {
      // Logical range of floating point data is 0.0 to 1.0
      (RawImageData::Float(samples), _) => {
        self.apply_gain(samples, width, cpp, rect, bounds, &|x: f32, gain| (x * gain).clamp(0.0, 1.0));
      }
      // Logical range of 16 bit data in OpcodeList1 is 0 to 65535
      (RawImageData::Integer(samples), OpcodeStage::List1) => {
        self.apply_gain(samples, width, cpp, rect, bounds, &|x: u16, gain| {
          (x as f32 * gain).round().clamp(0.0, 65535.0) as u16
        });
      }
      (RawImageData::Integer(_), _) => {
        return Err(RawlerError::DecoderFailed(format!(
          "DNG opcode GainMap in {} requires image data mapped to linear reference values",
          stage
        )));
      }
    }
    Ok(())
  }

  /// Multiply the planes of all pixels in `rect` (stepping by the row and column
  /// pitch) by the bilinear interpolated gain, using `mul`. Gain map coordinates
  /// are relative to the size of `bounds`, the image being modified.
  fn apply_gain<T: SubPixel, F>(&self, samples: &mut [T], width: usize, cpp: usize, rect: Rect, bounds: Rect, mul: &F)
  where
    F: Fn(T, f32) -> T + Sync,
  {
    let planes = self.area.plane_range(cpp);
    let col_pitch = self.area.col_pitch as usize;
    // Horizontal map position for each affected column
    let columns: Vec<(usize, usize, f32)> = (rect.x()..rect.x() + rect.width())
      .step_by(col_pitch)
      .map(|col| map_position(col - bounds.p.x, bounds.d.w, self.map_points_h, self.map_spacing_h, self.map_origin_h))
      .collect();
    apply_roi_threaded(samples, width, cpp, rect, self.area.row_pitch as usize, &|line, row| {
      let (v1, v2, fv) = map_position(row - bounds.p.y, bounds.d.h, self.map_points_v, self.map_spacing_v, self.map_origin_v);
      for plane in planes.clone() {
        // If Planes > MapPlanes, the last map plane is used for the remaining planes
        let map_plane = (plane - planes.start).min(self.map_planes as usize - 1);
        // Map row interpolated for this image row
        let gains: Vec<f32> = (0..self.map_points_h as usize)
          .map(|h| lerp(self.gain(v1, h, map_plane), self.gain(v2, h, map_plane), fv))
          .collect();
        for (pixel, &(h1, h2, fh)) in line.chunks_exact_mut(cpp).step_by(col_pitch).zip(&columns) {
          pixel[plane] = mul(pixel[plane], lerp(gains[h1], gains[h2], fh));
        }
      }
    });
  }

  pub(super) fn write_params(&self, buf: &mut Vec<u8>) {
    self.area.write(buf);
    put_u32(buf, self.map_points_v);
    put_u32(buf, self.map_points_h);
    put_f64(buf, self.map_spacing_v);
    put_f64(buf, self.map_spacing_h);
    put_f64(buf, self.map_origin_v);
    put_f64(buf, self.map_origin_h);
    put_u32(buf, self.map_planes);
    for gain in &self.gain_map {
      put_f32(buf, *gain);
    }
  }
}

/// Gain map point indices and interpolation weight for pixel `pos` of an image
/// with `size` pixels along one axis. Pixels are sampled at their centers,
/// positions outside of the map are clamped to its edges.
fn map_position(pos: usize, size: usize, points: u32, spacing: f64, origin: f64) -> (usize, usize, f32) {
  let last = points.saturating_sub(1) as usize;
  if last == 0 {
    return (0, 0, 0.0);
  }
  let index = ((pos as f64 + 0.5) / size as f64 - origin) / spacing;
  if index.is_nan() || index <= 0.0 {
    (0, 0, 0.0)
  } else if index >= last as f64 {
    (last, last, 0.0)
  } else {
    let i = index as usize;
    (i, i + 1, (index - i as f64) as f32)
  }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
  a + (b - a) * t
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{
    dng::opcodes::{Opcode, OpcodeList, testutil::*},
    imgop::{Dim2, Point},
  };

  fn single_gain_map(area: [u32; 8], points: (u32, u32), spacing: (f64, f64), origin: (f64, f64), map_planes: u32, gains: &[f32]) -> Result<OpcodeList> {
    OpcodeList::parse(&opcode_list(&[opcode(9, V1_3, 0, &gain_map(area, points, spacing, origin, map_planes, gains))]))
  }

  /// 2x2 map with points at 0.25 and 0.75 and the expected gains at the pixel centers of a 4x4 image
  const MAP_2X2: [f32; 4] = [1.0, 2.0, 1.5, 1.0];

  const MAP_2X2_GAINS: [[f32; 4]; 4] = [
    [1.0, 1.25, 1.75, 2.0],
    [1.125, 1.28125, 1.59375, 1.75],
    [1.375, 1.34375, 1.28125, 1.25],
    [1.5, 1.375, 1.125, 1.0],
  ];

  #[test]
  fn gain_map_validation() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let area = [0, 0, 4, 4, 0, 1, 1, 1];
    // Zero points, zero map planes, zero spacing for multiple points
    for params in [
      gain_map(area, (0, 1), (1.0, 1.0), (0.0, 0.0), 1, &[]),
      gain_map(area, (1, 1), (1.0, 1.0), (0.0, 0.0), 0, &[]),
      gain_map(area, (2, 1), (0.0, 1.0), (0.0, 0.0), 1, &[1.0, 1.0]),
    ] {
      assert!(OpcodeList::parse(&opcode_list(&[opcode(9, V1_3, 0, &params)])).is_err());
      let list = OpcodeList::parse(&opcode_list(&[opcode(9, V1_3, 1, &params)]))?;
      assert!(matches!(&list.opcodes()[0], Opcode::Unknown(op) if op.id == 9));
    }
    // Spacing and origin are ignored for a single map point
    let list = single_gain_map(area, (1, 1), (0.0, 0.0), (f64::NAN, 0.0), 1, &[1.0])?;
    assert!(matches!(&list.opcodes()[0], Opcode::GainMap(_)));
    Ok(())
  }

  #[test]
  fn gain_map_row_and_col_pitch() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut image = float_image(4, 4, 1, 0.25);
    single_gain_map([1, 0, 4, 4, 0, 1, 2, 2], (1, 1), (1.0, 1.0), (0.0, 0.0), 1, &[2.0])?.apply(&mut image, OpcodeStage::List2)?;
    for (i, value) in floats(&image).iter().enumerate() {
      let (row, col) = (i / 4, i % 4);
      let expected = if row % 2 == 1 && col % 2 == 0 { 0.5 } else { 0.25 };
      assert_eq!(*value, expected, "row {}, col {}", row, col);
    }
    Ok(())
  }

  #[test]
  fn gain_map_bilinear_interpolation() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut image = float_image(4, 4, 1, 0.5);
    single_gain_map([0, 0, 4, 4, 0, 1, 1, 1], (2, 2), (0.5, 0.5), (0.25, 0.25), 1, &MAP_2X2)?.apply(&mut image, OpcodeStage::List2)?;
    for (i, value) in floats(&image).iter().enumerate() {
      assert_eq!(*value, 0.5 * MAP_2X2_GAINS[i / 4][i % 4], "row {}, col {}", i / 4, i % 4);
    }
    Ok(())
  }

  #[test]
  fn gain_map_stage2_is_relative_to_active_area() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Map coordinates are relative to the active area, an empty area covers all of it
    let mut image = float_image(6, 6, 1, 0.5);
    image.active_area = Some(Rect::new(Point::new(2, 1), Dim2::new(4, 4)));
    single_gain_map([0, 0, 0, 0, 0, 1, 1, 1], (2, 2), (0.5, 0.5), (0.25, 0.25), 1, &MAP_2X2)?.apply(&mut image, OpcodeStage::List2)?;
    for (i, value) in floats(&image).iter().enumerate() {
      let (row, col) = (i / 6, i % 6);
      let expected = if (1..5).contains(&row) && (2..6).contains(&col) {
        0.5 * MAP_2X2_GAINS[row - 1][col - 2]
      } else {
        0.5
      };
      assert_eq!(*value, expected, "row {}, col {}", row, col);
    }
    Ok(())
  }

  #[test]
  fn gain_map_planes() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // A single map plane is used for all planes
    let mut image = float_image(1, 1, 3, 0.25);
    single_gain_map([0, 0, 1, 1, 0, 3, 1, 1], (1, 1), (1.0, 1.0), (0.0, 0.0), 1, &[2.0])?.apply(&mut image, OpcodeStage::List2)?;
    assert_eq!(floats(&image), [0.5, 0.5, 0.5]);
    // The last map plane is used for the remaining planes
    let mut image = float_image(1, 1, 3, 0.25);
    single_gain_map([0, 0, 1, 1, 0, 3, 1, 1], (1, 1), (1.0, 1.0), (0.0, 0.0), 2, &[2.0, 3.0])?.apply(&mut image, OpcodeStage::List2)?;
    assert_eq!(floats(&image), [0.5, 0.75, 0.75]);
    // Map planes start at the first modified plane
    let mut image = float_image(1, 1, 3, 0.25);
    single_gain_map([0, 0, 1, 1, 1, 2, 1, 1], (1, 1), (1.0, 1.0), (0.0, 0.0), 2, &[2.0, 3.0])?.apply(&mut image, OpcodeStage::List2)?;
    assert_eq!(floats(&image), [0.25, 0.5, 0.75]);
    Ok(())
  }

  #[test]
  fn gain_map_clipping_and_integer_data() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let list = single_gain_map([0, 0, 1, 2, 0, 1, 1, 1], (1, 1), (1.0, 1.0), (0.0, 0.0), 1, &[1.5])?;
    let mut image = float_image(2, 1, 1, 0.75);
    list.apply(&mut image, OpcodeStage::List2)?;
    assert_eq!(floats(&image), [1.0, 1.0]);
    image.data = RawImageData::Integer(vec![1001, 50000]);
    list.apply(&mut image, OpcodeStage::List1)?;
    assert!(matches!(&image.data, RawImageData::Integer(samples) if samples == &[1502, 65535]));
    // Integer data is not mapped to linear reference values
    assert!(list.apply(&mut image, OpcodeStage::List2).is_err());
    Ok(())
  }
}
