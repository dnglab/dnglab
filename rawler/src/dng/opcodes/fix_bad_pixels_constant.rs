// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! DNG opcode FixBadPixelsConstant (DNG 1.3)

use serde::{Deserialize, Serialize};

use crate::{
  RawImage, RawImageData, RawlerError, Result,
  imgop::{Dim2, crop},
};

use super::{
  OpcodeStage,
  helpers::{ParamReader, apply_roi_threaded, bayer_is_green, put_u32, same_phase_index, stage_bounds},
};

/// Patches bad pixels of a Bayer CFA image that are marked with a constant value.
/// A bad pixel is interpolated from the nearest pixels of the same color that are
/// not bad.
#[derive(Debug, Clone, Eq, PartialEq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FixBadPixelsConstant {
  pub version: u32,
  pub flags: u32,
  /// Value of bad pixels
  pub constant: u32,
  /// Color of the top-left pixel: 0 = red, 1 = green in a green/red row,
  /// 2 = green in a green/blue row, 3 = blue
  pub bayer_phase: u32,
}

impl FixBadPixelsConstant {
  pub(super) fn parse(reader: &mut ParamReader, version: u32, flags: u32) -> Result<Self> {
    let fix = Self {
      version,
      flags,
      constant: reader.u32()?,
      bayer_phase: reader.u32()?,
    };
    if fix.bayer_phase > 3 {
      return Err(RawlerError::DecoderFailed(format!(
        "DNG opcode FixBadPixelsConstant: invalid BayerPhase {}",
        fix.bayer_phase
      )));
    }
    Ok(fix)
  }

  pub(super) fn write_params(&self, buf: &mut Vec<u8>) {
    put_u32(buf, self.constant);
    put_u32(buf, self.bayer_phase);
  }

  pub fn apply(&self, rawimage: &mut RawImage, stage: OpcodeStage) -> Result<()> {
    let bounds = stage_bounds(rawimage, stage);
    let (width, height, cpp) = (rawimage.width, rawimage.height, rawimage.cpp);
    let samples = match &mut rawimage.data {
      RawImageData::Integer(samples) if cpp == 1 => samples,
      _ => {
        return Err(RawlerError::DecoderFailed(format!(
          "DNG opcode FixBadPixelsConstant in {} requires 16 bit raw data with a single plane",
          stage
        )));
      }
    };
    // A constant above the 16 bit range can't mark a pixel
    let Ok(bad) = u16::try_from(self.constant) else {
      return Ok(());
    };
    if bounds.width() == 0 || bounds.height() == 0 {
      return Ok(());
    }
    // Bad pixels are interpolated from the original values
    let dim = bounds.d;
    let src = crop(samples, Dim2::new(width, height), bounds);
    apply_roi_threaded(samples, width, 1, bounds, 1, &|line, row| {
      let y = row - bounds.y();
      for (x, value) in line.iter_mut().enumerate() {
        if src[y * dim.w + x] != bad {
          continue;
        }
        let neighbours: [(isize, isize); 4] = if bayer_is_green(self.bayer_phase, y, x) {
          [(-1, -1), (-1, 1), (1, -1), (1, 1)]
        } else {
          [(-2, 0), (2, 0), (0, -2), (0, 2)]
        };
        let (mut count, mut total) = (0_u32, 0_u32);
        for (dy, dx) in neighbours {
          let neighbour = src[same_phase_index(y as isize + dy, dim.h) * dim.w + same_phase_index(x as isize + dx, dim.w)];
          if neighbour != bad {
            count += 1;
            total += neighbour as u32;
          }
        }
        match count {
          0 => {}
          4 => *value = ((total + 2) >> 2) as u16,
          _ => *value = ((total + count / 2) / count) as u16,
        }
      }
    });
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::dng::opcodes::{Opcode, OpcodeList, testutil::*};

  fn fix_bad_pixels_constant(constant: u32, phase: u32) -> Vec<u8> {
    let mut buf = Vec::new();
    put_u32(&mut buf, constant);
    put_u32(&mut buf, phase);
    buf
  }

  /// 8x8 image with distinct values that are not linear in row and column
  fn bayer_value(row: usize, col: usize) -> u16 {
    ((row * 37 + col * 91 + row * col * 13) % 500 + 100) as u16
  }

  fn bayer_image(bad: &[(usize, usize)]) -> RawImage {
    let mut image = float_image(8, 8, 1, 0.0);
    image.data = RawImageData::Integer(
      (0..64)
        .map(|i| if bad.contains(&(i / 8, i % 8)) { 0 } else { bayer_value(i / 8, i % 8) })
        .collect(),
    );
    image
  }

  /// Rounded average of the original values at `pixels`
  fn average(pixels: &[(usize, usize)]) -> u16 {
    let total: u32 = pixels.iter().map(|(r, c)| bayer_value(*r, *c) as u32).sum();
    let count = pixels.len() as u32;
    ((total + count / 2) / count) as u16
  }

  fn fix_bad_pixels(image: &mut RawImage, phase: u32) -> Result<()> {
    OpcodeList::parse(&opcode_list(&[opcode(4, V1_3, 0, &fix_bad_pixels_constant(0, phase))]))?.apply(image, OpcodeStage::List1)
  }

  #[test]
  fn fix_bad_pixels_constant_parse() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let data = opcode_list(&[opcode(4, V1_3, 0, &fix_bad_pixels_constant(0, 2))]);
    let list = OpcodeList::parse(&data)?;
    assert!(matches!(&list.opcodes()[0], Opcode::FixBadPixelsConstant(op) if (op.constant, op.bayer_phase) == (0, 2)));
    assert_eq!(list.to_bytes()?, data);
    // Invalid BayerPhase
    assert!(OpcodeList::parse(&opcode_list(&[opcode(4, V1_3, 0, &fix_bad_pixels_constant(0, 4))])).is_err());
    let list = OpcodeList::parse(&opcode_list(&[opcode(4, V1_3, 1, &fix_bad_pixels_constant(0, 4))]))?;
    assert!(matches!(&list.opcodes()[0], Opcode::Unknown(op) if op.id == 4));
    // Parameter area must be 8 bytes
    let mut params = fix_bad_pixels_constant(0, 0);
    params.push(0);
    assert!(OpcodeList::parse(&opcode_list(&[opcode(4, V1_3, 0, &params)])).is_err());
    Ok(())
  }

