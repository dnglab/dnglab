// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! DNG opcode FixVignetteRadial (DNG 1.3)

use serde::{Deserialize, Serialize};

use crate::{RawImage, RawImageData, RawlerError, Result, imgop::Rect, pixarray::SubPixel};

use super::{
  OpcodeStage,
  helpers::{ParamReader, apply_roi_threaded, pixel_aspect_ratio, put_f64, stage_bounds},
};

/// Radially symmetric gain to correct vignetting
#[derive(Debug, Clone, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct FixVignetteRadial {
  pub version: u32,
  pub flags: u32,
  /// Gain coefficients k0 to k4 for r^2, r^4, r^6, r^8 and r^10
  pub k: [f64; 5],
  pub c_x: f64,
  pub c_y: f64,
}

impl FixVignetteRadial {
  pub(super) fn parse(reader: &mut ParamReader, version: u32, flags: u32) -> Result<Self> {
    let mut k = [0.0; 5];
    for coeff in k.iter_mut() {
      *coeff = reader.f64()?;
    }
    let vignette = Self {
      version,
      flags,
      k,
      c_x: reader.f64()?,
      c_y: reader.f64()?,
    };
    vignette.validate()?;
    Ok(vignette)
  }

  pub(super) fn write_params(&self, buf: &mut Vec<u8>) {
    for value in self.k.iter().chain([&self.c_x, &self.c_y]) {
      put_f64(buf, *value);
    }
  }

  /// Validate the parameters
  fn validate(&self) -> Result<()> {
    let invalid = |what: &str| Err(RawlerError::DecoderFailed(format!("DNG opcode FixVignetteRadial is invalid: {}", what)));
    if !self.k.iter().all(|k| k.is_finite()) {
      return invalid("coefficients must be finite");
    }
    if !((0.0..=1.0).contains(&self.c_x) && (0.0..=1.0).contains(&self.c_y)) {
      return invalid("optical center must be in range 0.0 to 1.0");
    }
    Ok(())
  }

  /// Gain for the squared normalized radius `rr`
  fn gain(&self, rr: f64) -> f64 {
    1.0 + self.k.iter().rev().fold(0.0, |acc, k| rr * (k + acc))
  }

  pub fn apply(&self, rawimage: &mut RawImage, stage: OpcodeStage) -> Result<()> {
    self.validate()?;
    if self.k.iter().all(|k| *k == 0.0) {
      return Ok(());
    }
    let bounds = stage_bounds(rawimage, stage);
    if bounds.width() == 0 || bounds.height() == 0 {
      return Ok(());
    }
    // Optical center and distance to the farthest image corner, relative to the
    // outer image edges and for square pixels
    let (w, h) = (bounds.width() as f64, bounds.height() as f64);
    let (c_x, c_y) = (self.c_x * w, self.c_y * h);
    let scale_v = 1.0 / pixel_aspect_ratio(rawimage);
    let radius = (c_y.max(h - c_y) * scale_v).hypot(c_x.max(w - c_x));
    let (width, cpp) = (rawimage.width, rawimage.cpp);
    match (&mut rawimage.data, stage) {
      // Logical range of floating point data is 0.0 to 1.0
      (RawImageData::Float(samples), _) => {
        self.apply_gain(samples, width, cpp, bounds, (c_x, c_y), scale_v, radius, &|v: f32, gain| {
          (v * gain).clamp(0.0, 1.0)
        });
      }
      // Logical range of 16 bit data in OpcodeList1 is 0 to 65535
      (RawImageData::Integer(samples), OpcodeStage::List1) => {
        self.apply_gain(samples, width, cpp, bounds, (c_x, c_y), scale_v, radius, &|v: u16, gain| {
          ((v as f32 / 65535.0 * gain).clamp(0.0, 1.0) * 65535.0).round() as u16
        });
      }
      (RawImageData::Integer(_), _) => {
        return Err(RawlerError::DecoderFailed(format!(
          "DNG opcode FixVignetteRadial in {} requires image data mapped to linear reference values",
          stage
        )));
      }
    }
    Ok(())
  }

