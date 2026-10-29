// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! DNG opcode TrimBounds (DNG 1.3)

use serde::{Deserialize, Serialize};

use crate::{
  RawlerError, Result,
  imgop::{Point, Rect},
  pixarray::ImageViewMut,
};

use super::{
  OpcodeData, OpcodeImage, OpcodeStage,
  helpers::{ParamReader, put_u32, stage_bounds},
};

/// Trims the image to a rectangle. The trimmed image becomes the image of the stage with its origin at the
/// top-left corner of the rectangle, so following opcodes use trimmed coordinates.
///
/// - OpcodeList1 crops the stored raw image. ActiveArea and DefaultCrop are
///   not adjusted, their coordinates apply to the trimmed image.
/// - OpcodeList2 and OpcodeList3 trim the stage image, which is the ActiveArea.
///   DefaultCrop is relative to the stage 3 image, so it moves with the trim.
#[derive(Debug, Clone, Eq, PartialEq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TrimBounds {
  pub version: u32,
  pub flags: u32,
  pub top: u32,
  pub left: u32,
  pub bottom: u32,
  pub right: u32,
}

impl TrimBounds {
  pub(super) fn parse(reader: &mut ParamReader, version: u32, flags: u32) -> Result<Self> {
    let trim = Self {
      version,
      flags,
      top: reader.u32()?,
      left: reader.u32()?,
      bottom: reader.u32()?,
      right: reader.u32()?,
    };
    if trim.is_empty() {
      return Err(RawlerError::DecoderFailed(format!("DNG opcode TrimBounds: rectangle is empty: {:?}", trim)));
    }
    Ok(trim)
  }

  pub(super) fn write_params(&self, buf: &mut Vec<u8>) {
    for value in [self.top, self.left, self.bottom, self.right] {
      put_u32(buf, value);
    }
  }

  fn is_empty(&self) -> bool {
    self.top >= self.bottom || self.left >= self.right
  }

  pub fn apply(&self, image: &mut OpcodeImage<'_>, stage: OpcodeStage) -> Result<()> {
    let bounds = stage_bounds(image, stage);
    if self.is_empty() || self.bottom as usize > bounds.d.h || self.right as usize > bounds.d.w {
      return Err(RawlerError::DecoderFailed(format!(
        "DNG opcode TrimBounds in {}: rectangle (t={}, l={}, b={}, r={}) is empty or exceeds image of {}x{}",
        stage, self.top, self.left, self.bottom, self.right, bounds.d.w, bounds.d.h
      )));
    }
    let rect = Rect::new_with_points(
      Point::new(self.left as usize, self.top as usize),
      Point::new(self.right as usize, self.bottom as usize),
    );
    match stage {
      OpcodeStage::List1 => Self::trim_raw_image(image, rect)?,
      OpcodeStage::List2 | OpcodeStage::List3 => {
        let area = Rect::new(Point::new(bounds.p.x + rect.p.x, bounds.p.y + rect.p.y), rect.d);
        image.active_area = Some(area);
        // The stage 3 image now starts at the top-left corner of the rectangle
        image.crop_area = image
          .crop_area
          .and_then(|crop_area| Rect::new(Point::new(crop_area.p.x + rect.p.x, crop_area.p.y + rect.p.y), crop_area.d).intersect(&area));
      }
    }
    Ok(())
  }

  /// Crop the stored raw image (OpcodeList1). ActiveArea and DefaultCrop keep
  /// their coordinates, which now apply to the trimmed image.
  fn trim_raw_image(image: &mut OpcodeImage<'_>, rect: Rect) -> Result<()> {
    let trimmed = Rect::new(Point::zero(), rect.d);
    if let Some(active_area) = image.active_area
      && active_area.intersect(&trimmed) != Some(active_area)
    {
      return Err(RawlerError::DecoderFailed(format!(
        "DNG opcode TrimBounds in OpcodeList1: ActiveArea {:?} exceeds trimmed image of {}x{}",
        active_area, rect.d.w, rect.d.h
      )));
    }
    // The trimmed image is moved to the start of the image data
    let data = std::mem::replace(&mut image.data, OpcodeData::Float(ImageViewMut::new(&mut [], 0, 0, 1)));
    image.data = data.crop_in_place(rect)?;
    image.crop_area = image.crop_area.and_then(|crop_area| crop_area.intersect(&trimmed));
    // Masked areas are relative to the stored raw image
    image.black_areas = image.black_areas.iter().filter_map(|masked| masked.rebase(&rect)).collect();
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::dng::opcodes::{Opcode, OpcodeList, testutil::*};
  use crate::{RawImageData, imgop::Dim2};

  fn trim_bounds(top: u32, left: u32, bottom: u32, right: u32) -> Vec<u8> {
    let mut buf = Vec::new();
    [top, left, bottom, right].iter().for_each(|v| put_u32(&mut buf, *v));
    buf
  }

  fn single_trim_bounds(top: u32, left: u32, bottom: u32, right: u32) -> Result<OpcodeList> {
    OpcodeList::parse(&opcode_list(&[opcode(6, V1_3, 0, &trim_bounds(top, left, bottom, right))]))
  }

  #[test]
  fn trim_bounds_parse() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let data = opcode_list(&[opcode(6, V1_3, 0, &trim_bounds(1, 2, 3, 5))]);
    let list = OpcodeList::parse(&data)?;
    assert!(matches!(&list.opcodes()[0], Opcode::TrimBounds(op) if (op.top, op.left, op.bottom, op.right) == (1, 2, 3, 5)));
    assert_eq!(list.to_bytes()?, data);
    // Empty rectangle
    assert!(OpcodeList::parse(&opcode_list(&[opcode(6, V1_3, 0, &trim_bounds(3, 2, 3, 5))])).is_err());
    let list = OpcodeList::parse(&opcode_list(&[opcode(6, V1_3, 1, &trim_bounds(3, 2, 3, 5))]))?;
    assert!(matches!(&list.opcodes()[0], Opcode::Unknown(op) if op.id == 6));
    // Parameter area must be 16 bytes
    let mut params = trim_bounds(1, 2, 3, 5);
    params.push(0);
    assert!(OpcodeList::parse(&opcode_list(&[opcode(6, V1_3, 0, &params)])).is_err());
    Ok(())
  }