  #[test]
  fn fix_bad_pixels_constant_interpolation() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // BayerPhase 0: red pixel at (0, 0), (3, 4) is green, (4, 4) is red
    let mut image = bayer_image(&[(3, 4), (4, 4)]);
    fix_bad_pixels(&mut image, 0)?;
    for (i, value) in integers(&image).iter().enumerate() {
      let expected = match (i / 8, i % 8) {
        (3, 4) => average(&[(2, 3), (2, 5), (4, 3), (4, 5)]),
        (4, 4) => average(&[(2, 4), (6, 4), (4, 2), (4, 6)]),
        (row, col) => bayer_value(row, col),
      };
      assert_eq!(*value, expected, "pixel {}", i);
    }
    // BayerPhase 1: green pixel at (0, 0), (3, 4) is red or blue, (4, 4) is green
    let mut image = bayer_image(&[(3, 4), (4, 4)]);
    fix_bad_pixels(&mut image, 1)?;
    assert_eq!(integers(&image)[3 * 8 + 4], average(&[(1, 4), (5, 4), (3, 2), (3, 6)]));
    assert_eq!(integers(&image)[4 * 8 + 4], average(&[(3, 3), (3, 5), (5, 3), (5, 5)]));
    Ok(())
  }

  #[test]
  fn fix_bad_pixels_constant_bad_neighbours_and_edges() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Bad neighbours are skipped
    let mut image = bayer_image(&[(4, 4), (2, 4)]);
    fix_bad_pixels(&mut image, 0)?;
    assert_eq!(integers(&image)[4 * 8 + 4], average(&[(6, 4), (4, 2), (4, 6)]));
    assert_eq!(integers(&image)[2 * 8 + 4], average(&[(0, 4), (2, 2), (2, 6)]));
    // No good neighbour
    let mut image = bayer_image(&[(4, 4), (2, 4), (6, 4), (4, 2), (4, 6)]);
    fix_bad_pixels(&mut image, 0)?;
    assert_eq!(integers(&image)[4 * 8 + 4], 0);
    // Pixels outside of the image are replicated from pixels of the same color
    let mut image = bayer_image(&[(0, 0), (0, 1), (7, 7)]);
    fix_bad_pixels(&mut image, 0)?;
    assert_eq!(integers(&image)[0], average(&[(2, 0), (0, 2)]));
    assert_eq!(integers(&image)[1], average(&[(1, 0), (1, 2), (1, 0), (1, 2)]));
    assert_eq!(integers(&image)[63], average(&[(5, 7), (7, 5)]));
    Ok(())
  }

  #[test]
  fn fix_bad_pixels_constant_data_checks() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut image = float_image(8, 8, 1, 0.0);
    assert!(fix_bad_pixels(&mut image, 0).is_err());
    let mut image = float_image(4, 4, 3, 0.0);
    image.data = RawImageData::Integer(vec![0; 48]);
    assert!(fix_bad_pixels(&mut image, 0).is_err());
    // A constant above 16 bit doesn't mark pixels
    let mut image = bayer_image(&[(4, 4)]);
    let list = OpcodeList::parse(&opcode_list(&[opcode(4, V1_3, 0, &fix_bad_pixels_constant(70000, 0))]))?;
    list.apply(&mut image, OpcodeStage::List1)?;
    assert_eq!(integers(&image)[4 * 8 + 4], 0);
    Ok(())
  }
}
