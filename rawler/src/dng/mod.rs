// SPDX-License-Identifier: LGPL-2.1
// Copyright 2023 Daniel Vogelbacher <daniel@chaospixel.com>

pub mod convert;
pub mod original;
pub mod writer;

use crate::imgop::Rect;

pub const DNG_VERSION_V1_0: [u8; 4] = [1, 0, 0, 0];
pub const DNG_VERSION_V1_1: [u8; 4] = [1, 1, 0, 0];
pub const DNG_VERSION_V1_2: [u8; 4] = [1, 2, 0, 0];
pub const DNG_VERSION_V1_3: [u8; 4] = [1, 3, 0, 0];
pub const DNG_VERSION_V1_4: [u8; 4] = [1, 4, 0, 0];
pub const DNG_VERSION_V1_5: [u8; 4] = [1, 5, 0, 0];
pub const DNG_VERSION_V1_6: [u8; 4] = [1, 6, 0, 0];

/// Convert internal crop rectangle to DNG active area
///
/// DNG ActiveArea  is:
///  Top, Left, Bottom, Right
pub fn rect_to_dng_area(area: &Rect) -> [u16; 4] {
  [
    area.p.y as u16,
    area.p.x as u16,
    area.p.y as u16 + area.d.h as u16,
    area.p.x as u16 + area.d.w as u16,
  ]
  /*
  [
    image.crops[0] as u16, // top
    image.crops[3] as u16, // left
    //(image.height-image.crops[0]-image.crops[2]) as u16, // bottom
    //(image.width-image.crops[1]-image.crops[3]) as u16, // Right
    (image.height - (image.crops[2])) as u16, // bottom coord
    (image.width - (image.crops[1])) as u16,  // Right coord
  ]
  */
}

#[cfg(feature = "clap")]
impl clap::ValueEnum for DngCompression {
  fn value_variants<'a>() -> &'a [Self] {
    &[Self::Lossless, Self::Uncompressed]
  }

  fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
    Some(match self {
      Self::Uncompressed => clap::builder::PossibleValue::new("uncompressed"),
      Self::Lossless => clap::builder::PossibleValue::new("lossless"),
    })
  }
}

#[derive(Clone, Copy, Debug)]
pub enum DngPhotometricConversion {
  Original,
  Linear,
}

impl Default for DngPhotometricConversion {
  fn default() -> Self {
    Self::Original
  }
}

#[derive(Clone, Copy, Debug)]
pub enum CropMode {
  Best,
  ActiveArea,
  None,
}

#[cfg(feature = "clap")]
impl clap::ValueEnum for CropMode {
  fn value_variants<'a>() -> &'a [Self] {
    &[Self::Best, Self::ActiveArea, Self::None]
  }

  fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
    Some(match self {
      Self::Best => clap::builder::PossibleValue::new("best"),
      Self::ActiveArea => clap::builder::PossibleValue::new("activearea"),
      Self::None => clap::builder::PossibleValue::new("none"),
    })
  }
}

/*
impl FromStr for CropMode {
  type Err = String;

  fn from_str(mode: &str) -> std::result::Result<Self, Self::Err> {
    Ok(match mode {
      "best" => Self::Best,
      "activearea" => Self::ActiveArea,
      "none" => Self::None,
      _ => return Err(format!("Unknown CropMode value: {}", mode)),
    })
  }
}
 */

/// Quality of preview images
const PREVIEW_JPEG_QUALITY: f32 = 0.75;
#[derive(Clone, Copy, Debug)]
/// Compression mode for DNG
pub enum DngCompression {
  /// No compression is applied
  Uncompressed,
  /// Lossless JPEG-92 compression
  Lossless,
  // Lossy
}
