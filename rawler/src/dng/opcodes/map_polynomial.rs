// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! DNG opcode MapPolynomial (DNG 1.3)

use serde::{Deserialize, Serialize};

use crate::{RawlerError, Result};

use super::{
  OpcodeData, OpcodeImage, OpcodeStage,
  helpers::{AreaSpec, ParamReader, map_area, put_f64, put_u32},
};

/// Maximum allowed degree for MapPolynomial
const MAP_POLYNOMIAL_MAX_DEGREE: usize = 8;

#[derive(Debug, Clone, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct MapPolynomial {
  pub version: u32,
  pub flags: u32,
  pub area: AreaSpec,
  /// Coefficients in increasing order, starting with the constant term
  pub coeffs: Vec<f64>,
}

impl MapPolynomial {
  pub(super) fn parse(reader: &mut ParamReader, version: u32, flags: u32) -> Result<Self> {
    let area = AreaSpec::parse(reader)?;
    let degree = reader.u32()? as usize;
    if degree > MAP_POLYNOMIAL_MAX_DEGREE {
      return Err(RawlerError::DecoderFailed(format!(
        "DNG opcode MapPolynomial: degree {} exceeds maximum of {}",
        degree, MAP_POLYNOMIAL_MAX_DEGREE
      )));
    }
    let mut coeffs = Vec::with_capacity(degree + 1);
    for _ in 0..=degree {
      coeffs.push(reader.f64()?);
    }

    Ok(Self { version, flags, area, coeffs })
  }

  pub(super) fn write_params(&self, buf: &mut Vec<u8>) -> Result<()> {
    if self.coeffs.is_empty() || self.coeffs.len() > MAP_POLYNOMIAL_MAX_DEGREE + 1 {
      return Err(RawlerError::DecoderFailed(format!(
        "DNG opcode MapPolynomial: invalid number of coefficients: {}",
        self.coeffs.len()
      )));
    }
    self.area.write(buf);
    put_u32(buf, (self.coeffs.len() - 1) as u32);
    for coeff in &self.coeffs {
      put_f64(buf, *coeff);
    }
    Ok(())
  }

  /// Evaluate the polynomial (Horner's method)
  pub fn evaluate(&self, x: f64) -> f64 {
    self.coeffs.iter().rev().fold(0.0, |acc, coeff| acc * x + coeff)
  }

  pub fn apply(&self, image: &mut OpcodeImage<'_>, stage: OpcodeStage) -> Result<()> {
    let Some(rect) = self.area.buffer_rect(image, stage) else {
      return Ok(());
    };
    let (width, cpp) = (image.width(), image.channels());
    match (&mut image.data, stage) {
      // Logical range of floating point data is 0.0 to 1.0
      (OpcodeData::Float(view), _) => {
        let samples = view.as_mut_slice();
        map_area(samples, width, cpp, rect, &self.area, &|x: f32| self.evaluate(x as f64).clamp(0.0, 1.0) as f32);
      }
      // Logical range of 16 bit data in OpcodeList1 is 0 to 65535, the polynomial
      // maps 16 bit sample values.
      (OpcodeData::Integer(view), OpcodeStage::List1) => {
        let samples = view.as_mut_slice();
        map_area(samples, width, cpp, rect, &self.area, &|x: u16| {
          self.evaluate(x as f64).round().clamp(0.0, 65535.0) as u16
        });
      }
      (OpcodeData::Integer(_), _) => {
        return Err(RawlerError::DecoderFailed(format!(
          "DNG opcode MapPolynomial in {} requires image data mapped to linear reference values",
          stage
        )));
      }
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{
    RawImageData,
    dng::opcodes::{OpcodeList, testutil::*},
    imgop::{Dim2, Point, Rect},
  };

  fn single_map_polynomial(area: [u32; 8], coeffs: &[f64]) -> Result<OpcodeList> {
    OpcodeList::parse(&opcode_list(&[opcode(8, V1_3, 0, &map_polynomial(area, coeffs))]))
  }

  #[test]
  fn map_polynomial_row_and_col_pitch() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut image = float_image(5, 5, 1, 0.25);
    single_map_polynomial([1, 1, 5, 5, 0, 1, 2, 2], &[0.0, 2.0])?.apply(&mut image, OpcodeStage::List1)?;
    for (i, value) in floats(&image).iter().enumerate() {
      let (row, col) = (i / 5, i % 5);
      let expected = if row % 2 == 1 && col % 2 == 1 { 0.5 } else { 0.25 };
      assert_eq!(*value, expected, "row {}, col {}", row, col);
    }
    Ok(())
  }

  #[test]
  fn map_polynomial_stage2_is_relative_to_active_area() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut image = float_image(6, 6, 1, 0.25);
    image.active_area = Some(Rect::new(Point::new(2, 1), Dim2::new(4, 4)));
    let list = single_map_polynomial([0, 0, 1, 1, 0, 1, 1, 1], &[0.5])?;
    list.apply(&mut image, OpcodeStage::List2)?;
    assert_eq!(floats(&image)[6 + 2], 0.5);
    assert_eq!(floats(&image).iter().filter(|v| **v == 0.5).count(), 1);
    // OpcodeList1 operates on the full raw image
    list.apply(&mut image, OpcodeStage::List1)?;
    assert_eq!(floats(&image)[0], 0.5);
    Ok(())
  }