  /// Multiply all planes of the pixels in `bounds` by the gain for their pixel
  /// centers, using `mul`
  #[allow(clippy::too_many_arguments)]
  fn apply_gain<T: SubPixel, F>(&self, samples: &mut [T], width: usize, cpp: usize, bounds: Rect, center: (f64, f64), scale_v: f64, radius: f64, mul: &F)
  where
    F: Fn(T, f32) -> T + Sync,
  {
    apply_roi_threaded(samples, width, cpp, bounds, 1, &|line, row| {
      let dy = ((row - bounds.y()) as f64 + 0.5 - center.1) * scale_v / radius;
      for (x, pixel) in line.chunks_exact_mut(cpp).enumerate() {
        let dx = (x as f64 + 0.5 - center.0) / radius;
        let gain = self.gain((dx * dx + dy * dy).min(1.0)) as f32;
        for value in pixel.iter_mut() {
          *value = mul(*value, gain);
        }
      }
    });
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{
    dng::opcodes::{Opcode, OpcodeList, testutil::*},
    imgop::{Dim2, Point},
  };

  fn fix_vignette_radial(k: [f64; 5], center: (f64, f64)) -> Vec<u8> {
    let mut buf = Vec::new();
    k.iter().chain([center.0, center.1].iter()).for_each(|v| put_f64(&mut buf, *v));
    buf
  }

  fn single_vignette(k: [f64; 5], center: (f64, f64)) -> Result<OpcodeList> {
    OpcodeList::parse(&opcode_list(&[opcode(3, V1_3, 0, &fix_vignette_radial(k, center))]))
  }

  const VIGNETTE_K: [f64; 5] = [0.4, 0.2, 0.0, 0.0, 0.0];

  #[test]
  fn fix_vignette_radial_parse() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let data = opcode_list(&[opcode(3, V1_3, 0, &fix_vignette_radial([0.1, -0.2, 0.3, -0.4, 0.5], (0.4, 0.6)))]);
    let list = OpcodeList::parse(&data)?;
    assert!(matches!(&list.opcodes()[0], Opcode::FixVignetteRadial(op) if op.k == [0.1, -0.2, 0.3, -0.4, 0.5] && (op.c_x, op.c_y) == (0.4, 0.6)));
    assert_eq!(list.to_bytes()?, data);
    // NaN coefficient, optical center out of range
    for params in [
      fix_vignette_radial([f64::NAN, 0.0, 0.0, 0.0, 0.0], (0.5, 0.5)),
      fix_vignette_radial(VIGNETTE_K, (1.5, 0.5)),
    ] {
      assert!(OpcodeList::parse(&opcode_list(&[opcode(3, V1_3, 0, &params)])).is_err());
      let list = OpcodeList::parse(&opcode_list(&[opcode(3, V1_3, 1, &params)]))?;
      assert!(matches!(&list.opcodes()[0], Opcode::Unknown(op) if op.id == 3));
    }
    // Parameter area must be 56 bytes
    let mut params = fix_vignette_radial(VIGNETTE_K, (0.5, 0.5));
    params.push(0);
    assert!(OpcodeList::parse(&opcode_list(&[opcode(3, V1_3, 0, &params)])).is_err());
    Ok(())
  }

  #[test]
  fn fix_vignette_radial_gain() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Zero coefficients don't change the image
    let mut image = ramp_image(4, 2, 1, 0.1);
    let original = floats(&image).to_vec();
    single_vignette([0.0; 5], (0.5, 0.5))?.apply(&mut image, OpcodeStage::List3)?;
    assert_eq!(floats(&image), original);
    // 4x2 image, radius sqrt(5): r^2 is 0.5 for pixel (0, 0) and 0.1 for pixel (1, 0), the same gain for all planes
    let mut image = float_image(4, 2, 3, 0.4);
    single_vignette(VIGNETTE_K, (0.5, 0.5))?.apply(&mut image, OpcodeStage::List3)?;
    for plane in 0..3 {
      assert_close(floats(&image)[plane], 0.4 * 1.25);
      assert_close(floats(&image)[3 + plane], 0.4 * 1.042);
      assert_close(floats(&image)[(4 + 3) * 3 + plane], 0.4 * 1.25);
    }
    // Clipped to 1.0
    let mut image = float_image(4, 2, 1, 0.9);
    single_vignette(VIGNETTE_K, (0.5, 0.5))?.apply(&mut image, OpcodeStage::List3)?;
    assert_eq!(floats(&image)[0], 1.0);
    Ok(())
  }

  #[test]
  fn fix_vignette_radial_geometry() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let k = [0.4, 0.2, 0.1, 0.05, 0.02];
    // Pixel aspect ratio 2
    let mut image = float_image(8, 6, 1, 0.5);
    image.camera.default_scale.0 = [[2, 1], [1, 1]];
    single_vignette(k, (0.5, 0.5))?.apply(&mut image, OpcodeStage::List3)?;
    assert_close(floats(&image)[1], 0.5 * 1.2176951688122184);
    // Optical center at the top-left corner, radius 10
    let mut image = float_image(8, 6, 1, 0.5);
    single_vignette(k, (0.0, 0.0))?.apply(&mut image, OpcodeStage::List3)?;
    assert_close(floats(&image)[5 * 8 + 7], 0.5 * 1.5980437382870625);
    Ok(())
  }

  #[test]
  fn fix_vignette_radial_stage_and_data() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let list = single_vignette(VIGNETTE_K, (0.5, 0.5))?;
    // OpcodeList2 is relative to the ActiveArea
    let mut image = float_image(10, 8, 1, 0.4);
    image.active_area = Some(Rect::new(Point::new(2, 1), Dim2::new(4, 2)));
    list.apply(&mut image, OpcodeStage::List2)?;
    assert_close(floats(&image)[10 + 2], 0.4 * 1.25);
    assert_close(floats(&image)[10 + 3], 0.4 * 1.042);
    assert_eq!(floats(&image).iter().filter(|v| **v == 0.4).count(), 80 - 8);
    // 16 bit integer data in OpcodeList1
    let mut image = float_image(4, 2, 1, 0.0);
    image.data = RawImageData::Integer(vec![20000; 8]);
    list.apply(&mut image, OpcodeStage::List1)?;
    assert_eq!(integers(&image)[0], 25000);
    // Integer data is not mapped to linear reference values
    assert!(list.apply(&mut image, OpcodeStage::List2).is_err());
    Ok(())
  }
}
