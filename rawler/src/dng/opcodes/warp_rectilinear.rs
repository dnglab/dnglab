// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! DNG opcode WarpRectilinear (DNG 1.3)

use serde::{Deserialize, Serialize};

use crate::{RawImage, RawlerError, Result};

use super::{
  OpcodeStage,
  helpers::{ParamReader, RadialModel, WARP_MAX_PLANES, WARP_RADIAL_TERMS, WarpPlane, apply_warp, put_f64, put_u32},
};

#[derive(Debug, Clone, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct WarpRectilinear {
  pub version: u32,
  pub flags: u32,
  pub coeffs: Vec<WarpRectilinearCoeffs>,
  pub c_x: f64,
  pub c_y: f64,
}

impl WarpRectilinear {
  pub(super) fn parse(reader: &mut ParamReader, version: u32, flags: u32) -> Result<Self> {
    let n = reader.u32()? as usize;
    reader.check_count(n, 6 * 8, "WarpRectilinear")?;
    let mut coeffs = Vec::with_capacity(n);
    for _ in 0..n {
      let k_r0 = reader.f64()?;
      let k_r1 = reader.f64()?;
      let k_r2 = reader.f64()?;
      let k_r3 = reader.f64()?;
      let k_t0 = reader.f64()?;
      let k_t1 = reader.f64()?;
      coeffs.push(WarpRectilinearCoeffs {
        k_r0,
        k_r1,
        k_r2,
        k_r3,
        k_t0,
        k_t1,
      });
    }
    let c_x = reader.f64()?;
    let c_y = reader.f64()?;

    let warp = Self {
      version,
      flags,
      coeffs,
      c_x,
      c_y,
    };
    warp.validate()?;
    Ok(warp)
  }

  pub(super) fn write_params(&self, buf: &mut Vec<u8>) {
    put_u32(buf, self.coeffs.len() as u32);
    for c in &self.coeffs {
      for value in c.values() {
        put_f64(buf, value);
      }
    }
    put_f64(buf, self.c_x);
    put_f64(buf, self.c_y);
  }

  /// Validate the parameters
  fn validate(&self) -> Result<()> {
    let invalid = |what: &str| Err(RawlerError::DecoderFailed(format!("DNG opcode WarpRectilinear is invalid: {}", what)));
    if self.coeffs.is_empty() || self.coeffs.len() > WARP_MAX_PLANES {
      return invalid("number of coefficient sets must be 1 to 4");
    }
    if !self.coeffs.iter().all(|c| c.values().iter().all(|v| v.is_finite())) {
      return invalid("coefficients must be finite");
    }
    if !((0.0..=1.0).contains(&self.c_x) && (0.0..=1.0).contains(&self.c_y)) {
      return invalid("optical center must be in range 0.0 to 1.0");
    }
    Ok(())
  }

  pub fn apply(&self, rawimage: &mut RawImage, stage: OpcodeStage) -> Result<()> {
    self.validate()?;
    apply_warp("WarpRectilinear", &self.planes(), (self.c_x, self.c_y), rawimage, stage)
  }

  /// Coefficient sets in the radial model of WarpRectilinear2
  fn planes(&self) -> Vec<WarpPlane> {
    self
      .coeffs
      .iter()
      .map(|c| {
        let mut radial = [0.0; WARP_RADIAL_TERMS];
        radial[0] = c.k_r0;
        radial[2] = c.k_r1;
        radial[4] = c.k_r2;
        radial[6] = c.k_r3;
        WarpPlane {
          radial: RadialModel::Polynomial {
            coeffs: radial,
            valid_radius: [0.0, 1.0],
            reciprocal: false,
          },
          tangential: [c.k_t0, c.k_t1],
        }
      })
      .collect()
  }
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct WarpRectilinearCoeffs {
  pub k_r0: f64,
  pub k_r1: f64,
  pub k_r2: f64,
  pub k_r3: f64,
  pub k_t0: f64,
  pub k_t1: f64,
}

impl WarpRectilinearCoeffs {
  fn values(&self) -> [f64; 6] {
    [self.k_r0, self.k_r1, self.k_r2, self.k_r3, self.k_t0, self.k_t1]
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{
    RawImageData,
    dng::opcodes::{Opcode, OpcodeList, helpers::WarpGeometry, testutil::*},
    imgop::{Dim2, Point, Rect},
  };

  fn first_warp(list: &OpcodeList) -> &WarpRectilinear {
    match &list.opcodes()[0] {
      Opcode::WarpRectilinear(warp) => warp,
      _ => panic!("expected WarpRectilinear"),
    }
  }

  /// Zoom by 2 around the image center, all source positions are on full or half pixels
  const ZOOM: [f64; 6] = [0.5, 0.0, 0.0, 0.0, 0.0, 0.0];

  const IDENTITY: [f64; 6] = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0];