  #[test]
  fn map_polynomial_area_and_planes_are_clipped() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Area exceeds image, planes exceed cpp, result is clipped to 1.0
    let mut image = float_image(3, 2, 3, 0.75);
    single_map_polynomial([0, 0, 100, 100, 1, 5, 1, 1], &[0.0, 2.0])?.apply(&mut image, OpcodeStage::List2)?;
    for pixel in floats(&image).as_chunks::<3>().0 {
      assert_eq!(*pixel, [0.75, 1.0, 1.0]);
    }
    // Area outside of image
    let mut image = float_image(3, 2, 1, 0.75);
    single_map_polynomial([10, 10, 20, 20, 0, 1, 1, 1], &[0.0])?.apply(&mut image, OpcodeStage::List2)?;
    assert!(floats(&image).iter().all(|v| *v == 0.75));
    // Empty area covers the whole image
    single_map_polynomial([0, 0, 0, 0, 0, 1, 1, 1], &[-1.0])?.apply(&mut image, OpcodeStage::List2)?;
    assert!(floats(&image).iter().all(|v| *v == 0.0));
    Ok(())
  }

  #[test]
  fn map_polynomial_integer_data() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut image = float_image(2, 1, 1, 0.0);
    image.data = RawImageData::Integer(vec![65535, 1000]);
    let list = single_map_polynomial([0, 0, 1, 2, 0, 1, 1, 1], &[0.0, 0.5])?;
    list.apply(&mut image, OpcodeStage::List1)?;
    assert!(matches!(&image.data, RawImageData::Integer(samples) if samples == &[32768, 500]));
    // Integer data is not mapped to linear reference values
    assert!(list.apply(&mut image, OpcodeStage::List2).is_err());
    // The polynomial maps 16 bit sample values, results are rounded and clipped
    let samples = vec![0, 100, 1000, 30000, 65535];
    for (coeffs, expected) in [
      (vec![100.0, 1.0], [100, 200, 1100, 30100, 65535]),
      (vec![-200.0, 1.1, 1e-6], [0, 0, 901, 33700, 65535]),
    ] {
      let mut image = float_image(5, 1, 1, 0.0);
      image.data = RawImageData::Integer(samples.clone());
      single_map_polynomial([0, 0, 1, 5, 0, 1, 1, 1], &coeffs)?.apply(&mut image, OpcodeStage::List1)?;
      assert_eq!(integers(&image), expected, "coefficients {:?}", coeffs);
    }
    Ok(())
  }
}
