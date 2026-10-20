// SPDX-License-Identifier: LGPL-2.1
// Copyright 2023 Daniel Vogelbacher <daniel@chaospixel.com>

pub mod convert;
pub mod convertprofile;
pub mod opcodes;
pub mod original;
pub mod writer;

use serde::{Deserialize, Serialize};

use crate::{formats::tiff::Value, imgop::Rect};

pub const DNG_VERSION_V1_0: DngVersionNumber = DngVersionNumber([1, 0, 0, 0]);
pub const DNG_VERSION_V1_1: DngVersionNumber = DngVersionNumber([1, 1, 0, 0]);
pub const DNG_VERSION_V1_2: DngVersionNumber = DngVersionNumber([1, 2, 0, 0]);
pub const DNG_VERSION_V1_3: DngVersionNumber = DngVersionNumber([1, 3, 0, 0]);
pub const DNG_VERSION_V1_4: DngVersionNumber = DngVersionNumber([1, 4, 0, 0]);
pub const DNG_VERSION_V1_5: DngVersionNumber = DngVersionNumber([1, 5, 0, 0]);
pub const DNG_VERSION_V1_6: DngVersionNumber = DngVersionNumber([1, 6, 0, 0]);

/// Highest DNG specification version the DNG reader is based on
pub const DNG_READER_VERSION: DngVersionNumber = DngVersionNumber([1, 7, 1, 0]);

/// DNG specification version
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum DngVersion {
  #[serde(rename = "1.0")]
  V1_0,
  #[serde(rename = "1.1")]
  V1_1,
  #[serde(rename = "1.2")]
  V1_2,
  #[serde(rename = "1.3")]
  V1_3,
  #[serde(rename = "1.4")]
  V1_4,
  #[serde(rename = "1.5")]
  V1_5,
  #[serde(rename = "1.6")]
  V1_6,
}

impl DngVersion {
  /// Get the version as DNGVersion/DNGBackwardVersion tag value
  pub fn as_dng_value(&self) -> DngVersionNumber {
    match self {
      Self::V1_0 => DNG_VERSION_V1_0,
      Self::V1_1 => DNG_VERSION_V1_1,
      Self::V1_2 => DNG_VERSION_V1_2,
      Self::V1_3 => DNG_VERSION_V1_3,
      Self::V1_4 => DNG_VERSION_V1_4,
      Self::V1_5 => DNG_VERSION_V1_5,
      Self::V1_6 => DNG_VERSION_V1_6,
    }
  }
}

/// Full 4-byte DNG version number as stored in DNGVersion/DNGBackwardVersion
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, Hash)]
pub struct DngVersionNumber(pub [u8; 4]);

impl DngVersionNumber {
  pub const fn major(&self) -> u8 {
    self.0[0]
  }

  pub const fn minor(&self) -> u8 {
    self.0[1]
  }

  pub const fn revision(&self) -> u8 {
    self.0[2]
  }

  pub const fn build(&self) -> u8 {
    self.0[3]
  }
}

impl From<[u8; 4]> for DngVersionNumber {
  fn from(value: [u8; 4]) -> Self {
    Self(value)
  }
}

impl From<DngVersion> for DngVersionNumber {
  fn from(value: DngVersion) -> Self {
    value.as_dng_value()
  }
}

impl From<DngVersionNumber> for Value {
  fn from(value: DngVersionNumber) -> Self {
    value.0.into()
  }
}

impl std::fmt::Display for DngVersionNumber {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}.{}.{}.{}", self.major(), self.minor(), self.revision(), self.build())
  }
}

#[cfg(feature = "clap")]
impl clap::ValueEnum for DngVersion {
  fn value_variants<'a>() -> &'a [Self] {
    &[Self::V1_0, Self::V1_1, Self::V1_2, Self::V1_3, Self::V1_4, Self::V1_5, Self::V1_6]
  }

  fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
    Some(match self {
      Self::V1_0 => clap::builder::PossibleValue::new("1.0"),
      Self::V1_1 => clap::builder::PossibleValue::new("1.1"),
      Self::V1_2 => clap::builder::PossibleValue::new("1.2"),
      Self::V1_3 => clap::builder::PossibleValue::new("1.3"),
      Self::V1_4 => clap::builder::PossibleValue::new("1.4"),
      Self::V1_5 => clap::builder::PossibleValue::new("1.5"),
      Self::V1_6 => clap::builder::PossibleValue::new("1.6"),
    })
  }
}

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DngPhotometricConversion {
  Original,
  Linear,
}

impl Default for DngPhotometricConversion {
  fn default() -> Self {
    Self::Original
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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

/// DNG opcode lists that are applied to the image data when converting to DNG.
/// Lists that are not applied are written to the DNG.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApplyDngOpcodes {
  /// Apply only opcode lists that can't be stored with the maximum DNGBackwardVersion
  #[default]
  Auto,
  /// Apply OpcodeList1
  List1,
  /// Apply OpcodeList1 and OpcodeList2
  List2,
  /// Apply all opcode lists
  List3,
}

impl ApplyDngOpcodes {
  /// Check if the opcode list of `stage` is applied
  pub fn includes(self, stage: opcodes::OpcodeStage) -> bool {
    let last = match self {
      Self::Auto => return false,
      Self::List1 => opcodes::OpcodeStage::List1,
      Self::List2 => opcodes::OpcodeStage::List2,
      Self::List3 => opcodes::OpcodeStage::List3,
    };
    stage <= last
  }
}

#[cfg(feature = "clap")]
impl clap::ValueEnum for ApplyDngOpcodes {
  fn value_variants<'a>() -> &'a [Self] {
    &[Self::Auto, Self::List1, Self::List2, Self::List3]
  }

  fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
    Some(match self {
      Self::Auto => clap::builder::PossibleValue::new("auto"),
      Self::List1 => clap::builder::PossibleValue::new("list1"),
      Self::List2 => clap::builder::PossibleValue::new("list2"),
      Self::List3 => clap::builder::PossibleValue::new("list3"),
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
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// Compression mode for DNG
pub enum DngCompression {
  /// No compression is applied
  Uncompressed,
  /// Lossless JPEG-92 compression
  Lossless,
  // Lossy
}
