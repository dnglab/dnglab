// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! DNG opcode WarpFisheye (DNG 1.3)

use serde::{Deserialize, Serialize};

use crate::{RawlerError, Result};

use super::{
  OpcodeImage, OpcodeStage,
  helpers::{ParamReader, RadialModel, WARP_MAX_PLANES, WarpPlane, apply_warp, put_f64, put_u32},
};

/// Warp to map a fisheye image to a perspective projection, also for distortion and
/// lateral chromatic aberration. Geometry and
/// resampling are the same as for WarpRectilinear.
#[derive(Debug, Clone, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct WarpFisheye {
  pub version: u32,
  pub flags: u32,
  /// Radial coefficients kr0 to kr3 for each coefficient set
  pub coeffs: Vec<[f64; 4]>,
  pub c_x: f64,
  pub c_y: f64,
}

impl WarpFisheye {
  pub(super) fn parse(reader: &mut ParamReader, version: u32, flags: u32) -> Result<Self> {
    let n = reader.u32()? as usize;
    reader.check_count(n, 4 * 8, "WarpFisheye")?;
    let mut coeffs = Vec::with_capacity(n);
    for _ in 0..n {
      coeffs.push([reader.f64()?, reader.f64()?, reader.f64()?, reader.f64()?]);
    }
    let warp = Self {
      version,
      flags,
      coeffs,
      c_x: reader.f64()?,
      c_y: reader.f64()?,
    };
    warp.validate()?;
    Ok(warp)
  }

  pub(super) fn write_params(&self, buf: &mut Vec<u8>) {
    put_u32(buf, self.coeffs.len() as u32);
    for value in self.coeffs.iter().flatten() {
      put_f64(buf, *value);
    }
    put_f64(buf, self.c_x);
    put_f64(buf, self.c_y);
  }

  /// Validate the parameters
  fn validate(&self) -> Result<()> {
    let invalid = |what: &str| Err(RawlerError::DecoderFailed(format!("DNG opcode WarpFisheye is invalid: {}", what)));
    if self.coeffs.is_empty() || self.coeffs.len() > WARP_MAX_PLANES {
      return invalid("number of coefficient sets must be 1 to 4");
    }
    if !self.coeffs.iter().flatten().all(|v| v.is_finite()) {
      return invalid("coefficients must be finite");
    }
    if !((0.0..=1.0).contains(&self.c_x) && (0.0..=1.0).contains(&self.c_y)) {
      return invalid("optical center must be in range 0.0 to 1.0");
    }
    Ok(())
  }

  pub fn apply(&self, image: &mut OpcodeImage<'_>, stage: OpcodeStage) -> Result<()> {
    self.validate()?;
    apply_warp("WarpFisheye", &self.planes(), (self.c_x, self.c_y), image, stage)
  }

  fn planes(&self) -> Vec<WarpPlane> {
    self
      .coeffs
      .iter()
      .map(|coeffs| WarpPlane {
        radial: RadialModel::Fisheye { coeffs: *coeffs },
        tangential: [0.0, 0.0],
      })
      .collect()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{
    dng::opcodes::{Opcode, OpcodeList, helpers::WarpGeometry, testutil::*},
    imgop::Dim2,
  };

  fn warp_fisheye(sets: &[[f64; 4]], center: (f64, f64)) -> Vec<u8> {
    let mut buf = Vec::new();
    put_u32(&mut buf, sets.len() as u32);
    sets.iter().flatten().for_each(|v| put_f64(&mut buf, *v));
    put_f64(&mut buf, center.0);
    put_f64(&mut buf, center.1);
    buf
  }

  fn single_fisheye(sets: &[[f64; 4]], center: (f64, f64)) -> Result<OpcodeList> {
    OpcodeList::parse(&opcode_list(&[opcode(2, V1_3, 0, &warp_fisheye(sets, center))]))
  }

  fn first_fisheye(list: &OpcodeList) -> &WarpFisheye {
    match &list.opcodes()[0] {
      Opcode::WarpFisheye(warp) => warp,
      _ => panic!("expected WarpFisheye"),
    }
  }

  const FISHEYE: [f64; 4] = [1.0, 0.0, 0.0, 0.0];

  #[test]
  fn warp_fisheye_parse() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let data = opcode_list(&[opcode(2, V1_3, 1, &warp_fisheye(&[[0.9, 0.1, -0.05, 0.01], FISHEYE], (0.4, 0.6)))]);
    let list = OpcodeList::parse(&data)?;
    assert_eq!(first_fisheye(&list).coeffs, [[0.9, 0.1, -0.05, 0.01], FISHEYE]);
    assert_eq!(list.to_bytes()?, data);
    // No or too many coefficient sets, optical center out of range, NaN coefficient
    for params in [
      warp_fisheye(&[], (0.5, 0.5)),
      warp_fisheye(&[FISHEYE; 5], (0.5, 0.5)),
      warp_fisheye(&[FISHEYE], (1.5, 0.5)),
      warp_fisheye(&[[f64::NAN, 0.0, 0.0, 0.0]], (0.5, 0.5)),
    ] {
      assert!(OpcodeList::parse(&opcode_list(&[opcode(2, V1_3, 0, &params)])).is_err());
      let list = OpcodeList::parse(&opcode_list(&[opcode(2, V1_3, 1, &params)]))?;
      assert!(matches!(&list.opcodes()[0], Opcode::Unknown(op) if op.id == 2));
    }
    let mut params = warp_fisheye(&[FISHEYE], (0.5, 0.5));
    params.push(0);
    assert!(OpcodeList::parse(&opcode_list(&[opcode(2, V1_3, 0, &params)])).is_err());
    Ok(())
  }