  #[test]
  fn warp_rectilinear_parse() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Parameters of a Google Pixel 8 Pro
    let params = warp_rectilinear(
      &[[0.9897059202194214, 0.02857298031449318, -0.05583058297634125, 0.03774712607264519, 0.0, 0.0]],
      (0.5008903952205882, 0.4987386067708333),
    );
    let data = opcode_list(&[opcode(1, V1_3, 1, &params)]);
    let list = OpcodeList::parse(&data)?;
    assert_eq!(first_warp(&list).coeffs.len(), 1);
    assert_eq!(list.to_bytes()?, data);
    // No or too many coefficient sets, optical center out of range, NaN coefficient
    for params in [
      warp_rectilinear(&[], (0.5, 0.5)),
      warp_rectilinear(&[IDENTITY; 5], (0.5, 0.5)),
      warp_rectilinear(&[IDENTITY], (1.5, 0.5)),
      warp_rectilinear(&[[f64::NAN, 0.0, 0.0, 0.0, 0.0, 0.0]], (0.5, 0.5)),
    ] {
      assert!(OpcodeList::parse(&opcode_list(&[opcode(1, V1_3, 0, &params)])).is_err());
      let list = OpcodeList::parse(&opcode_list(&[opcode(1, V1_3, 1, &params)]))?;
      assert!(matches!(&list.opcodes()[0], Opcode::Unknown(op) if op.id == 1));
    }
    Ok(())
  }

