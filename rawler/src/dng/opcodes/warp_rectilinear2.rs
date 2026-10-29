// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! DNG opcode WarpRectilinear2 (DNG 1.6)

use serde::{Deserialize, Serialize};

use crate::{RawlerError, Result};

use super::{
  OpcodeImage, OpcodeStage,
  helpers::{ParamReader, RadialModel, WARP_MAX_PLANES, WARP_RADIAL_TERMS, WarpPlane, apply_warp, put_f64, put_u32},
};

/// Rectilinear warp with a higher-order radial distortion model (DNG 1.6).
/// Geometry and resampling are the same as for WarpRectilinear.
#[derive(Debug, Clone, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct WarpRectilinear2 {
  pub version: u32,
  pub flags: u32,
  pub coeffs: Vec<WarpRectilinear2Coeffs>,
  pub c_x: f64,
  pub c_y: f64,
  /// Non-zero if the ideal radius is divided by the radial polynomial (reciprocalRadial)
  pub reciprocal_radial: u32,
}

impl WarpRectilinear2 {
  pub(super) fn parse(reader: &mut ParamReader, version: u32, flags: u32) -> Result<Self> {
    let n = reader.u32()? as usize;
    reader.check_count(n, (WARP_RADIAL_TERMS + 4) * 8, "WarpRectilinear2")?;
    let mut coeffs = Vec::with_capacity(n);
    for _ in 0..n {
      let mut k_r = [0.0; WARP_RADIAL_TERMS];
      for k in k_r.iter_mut() {
        *k = reader.f64()?;
      }
      coeffs.push(WarpRectilinear2Coeffs {
        k_r,
        k_t0: reader.f64()?,
        k_t1: reader.f64()?,
        min_valid_radius: reader.f64()?,
        max_valid_radius: reader.f64()?,
      });
    }
    let c_x = reader.f64()?;
    let c_y = reader.f64()?;
    let reciprocal_radial = reader.u32()?;

    let warp = Self {
      version,
      flags,
      coeffs,
      c_x,
      c_y,
      reciprocal_radial,
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
    put_u32(buf, self.reciprocal_radial);
  }

  /// Validate the parameters
  fn validate(&self) -> Result<()> {
    let invalid = |what: &str| Err(RawlerError::DecoderFailed(format!("DNG opcode WarpRectilinear2 is invalid: {}", what)));
    if self.coeffs.is_empty() || self.coeffs.len() > WARP_MAX_PLANES {
      return invalid("number of coefficient sets must be 1 to 4");
    }
    if !self.coeffs.iter().all(|c| c.values().all(f64::is_finite)) {
      return invalid("coefficients must be finite");
    }
    if !self
      .coeffs
      .iter()
      .all(|c| 0.0 <= c.min_valid_radius && c.min_valid_radius < c.max_valid_radius && c.max_valid_radius <= 1.0)
    {
      return invalid("valid radius range must satisfy 0.0 <= min < max <= 1.0");
    }
    if !((0.0..=1.0).contains(&self.c_x) && (0.0..=1.0).contains(&self.c_y)) {
      return invalid("optical center must be in range 0.0 to 1.0");
    }
    Ok(())
  }

  pub fn apply(&self, image: &mut OpcodeImage<'_>, stage: OpcodeStage) -> Result<()> {
    self.validate()?;
    apply_warp("WarpRectilinear2", &self.planes(), (self.c_x, self.c_y), image, stage)
  }

  fn planes(&self) -> Vec<WarpPlane> {
    self
      .coeffs
      .iter()
      .map(|c| WarpPlane {
        radial: RadialModel::Polynomial {
          coeffs: c.k_r,
          valid_radius: [c.min_valid_radius, c.max_valid_radius],
          reciprocal: self.reciprocal_radial != 0,
        },
        tangential: [c.k_t0, c.k_t1],
      })
      .collect()
  }
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct WarpRectilinear2Coeffs {
  /// Radial coefficients kr0 to kr14 for r^0 to r^14
  pub k_r: [f64; WARP_RADIAL_TERMS],
  pub k_t0: f64,
  pub k_t1: f64,
  pub min_valid_radius: f64,
  pub max_valid_radius: f64,
}

impl WarpRectilinear2Coeffs {
  /// All coefficients in stored order
  fn values(&self) -> impl Iterator<Item = f64> + '_ {
    self
      .k_r
      .iter()
      .copied()
      .chain([self.k_t0, self.k_t1, self.min_valid_radius, self.max_valid_radius])
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{
    RawImageData,
    dng::{
      DngVersionNumber,
      opcodes::{Opcode, OpcodeList, helpers::WarpGeometry, testutil::*},
    },
    imgop::Dim2,
  };

  fn first_warp2(list: &OpcodeList) -> &WarpRectilinear2 {
    match &list.opcodes()[0] {
      Opcode::WarpRectilinear2(warp) => warp,
      _ => panic!("expected WarpRectilinear2"),
    }
  }

  #[test]
  fn warp_rectilinear2_parse() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let params = warp_rectilinear2(&[(radial(&[0.99, 0.01, 0.02, -0.03]), [0.001, -0.002], FULL_RANGE)], (0.5, 0.5), 7);
    assert_eq!(params.len(), 4 + 19 * 8 + 16 + 4);
    let data = opcode_list(&[opcode(14, V1_6, 1, &params)]);
    let list = OpcodeList::parse(&data)?;
    assert_eq!(first_warp2(&list).reciprocal_radial, 7);
    assert_eq!(list.opcodes()[0].version(), DngVersionNumber::from([1, 6, 0, 0]));
    assert_eq!(list.to_bytes()?, data);
    // No or too many coefficient sets, invalid valid radius ranges, NaN coefficient, center out of range
    let set = (radial(&[1.0]), NO_TANGENTIAL, FULL_RANGE);
    for params in [
      warp_rectilinear2(&[], (0.5, 0.5), 0),
      warp_rectilinear2(&[set; 5], (0.5, 0.5), 0),
      warp_rectilinear2(&[(radial(&[1.0]), NO_TANGENTIAL, [0.5, 0.5])], (0.5, 0.5), 0),
      warp_rectilinear2(&[(radial(&[1.0]), NO_TANGENTIAL, [0.0, 1.5])], (0.5, 0.5), 0),
      warp_rectilinear2(&[(radial(&[1.0]), NO_TANGENTIAL, [-0.1, 1.0])], (0.5, 0.5), 0),
      warp_rectilinear2(&[(radial(&[1.0, f64::NAN]), NO_TANGENTIAL, FULL_RANGE)], (0.5, 0.5), 0),
      warp_rectilinear2(&[set], (0.5, -0.1), 0),
    ] {
      assert!(OpcodeList::parse(&opcode_list(&[opcode(14, V1_6, 0, &params)])).is_err());
      let list = OpcodeList::parse(&opcode_list(&[opcode(14, V1_6, 1, &params)]))?;
      assert!(matches!(&list.opcodes()[0], Opcode::Unknown(op) if op.id == 14));
    }
    Ok(())
  }

  #[test]
  fn warp_rectilinear2_matches_warp_rectilinear() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Even radial terms and tangential terms of WarpRectilinear
    let (kr, kt) = (
      [0.9897059202194214, 0.02857298031449318, -0.05583058297634125, 0.03774712607264519],
      [0.003, -0.002],
    );
    let warp = single_warp(&[[kr[0], kr[1], kr[2], kr[3], kt[0], kt[1]]], (0.48, 0.53))?;
    let params = warp_rectilinear2(&[(radial(&[kr[0], 0.0, kr[1], 0.0, kr[2], 0.0, kr[3]]), kt, FULL_RANGE)], (0.48, 0.53), 0);
    let warp2 = OpcodeList::parse(&opcode_list(&[opcode(14, V1_6, 0, &params)]))?;
    let mut image = float_image(16, 12, 1, 0.0);
    image.data = RawImageData::Float((0..16 * 12).map(|i| ((i % 16) + 2 * (i / 16)) as f32 / 64.0).collect());
    let mut image2 = image.clone();
    warp.apply(&mut image, OpcodeStage::List3)?;
    warp2.apply(&mut image2, OpcodeStage::List3)?;
    for (a, b) in floats(&image).iter().zip(floats(&image2)) {
      assert!((a - b).abs() < 1e-6, "{} {}", a, b);
    }
    Ok(())
  }