  #[test]
  fn trim_bounds_list1_crops_raw_image() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let list = single_trim_bounds(1, 2, 3, 5)?;
    let mut image = float_image(6, 4, 1, 0.0);
    image.data = RawImageData::Integer((0..24).collect());
    image.active_area = Some(Rect::new(Point::new(0, 0), Dim2::new(2, 1)));
    image.crop_area = Some(Rect::new(Point::new(1, 0), Dim2::new(5, 5)));
    image.blackareas = vec![Rect::new(Point::new(0, 0), Dim2::new(6, 1)), Rect::new(Point::new(4, 1), Dim2::new(2, 3))];
    list.apply(&mut image, OpcodeStage::List1)?;
    assert!(matches!(&image.data, RawImageData::Integer(samples) if samples == &[8, 9, 10, 14, 15, 16]));
    assert_eq!((image.width, image.height), (3, 2));
    // ActiveArea keeps its coordinates, DefaultCrop is clipped, masked areas are moved
    assert_eq!(image.active_area, Some(Rect::new(Point::new(0, 0), Dim2::new(2, 1))));
    assert_eq!(image.crop_area, Some(Rect::new(Point::new(1, 0), Dim2::new(2, 2))));
    assert_eq!(image.blackareas, [Rect::new(Point::new(2, 0), Dim2::new(1, 2))]);
    // ActiveArea must fit into the trimmed image
    let mut image = float_image(6, 4, 1, 0.0);
    image.active_area = Some(Rect::new(Point::new(0, 0), Dim2::new(4, 1)));
    assert!(list.apply(&mut image, OpcodeStage::List1).is_err());
    // Multiple planes
    let mut image = float_image(3, 2, 3, 0.0);
    image.data = RawImageData::Float((0..18).map(|v| v as f32).collect());
    single_trim_bounds(0, 1, 2, 3)?.apply(&mut image, OpcodeStage::List1)?;
    assert_eq!(floats(&image), [3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0]);
    Ok(())
  }

  #[test]
  fn trim_bounds_list2_shrinks_active_area() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut image = float_image(8, 6, 1, 0.25);
    image.active_area = Some(Rect::new(Point::new(2, 1), Dim2::new(4, 4)));
    image.crop_area = Some(Rect::new(Point::new(3, 2), Dim2::new(3, 3)));
    // Following opcodes use the coordinates of the trimmed image
    let list = OpcodeList::parse(&opcode_list(&[
      opcode(6, V1_3, 0, &trim_bounds(1, 1, 3, 4)),
      opcode(8, V1_3, 0, &map_polynomial([0, 0, 1, 1, 0, 1, 1, 1], &[1.0])),
    ]))?;
    list.apply(&mut image, OpcodeStage::List2)?;
    assert_eq!(image.active_area, Some(Rect::new(Point::new(3, 2), Dim2::new(3, 2))));
    // DefaultCrop moves with the trimmed image and is clipped to it
    assert_eq!(image.crop_area, Some(Rect::new(Point::new(4, 3), Dim2::new(2, 1))));
    assert_eq!((image.width, image.height), (8, 6));
    assert_eq!(floats(&image)[2 * 8 + 3], 1.0);
    assert_eq!(floats(&image).iter().filter(|v| **v == 1.0).count(), 1);
    Ok(())
  }

  #[test]
  fn trim_bounds_list3_and_bounds_check() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut image = float_image(4, 4, 3, 0.25);
    single_trim_bounds(1, 1, 3, 3)?.apply(&mut image, OpcodeStage::List3)?;
    assert_eq!(image.active_area, Some(Rect::new(Point::new(1, 1), Dim2::new(2, 2))));
    assert_eq!(floats(&image).len(), 4 * 4 * 3);
    // Rectangle must be inside of the image
    let list = single_trim_bounds(0, 0, 5, 4)?;
    let mut image = float_image(4, 4, 1, 0.25);
    assert!(list.apply(&mut image, OpcodeStage::List1).is_err());
    assert!(list.apply(&mut image, OpcodeStage::List2).is_err());
    Ok(())
  }
}