  #[test]
  fn warp_rectilinear_geometry() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let list = single_warp(&[[1.0, 0.1, 0.0, 0.0, 0.01, 0.02]], (0.5, 0.5))?;
    let warp = first_warp(&list);
    let geometry = WarpGeometry::new((warp.c_x, warp.c_y), Dim2::new(8, 6), 1.0);
    assert_eq!((geometry.center, geometry.radius), ((4.0, 3.0), 5.0));
    let (x, y) = geometry.source(&warp.planes()[0], 0.0, 0.0);
    assert!((x + 0.124).abs() < 1e-12 && (y + 0.118).abs() < 1e-12, "{} {}", x, y);
    // Optical center at the top-right corner of the image
    let list = single_warp(&[[1.0, 0.1, 0.0, 0.0, 0.0, 0.0]], (1.0, 0.0))?;
    let warp = first_warp(&list);
    let geometry = WarpGeometry::new((warp.c_x, warp.c_y), Dim2::new(8, 6), 1.0);
    assert_eq!((geometry.center, geometry.radius), ((8.0, 0.0), 10.0));
    let (x, y) = geometry.source(&warp.planes()[0], 0.0, 6.0);
    assert!((x + 0.8).abs() < 1e-12 && (y - 6.6).abs() < 1e-12, "{} {}", x, y);
    // Non-square pixels
    let list = single_warp(&[[1.0, 0.0, 0.0, 0.0, 0.01, 0.02]], (0.5, 0.5))?;
    let warp = first_warp(&list);
    let geometry = WarpGeometry::new((warp.c_x, warp.c_y), Dim2::new(8, 6), 2.0);
    assert!((geometry.radius - 4.272001872658765).abs() < 1e-12);
    let (x, y) = geometry.source(&warp.planes()[0], 1.0, 0.0);
    assert!((x - 1.1580055487147765).abs() < 1e-12 && (y - 0.15800554871477646).abs() < 1e-12, "{} {}", x, y);
    Ok(())
  }

  #[test]
  fn warp_rectilinear_resampling() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Full and half pixel source positions resample a ramp exactly
    let mut image = ramp_image(8, 6, 1, 1.0 / 8.0);
    single_warp(&[ZOOM], (0.5, 0.5))?.apply(&mut image, OpcodeStage::List3)?;
    for (i, value) in floats(&image).iter().enumerate() {
      let x = (i % 8) as f32;
      assert_eq!(*value, (2.0 + x / 2.0) / 8.0, "pixel {}", i);
    }
    // Constant image stays constant
    let mut image = float_image(8, 6, 1, 0.3);
    single_warp(&[[0.9, 0.2, -0.1, 0.05, 0.01, -0.02]], (0.4, 0.6))?.apply(&mut image, OpcodeStage::List3)?;
    assert!(floats(&image).iter().all(|v| (v - 0.3).abs() < 1e-6));
    // Overshoot at a step edge is clipped
    let mut image = float_image(8, 6, 1, 0.0);
    image.data = RawImageData::Float((0..48).map(|i| if i % 8 < 4 { 0.0 } else { 1.0 }).collect());
    single_warp(&[ZOOM], (0.5, 0.5))?.apply(&mut image, OpcodeStage::List3)?;
    assert!(floats(&image).iter().all(|v| (0.0..=1.0).contains(v)));
    Ok(())
  }

  #[test]
  fn warp_rectilinear_planes() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Identity warp is skipped
    let mut image = ramp_image(8, 6, 3, 1.0 / 8.0);
    let original = floats(&image).to_vec();
    single_warp(&[IDENTITY], (0.3, 0.7))?.apply(&mut image, OpcodeStage::List3)?;
    assert_eq!(floats(&image), original);
    // One coefficient set per plane
    single_warp(&[IDENTITY, ZOOM, IDENTITY], (0.5, 0.5))?.apply(&mut image, OpcodeStage::List3)?;
    for (i, pixel) in floats(&image).as_chunks::<3>().0.iter().enumerate() {
      let x = (i % 8) as f32;
      assert!((pixel[0] - x / 8.0).abs() < 1e-6 && (pixel[2] - x / 8.0).abs() < 1e-6, "pixel {}", i);
      assert_eq!(pixel[1], (2.0 + x / 2.0) / 8.0, "pixel {}", i);
    }
    // Number of coefficient sets must be 1 or the number of planes
    assert!(single_warp(&[ZOOM, ZOOM], (0.5, 0.5))?.apply(&mut image, OpcodeStage::List3).is_err());
    Ok(())
  }

  #[test]
  fn warp_rectilinear_active_area_and_integer_data() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // OpcodeList2 warps the ActiveArea
    let mut image = ramp_image(10, 8, 1, 1.0 / 16.0);
    image.active_area = Some(Rect::new(Point::new(2, 1), Dim2::new(8, 6)));
    let original = floats(&image).to_vec();
    single_warp(&[ZOOM], (0.5, 0.5))?.apply(&mut image, OpcodeStage::List2)?;
    for (i, value) in floats(&image).iter().enumerate() {
      let (row, col) = (i / 10, i % 10);
      let expected = if (1..7).contains(&row) && col >= 2 {
        (4.0 + (col - 2) as f32 / 2.0) / 16.0
      } else {
        original[i]
      };
      assert_eq!(*value, expected, "row {}, col {}", row, col);
    }
    // 16 bit integer data in OpcodeList1
    let list = single_warp(&[ZOOM], (0.5, 0.5))?;
    let mut image = float_image(8, 6, 1, 0.0);
    image.data = RawImageData::Integer((0..48).map(|i| (i % 8) as u16 * 1000).collect());
    list.apply(&mut image, OpcodeStage::List1)?;
    let expected: Vec<u16> = (0..48).map(|i| 2000 + (i % 8) as u16 * 500).collect();
    assert!(matches!(&image.data, RawImageData::Integer(samples) if samples == &expected));
    // Integer data is not mapped to linear reference values
    assert!(list.apply(&mut image, OpcodeStage::List2).is_err());
    Ok(())
  }
}
