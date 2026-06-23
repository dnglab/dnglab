// SPDX-License-Identifier: LGPL-2.1
// Copyright 2023 Daniel Vogelbacher <daniel@chaospixel.com>

pub mod convert;
pub(crate) mod jxl_encoder;
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
pub const DNG_VERSION_V1_7_0: [u8; 4] = [1, 7, 0, 0];
pub const DNG_VERSION_V1_7_1: [u8; 4] = [1, 7, 1, 0];

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
    &[
      Self::Lossless,
      Self::Uncompressed,
      Self::JxlLossy {
        distance: 1.0,
        effort: 7,
        decode_speed: None,
      },
    ]
  }

  fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
    Some(match self {
      Self::Uncompressed => clap::builder::PossibleValue::new("uncompressed"),
      Self::Lossless => clap::builder::PossibleValue::new("lossless"),
      Self::JxlLossy { .. } => clap::builder::PossibleValue::new("jpegxl-lossy"),
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

/// Compression mode for DNG
#[derive(Clone, Copy, Debug)]
pub enum DngCompression {
  /// No compression is applied
  Uncompressed,
  /// Lossless JPEG-92 compression
  Lossless,
  /// JPEG XL lossy compression (DNG 1.7.0+).
  ///
  /// `distance` is the JXL butterfly distance: 0.0 = mathematically lossless,
  /// 1.0 ≈ visually lossless, higher values increase compression at the cost of
  /// quality (max ≈ 15).  `effort` controls encoder effort (1 = fastest … 9 =
  /// best quality, default 7).  `decode_speed` is the optional JXL decode-speed
  /// hint (1 = slowest/best quality … 4 = fastest).
  JxlLossy {
    distance: f32,
    effort: u32,
    decode_speed: Option<u32>,
  },
}

impl PartialEq for DngCompression {
  fn eq(&self, other: &Self) -> bool {
    match (self, other) {
      (Self::Uncompressed, Self::Uncompressed) => true,
      (Self::Lossless, Self::Lossless) => true,
      (
        Self::JxlLossy { distance: d1, effort: e1, decode_speed: ds1 },
        Self::JxlLossy { distance: d2, effort: e2, decode_speed: ds2 },
      ) => d1.to_bits() == d2.to_bits() && e1 == e2 && ds1 == ds2,
      _ => false,
    }
  }
}
impl Eq for DngCompression {}