  #[test]
  fn warp_fisheye_geometry() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // f = arctan(r) / r
    let list = single_fisheye(&[FISHEYE], (0.5, 0.5))?;
    let plane = &first_fisheye(&list).planes()[0];
    assert!((plane.ratio(1.0) - std::f64::consts::FRAC_PI_4).abs() < 1e-15);
    assert!((plane.ratio(0.25) - 0.9272952180016122).abs() < 1e-15);
    assert_eq!(plane.ratio(0.0), 1.0);
    let list = single_fisheye(&[[0.9, 0.1, -0.05, 0.01]], (0.5, 0.5))?;
    assert!((first_fisheye(&list).planes()[0].ratio(0.25) - 0.8524492017196762).abs() < 1e-15);
    // 8x6 image: optical center (4, 3), radius 5
    let geometry = WarpGeometry::new((0.5, 0.5), Dim2::new(8, 6), 1.0);
    let (x, y) = geometry.source(plane, 0.0, 0.0);
    assert!((x - 0.8584073464102069).abs() < 1e-12 && (y - 0.6438055098076552).abs() < 1e-12, "{} {}", x, y);
    Ok(())
  }

  #[test]
  fn warp_fisheye_apply() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Constant image stays constant, a fisheye warp is never an identity
    let mut image = float_image(8, 6, 1, 0.3);
    single_fisheye(&[FISHEYE], (0.5, 0.5))?.apply(&mut image, OpcodeStage::List3)?;
    assert!(floats(&image).iter().all(|v| (v - 0.3).abs() < 1e-6));
    let mut image = ramp_image(8, 6, 1, 1.0 / 8.0);
    let original = floats(&image).to_vec();
    single_fisheye(&[FISHEYE], (0.5, 0.5))?.apply(&mut image, OpcodeStage::List3)?;
    assert_ne!(floats(&image), original);
    // One coefficient set per plane
    let mut image = ramp_image(8, 6, 3, 1.0 / 8.0);
    single_fisheye(&[FISHEYE, [0.9, 0.0, 0.0, 0.0], FISHEYE], (0.5, 0.5))?.apply(&mut image, OpcodeStage::List3)?;
    let pixels = floats(&image).as_chunks::<3>().0;
    assert!(pixels.iter().all(|p| p[0] == p[2]));
    assert!(pixels.iter().any(|p| p[0] != p[1]));
    // Number of coefficient sets must be 1 or the number of planes
    assert!(single_fisheye(&[FISHEYE, FISHEYE], (0.5, 0.5))?.apply(&mut image, OpcodeStage::List3).is_err());
    Ok(())
  }

  #[test]
  fn warp_fisheye_skip_rule() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let warp2_params = warp_rectilinear2(&[(radial(&[0.5]), NO_TANGENTIAL, FULL_RANGE)], (0.5, 0.5), 0);
    let fisheye = opcode(2, V1_3, 0, &warp_fisheye(&[FISHEYE], (0.5, 0.5)));
    let run = |opcodes: &[Vec<u8>]| -> Result<Vec<f32>> {
      let mut image = ramp_image(8, 6, 1, 1.0 / 8.0);
      OpcodeList::parse(&opcode_list(opcodes))?.apply(&mut image, OpcodeStage::List3)?;
      Ok(floats(&image).to_vec())
    };
    let warp2_only = run(&[opcode(14, V1_6, 1, &warp2_params)])?;
    // WarpFisheye immediately following an optional WarpRectilinear2 is skipped
    assert_eq!(run(&[opcode(14, V1_6, 1, &warp2_params), fisheye.clone()])?, warp2_only);
    assert_ne!(run(&[opcode(14, V1_6, 0, &warp2_params), fisheye])?, warp2_only);
    Ok(())
  }
}