  #[test]
  fn warp_rectilinear2_geometry() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // 8x6 image: optical center (4, 3), radius 5
    let source_x = |coeffs: &[f64], valid_radius: [f64; 2], x: f64| -> Result<f64> {
      let params = warp_rectilinear2(&[(radial(coeffs), NO_TANGENTIAL, valid_radius)], (0.5, 0.5), 0);
      let list = OpcodeList::parse(&opcode_list(&[opcode(14, V1_6, 0, &params)]))?;
      let geometry = WarpGeometry::new((0.5, 0.5), Dim2::new(8, 6), 1.0);
      let (sx, sy) = geometry.source(&first_warp2(&list).planes()[0], x, 3.0);
      assert_eq!(sy, 3.0);
      Ok(sx)
    };
    // Odd radial term: f(r) = 1 + 0.1 * r, at r = 0.4 (WarpRectilinear with kr1 = 0.1 would give 6.032)
    assert!((source_x(&[1.0, 0.1], FULL_RANGE, 6.0)? - 6.08).abs() < 1e-12);
    // f(r) = 1 + 0.2 * r^2 is evaluated at the clamped radius 0.5, for r = 0.8 and r = 0.2
    assert!((source_x(&[1.0, 0.0, 0.2], [0.0, 0.5], 0.0)? + 0.2).abs() < 1e-12);
    assert!((source_x(&[1.0, 0.0, 0.2], [0.5, 1.0], 5.0)? - 5.05).abs() < 1e-12);
    Ok(())
  }

  #[test]
  fn warp_rectilinear2_reciprocal() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // r_real = r_ideal / 2, the same zoom as kr0 = 0.5
    let params = warp_rectilinear2(&[(radial(&[2.0]), NO_TANGENTIAL, FULL_RANGE)], (0.5, 0.5), 1);
    let mut image = ramp_image(8, 6, 1, 1.0 / 8.0);
    OpcodeList::parse(&opcode_list(&[opcode(14, V1_6, 0, &params)]))?.apply(&mut image, OpcodeStage::List3)?;
    for (i, value) in floats(&image).iter().enumerate() {
      let x = (i % 8) as f32;
      assert_eq!(*value, (2.0 + x / 2.0) / 8.0, "pixel {}", i);
    }
    Ok(())
  }

  #[test]
  fn warp_rectilinear2_skip_rule() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let warp2_params = warp_rectilinear2(&[(radial(&[0.5]), NO_TANGENTIAL, FULL_RANGE)], (0.5, 0.5), 0);
    let warp2_optional = opcode(14, V1_6, 1, &warp2_params);
    let warp2_required = opcode(14, V1_6, 0, &warp2_params);
    let warp = opcode(1, V1_3, 0, &warp_rectilinear(&[[0.8, 0.0, 0.0, 0.0, 0.0, 0.0]], (0.5, 0.5)));
    let gain = opcode(9, V1_3, 0, &gain_map([0, 0, 0, 0, 0, 1, 1, 1], (1, 1), (1.0, 1.0), (0.0, 0.0), 1, &[1.0]));
    let run = |opcodes: &[&Vec<u8>]| -> Result<Vec<f32>> {
      let mut image = ramp_image(8, 6, 1, 1.0 / 8.0);
      let opcodes: Vec<Vec<u8>> = opcodes.iter().map(|op| (*op).clone()).collect();
      OpcodeList::parse(&opcode_list(&opcodes))?.apply(&mut image, OpcodeStage::List3)?;
      Ok(floats(&image).to_vec())
    };
    let warp2_only = run(&[&warp2_optional])?;
    let warp2_and_warp = run(&[&warp2_required, &warp])?;
    assert_ne!(warp2_only, warp2_and_warp);
    // WarpRectilinear immediately following an optional WarpRectilinear2 is skipped
    assert_eq!(run(&[&warp2_optional, &warp])?, warp2_only);
    // Only the immediately following warp is skipped
    assert_eq!(run(&[&warp2_optional, &warp, &warp])?, warp2_and_warp);
    assert_eq!(run(&[&warp2_optional, &gain, &warp])?, run(&[&warp2_required, &gain, &warp])?);
    Ok(())
  }
}
