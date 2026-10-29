// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! DNG opcode lists (DNG specification, chapter 7 "Opcode List Processing")

mod delta_per_column;
mod delta_per_row;
mod fix_bad_pixels_constant;
mod fix_bad_pixels_list;
mod fix_vignette_radial;
mod gain_map;
mod helpers;
mod map_polynomial;
mod map_table;
mod scale_per_column;
mod scale_per_row;
#[cfg(test)]
mod testutil;
mod trim_bounds;
mod warp_fisheye;
mod warp_rectilinear;
mod warp_rectilinear2;

pub use self::delta_per_column::DeltaPerColumn;
pub use self::delta_per_row::DeltaPerRow;
pub use self::fix_bad_pixels_constant::FixBadPixelsConstant;
pub use self::fix_bad_pixels_list::FixBadPixelsList;
pub use self::fix_vignette_radial::FixVignetteRadial;
pub use self::gain_map::GainMap;
pub use self::helpers::AreaSpec;
pub use self::map_polynomial::MapPolynomial;
pub use self::map_table::MapTable;
pub use self::scale_per_column::ScalePerColumn;
pub use self::scale_per_row::ScalePerRow;
pub use self::trim_bounds::TrimBounds;
pub use self::warp_fisheye::WarpFisheye;
pub use self::warp_rectilinear::{WarpRectilinear, WarpRectilinearCoeffs};
pub use self::warp_rectilinear2::{WarpRectilinear2, WarpRectilinear2Coeffs};

use serde::{Deserialize, Serialize};

use crate::{
  RawImage, RawImageData, RawlerError, Result,
  dng::{DNG_READER_VERSION, DngVersionNumber},
  imgop::Rect,
  pixarray::ImageViewMut,
};

use self::helpers::{ParamReader, put_u32};

/// Image data that DNG opcodes are applied to
#[derive(Debug)]
pub enum OpcodeData<'a> {
  /// 16 bit data as stored in the file, for OpcodeList1
  Integer(ImageViewMut<'a, u16>),
  /// Floating point data
  Float(ImageViewMut<'a, f32>),
}

impl<'a> OpcodeData<'a> {
  /// Crop to `area` without a new buffer, see [`ImageViewMut::crop_in_place`]
  fn crop_in_place(self, area: Rect) -> Result<Self> {
    Ok(match self {
      Self::Integer(view) => Self::Integer(view.crop_in_place(area)?),
      Self::Float(view) => Self::Float(view.crop_in_place(area)?),
    })
  }
}

/// Image data with the metadata that DNG opcodes use and change
///
/// Opcodes work in place on the borrowed image data. TrimBounds in
/// OpcodeList1 crops the data to the start of the buffer, the owner of the
/// buffer can truncate it to the size of the image after the opcodes.
#[derive(Debug)]
pub struct OpcodeImage<'a> {
  pub data: OpcodeData<'a>,
  /// ActiveArea, relative to the image data
  pub active_area: Option<Rect>,
  /// DefaultCrop, relative to the image data
  pub crop_area: Option<Rect>,
  /// Masked areas, relative to the image data
  pub black_areas: Vec<Rect>,
  /// Floating point data is mapped from 16 bit data: deltas of opcodes are in
  /// units of 1/65535
  pub scaled_from_integer: bool,
  /// Pixel aspect ratio (width / height), for warps
  pub aspect_ratio: f64,
}

impl OpcodeImage<'_> {
  pub fn width(&self) -> usize {
    match &self.data {
      OpcodeData::Integer(view) => view.width(),
      OpcodeData::Float(view) => view.width(),
    }
  }

  pub fn height(&self) -> usize {
    match &self.data {
      OpcodeData::Integer(view) => view.height(),
      OpcodeData::Float(view) => view.height(),
    }
  }

  pub fn channels(&self) -> usize {
    match &self.data {
      OpcodeData::Integer(view) => view.channels(),
      OpcodeData::Float(view) => view.channels(),
    }
  }
}

/// Opcode flag bit 0: the opcode is optional, a reader may skip it
const FLAG_OPTIONAL: u32 = 1;

#[derive(Debug, Copy, Clone, PartialEq, enumn::N)]
#[repr(u32)]
pub enum OpcodeID {
  WarpRectilinear = 1,
  WarpFisheye = 2,
  FixVignetteRadial = 3,
  FixBadPixelsConstant = 4,
  FixBadPixelsList = 5,
  TrimBounds = 6,
  MapTable = 7,
  MapPolynomial = 8,
  GainMap = 9,
  DeltaPerRow = 10,
  DeltaPerColumn = 11,
  ScalePerRow = 12,
  ScalePerColumn = 13,
  WarpRectilinear2 = 14,
}

/// Processing stage of an opcode list
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OpcodeStage {
  /// OpcodeList1: applied to the raw image as read from the file
  List1,
  /// OpcodeList2: applied after mapping to linear reference values
  List2,
  /// OpcodeList3: applied after demosaicing
  List3,
}

impl std::fmt::Display for OpcodeStage {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::List1 => f.write_str("OpcodeList1"),
      Self::List2 => f.write_str("OpcodeList2"),
      Self::List3 => f.write_str("OpcodeList3"),
    }
  }
}

/// Selects opcodes by ID, in all opcode lists or in the list of a single stage.
///
/// The string form is `[<list>:]<opcode>`, where `<list>` is 1, 2 or 3 and
/// `<opcode>` is an opcode name (case-insensitive) or a decimal opcode ID,
/// e.g. `WarpRectilinear`, `2:GainMap` or `3:51966`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OpcodeSelector {
  /// Opcode list, `None` for all lists
  pub stage: Option<OpcodeStage>,
  /// Opcode ID
  pub id: u32,
}

impl OpcodeSelector {
  /// Check if `opcode` in the opcode list of `stage` is selected
  pub fn matches(&self, stage: OpcodeStage, opcode: &Opcode) -> bool {
    self.stage.is_none_or(|selected| selected == stage) && self.id == opcode.id()
  }
}

impl std::str::FromStr for OpcodeSelector {
  type Err = String;

  fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
    let (stage, opcode) = match value.split_once(':') {
      Some((list, opcode)) => {
        let stage = match list.trim() {
          "1" => OpcodeStage::List1,
          "2" => OpcodeStage::List2,
          "3" => OpcodeStage::List3,
          _ => return Err(format!("Invalid opcode list '{}' in '{}', expected 1, 2 or 3", list, value)),
        };
        (Some(stage), opcode.trim())
      }
      None => (None, value.trim()),
    };
    let id = match opcode.parse::<u32>() {
      Ok(id) => id,
      Err(_) => (1..=u32::MAX)
        .map_while(OpcodeID::n)
        .find(|id| format!("{:?}", id).eq_ignore_ascii_case(opcode))
        .map(|id| id as u32)
        .ok_or_else(|| {
          let names: Vec<String> = (1..=u32::MAX).map_while(OpcodeID::n).map(|id| format!("{:?}", id)).collect();
          format!("Unknown DNG opcode '{}', expected an opcode ID or one of: {}", opcode, names.join(", "))
        })?,
    };
    Ok(Self { stage, id })
  }
}

impl std::fmt::Display for OpcodeSelector {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self.stage {
      Some(OpcodeStage::List1) => f.write_str("1:")?,
      Some(OpcodeStage::List2) => f.write_str("2:")?,
      Some(OpcodeStage::List3) => f.write_str("3:")?,
      None => {}
    }
    match OpcodeID::n(self.id) {
      Some(id) => write!(f, "{:?}", id),
      None => write!(f, "{}", self.id),
    }
  }
}

impl Serialize for OpcodeSelector {
  fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
    serializer.collect_str(self)
  }
}

impl<'de> Deserialize<'de> for OpcodeSelector {
  fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
    String::deserialize(deserializer)?.parse().map_err(serde::de::Error::custom)
  }
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum Opcode {
  WarpRectilinear(WarpRectilinear),
  WarpFisheye(WarpFisheye),
  FixVignetteRadial(FixVignetteRadial),
  FixBadPixelsConstant(FixBadPixelsConstant),
  FixBadPixelsList(FixBadPixelsList),
  TrimBounds(TrimBounds),
  MapTable(MapTable),
  MapPolynomial(MapPolynomial),
  GainMap(GainMap),
  DeltaPerRow(DeltaPerRow),
  DeltaPerColumn(DeltaPerColumn),
  ScalePerRow(ScalePerRow),
  ScalePerColumn(ScalePerColumn),
  WarpRectilinear2(WarpRectilinear2),
  Unknown(UnknownOpcode),
}

impl Opcode {
  fn parse(id: u32, version: u32, flags: u32, params: &[u8]) -> Result<Self> {
    let mut reader = ParamReader::new(params);
    let parsed = match OpcodeID::n(id) {
      Some(OpcodeID::WarpRectilinear) => WarpRectilinear::parse(&mut reader, version, flags).map(Self::WarpRectilinear),
      Some(OpcodeID::WarpFisheye) => WarpFisheye::parse(&mut reader, version, flags).map(Self::WarpFisheye),
      Some(OpcodeID::FixBadPixelsConstant) => FixBadPixelsConstant::parse(&mut reader, version, flags).map(Self::FixBadPixelsConstant),
      Some(OpcodeID::FixVignetteRadial) => FixVignetteRadial::parse(&mut reader, version, flags).map(Self::FixVignetteRadial),
      Some(OpcodeID::FixBadPixelsList) => FixBadPixelsList::parse(&mut reader, version, flags).map(Self::FixBadPixelsList),
      Some(OpcodeID::TrimBounds) => TrimBounds::parse(&mut reader, version, flags).map(Self::TrimBounds),
      Some(OpcodeID::MapTable) => MapTable::parse(&mut reader, version, flags).map(Self::MapTable),
      Some(OpcodeID::MapPolynomial) => MapPolynomial::parse(&mut reader, version, flags).map(Self::MapPolynomial),
      Some(OpcodeID::GainMap) => GainMap::parse(&mut reader, version, flags).map(Self::GainMap),
      Some(OpcodeID::DeltaPerRow) => DeltaPerRow::parse(&mut reader, version, flags).map(Self::DeltaPerRow),
      Some(OpcodeID::DeltaPerColumn) => DeltaPerColumn::parse(&mut reader, version, flags).map(Self::DeltaPerColumn),
      Some(OpcodeID::ScalePerRow) => ScalePerRow::parse(&mut reader, version, flags).map(Self::ScalePerRow),
      Some(OpcodeID::ScalePerColumn) => ScalePerColumn::parse(&mut reader, version, flags).map(Self::ScalePerColumn),
      Some(OpcodeID::WarpRectilinear2) => WarpRectilinear2::parse(&mut reader, version, flags).map(Self::WarpRectilinear2),
      None => {
        log::warn!("Unknown DNG opcode id: {}", id);
        return Ok(Self::Unknown(UnknownOpcode::new(id, version, flags, params)));
      }
    };
    let parsed = parsed.and_then(|opcode| {
      if reader.remaining() != 0 {
        return Err(RawlerError::DecoderFailed(format!(
          "DNG opcode {}: parameter area has {} bytes, but only {} are used",
          opcode.name(),
          params.len(),
          params.len() - reader.remaining()
        )));
      }
      Ok(opcode)
    });
    match parsed {
      Ok(opcode) => Ok(opcode),
      // Optional opcodes may be skipped by readers, so keep a malformed one as raw data
      Err(err) if flags & FLAG_OPTIONAL != 0 => {
        log::warn!("Failed to parse optional DNG opcode id {}, keep it as raw data: {}", id, err);
        Ok(Self::Unknown(UnknownOpcode::new(id, version, flags, params)))
      }
      Err(err) => Err(err),
    }
  }

  /// Opcode ID as stored in the opcode list
  pub fn id(&self) -> u32 {
    match self {
      Self::WarpRectilinear(_) => OpcodeID::WarpRectilinear as u32,
      Self::WarpFisheye(_) => OpcodeID::WarpFisheye as u32,
      Self::FixVignetteRadial(_) => OpcodeID::FixVignetteRadial as u32,
      Self::FixBadPixelsConstant(_) => OpcodeID::FixBadPixelsConstant as u32,
      Self::FixBadPixelsList(_) => OpcodeID::FixBadPixelsList as u32,
      Self::TrimBounds(_) => OpcodeID::TrimBounds as u32,
      Self::MapTable(_) => OpcodeID::MapTable as u32,
      Self::MapPolynomial(_) => OpcodeID::MapPolynomial as u32,
      Self::GainMap(_) => OpcodeID::GainMap as u32,
      Self::DeltaPerRow(_) => OpcodeID::DeltaPerRow as u32,
      Self::DeltaPerColumn(_) => OpcodeID::DeltaPerColumn as u32,
      Self::ScalePerRow(_) => OpcodeID::ScalePerRow as u32,
      Self::ScalePerColumn(_) => OpcodeID::ScalePerColumn as u32,
      Self::WarpRectilinear2(_) => OpcodeID::WarpRectilinear2 as u32,
      Self::Unknown(op) => op.id,
    }
  }

  /// Opcode name for messages
  pub fn name(&self) -> String {
    match OpcodeID::n(self.id()) {
      Some(id) => format!("{:?}", id),
      None => format!("Unknown({})", self.id()),
    }
  }

  /// Raw version and flags fields
  fn header(&self) -> (u32, u32) {
    match self {
      Self::WarpRectilinear(op) => (op.version, op.flags),
      Self::WarpFisheye(op) => (op.version, op.flags),
      Self::FixVignetteRadial(op) => (op.version, op.flags),
      Self::FixBadPixelsConstant(op) => (op.version, op.flags),
      Self::FixBadPixelsList(op) => (op.version, op.flags),
      Self::TrimBounds(op) => (op.version, op.flags),
      Self::MapTable(op) => (op.version, op.flags),
      Self::MapPolynomial(op) => (op.version, op.flags),
      Self::GainMap(op) => (op.version, op.flags),
      Self::DeltaPerRow(op) => (op.version, op.flags),
      Self::DeltaPerColumn(op) => (op.version, op.flags),
      Self::ScalePerRow(op) => (op.version, op.flags),
      Self::ScalePerColumn(op) => (op.version, op.flags),
      Self::WarpRectilinear2(op) => (op.version, op.flags),
      Self::Unknown(op) => (op.version, op.flags),
    }
  }

  /// DNG specification version in which the opcode was defined
  pub fn version(&self) -> DngVersionNumber {
    DngVersionNumber::from(self.header().0.to_be_bytes())
  }

  pub fn flags(&self) -> u32 {
    self.header().1
  }

  /// Optional opcodes may be skipped by a DNG reader
  pub fn is_optional(&self) -> bool {
    self.flags() & FLAG_OPTIONAL != 0
  }

  /// Check if apply() is implemented for this opcode
  pub fn is_supported(&self) -> bool {
    !matches!(self, Self::Unknown(_))
  }

  pub fn apply(&self, image: &mut OpcodeImage<'_>, stage: OpcodeStage) -> Result<()> {
    match self {
      Self::WarpRectilinear(warp_rectilinear) => warp_rectilinear.apply(image, stage),
      Self::WarpFisheye(warp_fisheye) => warp_fisheye.apply(image, stage),
      Self::WarpRectilinear2(warp_rectilinear2) => warp_rectilinear2.apply(image, stage),
      Self::FixVignetteRadial(fix_vignette) => fix_vignette.apply(image, stage),
      Self::FixBadPixelsConstant(fix_bad_pixels) => fix_bad_pixels.apply(image, stage),
      Self::FixBadPixelsList(fix_bad_pixels) => fix_bad_pixels.apply(image, stage),
      Self::TrimBounds(trim_bounds) => trim_bounds.apply(image, stage),
      Self::MapTable(map_table) => map_table.apply(image, stage),
      Self::MapPolynomial(map_polynomial) => map_polynomial.apply(image, stage),
      Self::GainMap(gain_map) => gain_map.apply(image, stage),
      Self::DeltaPerRow(delta_per_row) => delta_per_row.apply(image, stage),
      Self::DeltaPerColumn(delta_per_column) => delta_per_column.apply(image, stage),
      Self::ScalePerRow(scale_per_row) => scale_per_row.apply(image, stage),
      Self::ScalePerColumn(scale_per_column) => scale_per_column.apply(image, stage),
      Self::Unknown(_) => Err(RawlerError::DecoderFailed(format!("DNG opcode {} is not supported", self.name()))),
    }
  }

  fn write_params(&self, buf: &mut Vec<u8>) -> Result<()> {
    match self {
      Self::WarpRectilinear(op) => op.write_params(buf),
      Self::WarpFisheye(op) => op.write_params(buf),
      Self::FixVignetteRadial(op) => op.write_params(buf),
      Self::FixBadPixelsConstant(op) => op.write_params(buf),
      Self::FixBadPixelsList(op) => op.write_params(buf),
      Self::TrimBounds(op) => op.write_params(buf),
      Self::MapTable(op) => op.write_params(buf),
      Self::MapPolynomial(op) => op.write_params(buf)?,
      Self::GainMap(op) => op.write_params(buf),
      Self::DeltaPerRow(op) => op.write_params(buf),
      Self::DeltaPerColumn(op) => op.write_params(buf),
      Self::ScalePerRow(op) => op.write_params(buf),
      Self::ScalePerColumn(op) => op.write_params(buf),
      Self::WarpRectilinear2(op) => op.write_params(buf),
      Self::Unknown(op) => buf.extend_from_slice(&op.data),
    }
    Ok(())
  }
}

#[derive(Debug, Clone, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct OpcodeList(Vec<Opcode>);

impl OpcodeList {
  pub fn new(opcodes: Vec<Opcode>) -> Self {
    Self(opcodes)
  }

  pub fn opcodes(&self) -> &[Opcode] {
    self.0.as_slice()
  }

  pub fn is_empty(&self) -> bool {
    self.0.is_empty()
  }

  /// Parse an opcode list. Opcode lists are always stored in big-endian byte order.
  pub fn parse(data: &[u8]) -> Result<Self> {
    let mut reader = ParamReader::new(data);
    let count = reader.u32()?;
    let mut opcodes = Vec::new();
    for _ in 0..count {
      let id = reader.u32()?;
      let version = reader.u32()?;
      let flags = reader.u32()?;
      let len = reader.u32()? as usize;
      let params = reader.take(len)?;
      opcodes.push(Opcode::parse(id, version, flags, params)?);
    }
    Ok(Self(opcodes))
  }

  /// Serialize the opcode list (big-endian)
  pub fn to_bytes(&self) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    put_u32(&mut buf, self.0.len() as u32);
    for opcode in &self.0 {
      let mut params = Vec::new();
      opcode.write_params(&mut params)?;
      let (version, flags) = opcode.header();
      put_u32(&mut buf, opcode.id());
      put_u32(&mut buf, version);
      put_u32(&mut buf, flags);
      put_u32(&mut buf, params.len() as u32);
      buf.extend_from_slice(&params);
    }
    Ok(buf)
  }

  /// Check if the list contains a required opcode that needs a newer DNG reader
  /// than `max_backward_version`. Such a list can't be stored in a DNG with this
  /// DNGBackwardVersion and must be applied to the image data instead.
  pub fn needs_processing(&self, max_backward_version: DngVersionNumber) -> bool {
    self.0.iter().any(|opcode| !opcode.is_optional() && opcode.version() > max_backward_version)
  }

  /// Remove the opcodes selected by `selectors` from this list of `stage`,
  /// returns the removed opcodes
  pub fn remove(&mut self, stage: OpcodeStage, selectors: &[OpcodeSelector]) -> Vec<Opcode> {
    let (removed, kept) = std::mem::take(&mut self.0)
      .into_iter()
      .partition(|opcode| selectors.iter().any(|selector| selector.matches(stage, opcode)));
    self.0 = kept;
    removed
  }

  /// Multiply the values of DeltaPerRow and DeltaPerColumn by `factor`. Their
  /// deltas are in pixel units of the image data, so they change with its
  /// sample format.
  pub fn scale_deltas(&mut self, factor: f32) {
    for opcode in &mut self.0 {
      let deltas = match opcode {
        Opcode::DeltaPerRow(op) => &mut op.deltas,
        Opcode::DeltaPerColumn(op) => &mut op.deltas,
        _ => continue,
      };
      // Keep the values finite, as required by the parser
      deltas.iter_mut().for_each(|delta| *delta = (*delta * factor).clamp(f32::MIN, f32::MAX));
    }
  }

  /// Apply all opcodes to the image data of `rawimage`, see [`Self::apply_to`]
  pub fn apply(&self, rawimage: &mut RawImage, stage: OpcodeStage) -> Result<()> {
    let (width, height, cpp) = (rawimage.width, rawimage.height, rawimage.cpp);
    let mut image = OpcodeImage {
      data: match &mut rawimage.data {
        RawImageData::Integer(samples) => OpcodeData::Integer(ImageViewMut::new(samples, width, height, cpp)),
        RawImageData::Float(samples) => OpcodeData::Float(ImageViewMut::new(samples, width, height, cpp)),
      },
      active_area: rawimage.active_area,
      crop_area: rawimage.crop_area,
      black_areas: rawimage.blackareas.clone(),
      scaled_from_integer: rawimage.scaled_from_integer,
      aspect_ratio: rawimage.camera.default_scale.pixel_aspect_ratio(),
    };
    let result = self.apply_to(&mut image, stage);
    let (width, height) = (image.width(), image.height());
    let OpcodeImage {
      active_area,
      crop_area,
      black_areas,
      ..
    } = image;
    // TrimBounds may have cropped the image data
    match &mut rawimage.data {
      RawImageData::Integer(samples) => samples.truncate(width * height * cpp),
      RawImageData::Float(samples) => samples.truncate(width * height * cpp),
    }
    (rawimage.width, rawimage.height) = (width, height);
    (rawimage.active_area, rawimage.crop_area, rawimage.blackareas) = (active_area, crop_area, black_areas);
    result
  }

  /// Apply all opcodes in sequence to `image`. Unsupported optional opcodes are
  /// skipped, unsupported required opcodes return an error.
  pub fn apply_to(&self, image: &mut OpcodeImage<'_>, stage: OpcodeStage) -> Result<()> {
    // DNG spec "skip rule": a WarpRectilinear or WarpFisheye immediately following
    // an optional WarpRectilinear2 is a fallback for older DNG readers.
    let mut skip_fallback_warp = false;
    for opcode in &self.0 {
      if std::mem::take(&mut skip_fallback_warp) && matches!(OpcodeID::n(opcode.id()), Some(OpcodeID::WarpRectilinear | OpcodeID::WarpFisheye)) {
        log::debug!("Skip DNG opcode {} in {}: fallback for optional WarpRectilinear2", opcode.name(), stage);
        continue;
      }
      let reason = if opcode.version() > DNG_READER_VERSION {
        Some(format!("requires DNG reader version {}", opcode.version()))
      } else if !opcode.is_supported() {
        Some("is not supported".to_string())
      } else {
        None
      };
      match reason {
        None => {
          log::debug!("Apply {}: {:?}", stage, opcode);
          opcode.apply(image, stage)?;
          skip_fallback_warp = opcode.is_optional() && matches!(opcode, Opcode::WarpRectilinear2(_));
        }
        Some(reason) if opcode.is_optional() => {
          log::debug!("Skip optional DNG opcode {} in {}: opcode {}", opcode.name(), stage, reason);
        }
        Some(reason) => {
          return Err(RawlerError::DecoderFailed(format!(
            "Required DNG opcode {} (DNG {}) in {} {}",
            opcode.name(),
            opcode.version(),
            stage,
            reason
          )));
        }
      }
    }
    Ok(())
  }
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct UnknownOpcode {
  pub id: u32,
  pub version: u32,
  pub flags: u32,
  pub data: Vec<u8>,
}

impl UnknownOpcode {
  fn new(id: u32, version: u32, flags: u32, data: &[u8]) -> Self {
    Self {
      id,
      version,
      flags,
      data: data.to_vec(),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::dng::opcodes::{
    helpers::{put_f32, put_f64},
    testutil::*,
  };

  /// `data` without ActiveArea, DefaultCrop and masked areas
  fn opcode_image(data: OpcodeData<'_>) -> OpcodeImage<'_> {
    OpcodeImage {
      data,
      active_area: None,
      crop_area: None,
      black_areas: Vec::new(),
      scaled_from_integer: false,
      aspect_ratio: 1.0,
    }
  }

  #[test]
  fn apply_to_plain_buffers() -> Result<()> {
    // MapPolynomial doubles the values of floating point data
    let polynomial = map_polynomial([0, 0, 2, 3, 0, 1, 1, 1], &[0.0, 2.0]);
    let list = OpcodeList::parse(&opcode_list(&[opcode(OpcodeID::MapPolynomial as u32, V1_3, 0, &polynomial)]))?;
    let mut samples = vec![0.25_f32; 6];
    list.apply_to(
      &mut opcode_image(OpcodeData::Float(ImageViewMut::new(&mut samples, 3, 2, 1))),
      OpcodeStage::List2,
    )?;
    assert_eq!(samples, [0.5; 6]);

    // TrimBounds crops integer data of OpcodeList1 to the start of the buffer
    let mut bounds = Vec::new();
    [0, 1, 2, 3].into_iter().for_each(|v| put_u32(&mut bounds, v)); // Top, left, bottom, right
    let list = OpcodeList::parse(&opcode_list(&[opcode(OpcodeID::TrimBounds as u32, V1_3, 0, &bounds)]))?;
    let mut samples: Vec<u16> = (0..12).collect();
    let mut image = opcode_image(OpcodeData::Integer(ImageViewMut::new(&mut samples, 4, 3, 1)));
    list.apply_to(&mut image, OpcodeStage::List1)?;
    assert_eq!((image.width(), image.height()), (2, 2));
    assert_eq!(samples[..4], [1, 2, 5, 6]);
    Ok(())
  }

  #[test]
  fn parse_and_serialize_map_polynomial_list() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Same structure as in a lossy JPEG XL DNG written by Adobe
    let coeffs = [
      [0.0, 0.013606967336167251, 0.0, 0.20410451004250876],
      [0.0001422227840900107, 0.024427750825682274, 0.0, 0.3664162623852341],
      [0.0, 0.015184257518054392, 0.0, 0.2277638627708159],
    ];
    let data = opcode_list(
      &(0..3)
        .map(|plane| opcode(8, V1_3, 0, &map_polynomial([0, 0, 5464, 8192, plane, 1, 1, 1], &coeffs[plane as usize])))
        .collect::<Vec<_>>(),
    );
    assert_eq!(data.len(), 256);
    let list = OpcodeList::parse(&data)?;
    assert_eq!(list.opcodes().len(), 3);
    for (plane, opcode) in list.opcodes().iter().enumerate() {
      let Opcode::MapPolynomial(op) = opcode else {
        panic!("expected MapPolynomial");
      };
      assert_eq!(opcode.version(), DngVersionNumber::from([1, 3, 0, 0]));
      assert!(!opcode.is_optional());
      assert_eq!((op.area.top, op.area.left, op.area.bottom, op.area.right), (0, 0, 5464, 8192));
      assert_eq!((op.area.plane, op.area.planes, op.area.row_pitch, op.area.col_pitch), (plane as u32, 1, 1, 1));
      assert_eq!(op.coeffs, coeffs[plane]);
    }
    assert_eq!(list.to_bytes()?, data);
    Ok(())
  }

  #[test]
  fn serialize_unknown_and_gain_map() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut gain_map = Vec::new();
    [0, 0, 4, 4, 0, 1, 2, 2, 2, 2].iter().for_each(|v| put_u32(&mut gain_map, *v));
    [0.5, 0.5, 0.25, 0.25].iter().for_each(|v| put_f64(&mut gain_map, *v));
    put_u32(&mut gain_map, 1);
    [1.0, 1.5, 2.0, 2.5].iter().for_each(|v| put_f32(&mut gain_map, *v));
    let data = opcode_list(&[
      opcode(9, V1_3, 1, &gain_map),
      opcode(99, 0x01070000, 1, &[1, 2, 3]),
      // Malformed optional opcode is kept as raw data
      opcode(13, V1_3, 1, &[0; 12]),
    ]);
    let list = OpcodeList::parse(&data)?;
    assert!(matches!(&list.opcodes()[0], Opcode::GainMap(op) if op.map_planes == 1 && op.gain_map == [1.0, 1.5, 2.0, 2.5]));
    assert!(matches!(&list.opcodes()[1], Opcode::Unknown(op) if op.id == 99));
    assert!(matches!(&list.opcodes()[2], Opcode::Unknown(op) if op.id == 13));
    assert_eq!(list.opcodes()[2].name(), "ScalePerColumn");
    assert_eq!(list.to_bytes()?, data);
    Ok(())
  }

  #[test]
  fn parse_respects_count() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut data = opcode_list(&[opcode(8, V1_3, 0, &map_polynomial([0, 0, 1, 1, 0, 1, 1, 1], &[0.0, 1.0]))]);
    data.extend_from_slice(&[0xFF; 7]);
    assert_eq!(OpcodeList::parse(&data)?.opcodes().len(), 1);
    assert!(OpcodeList::parse(&[0, 0, 0]).is_err());
    Ok(())
  }

  #[test]
  fn parse_malformed_data_fails() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let valid = map_polynomial([0, 0, 4, 4, 0, 1, 1, 1], &[0.0, 1.0]);
    // Count larger than number of opcodes
    let mut data = opcode_list(&[opcode(8, V1_3, 0, &valid)]);
    data[3] = 2;
    assert!(OpcodeList::parse(&data).is_err());
    // Parameter area larger than the opcode list
    let mut data = opcode_list(&[opcode(8, V1_3, 0, &valid)]);
    data[19] += 1;
    assert!(OpcodeList::parse(&data).is_err());
    // Unused bytes in parameter area
    let mut params = valid.clone();
    params.extend_from_slice(&[0; 4]);
    assert!(OpcodeList::parse(&opcode_list(&[opcode(8, V1_3, 0, &params)])).is_err());
    // Degree above 8
    let params = map_polynomial([0, 0, 4, 4, 0, 1, 1, 1], &[0.0; 10]);
    assert!(OpcodeList::parse(&opcode_list(&[opcode(8, V1_3, 0, &params)])).is_err());
    // Huge degree must not allocate
    let mut params = valid.clone();
    params[32..36].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(OpcodeList::parse(&opcode_list(&[opcode(8, V1_3, 0, &params)])).is_err());
    // Huge GainMap must not allocate
    let mut gain_map = Vec::new();
    [0, 0, 4, 4, 0, 1, 1, 1, u32::MAX, u32::MAX].iter().for_each(|v| put_u32(&mut gain_map, *v));
    [1.0; 4].iter().for_each(|v| put_f64(&mut gain_map, *v));
    put_u32(&mut gain_map, u32::MAX);
    assert!(OpcodeList::parse(&opcode_list(&[opcode(9, V1_3, 0, &gain_map)])).is_err());
    // Invalid areas: zero row pitch, zero planes, pitch larger than area, pitch on empty area
    for area in [
      [0, 0, 4, 4, 0, 1, 0, 1],
      [0, 0, 4, 4, 0, 0, 1, 1],
      [0, 0, 4, 4, 0, 1, 5, 1],
      [0, 0, 0, 0, 0, 1, 2, 2],
    ] {
      let params = map_polynomial(area, &[0.0, 1.0]);
      assert!(OpcodeList::parse(&opcode_list(&[opcode(8, V1_3, 0, &params)])).is_err(), "{:?}", area);
    }
    Ok(())
  }

  #[test]
  fn malformed_optional_opcode_is_kept_as_raw_data() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let params = map_polynomial([0, 0, 4, 4, 0, 1, 1, 1], &[0.0; 10]);
    let data = opcode_list(&[opcode(8, V1_3, 1, &params)]);
    let list = OpcodeList::parse(&data)?;
    assert!(matches!(&list.opcodes()[0], Opcode::Unknown(op) if op.id == 8));
    assert_eq!(list.to_bytes()?, data);
    Ok(())
  }

  #[test]
  fn needs_processing() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let required = OpcodeList::parse(&opcode_list(&[opcode(8, V1_3, 0, &map_polynomial([0, 0, 1, 1, 0, 1, 1, 1], &[0.0]))]))?;
    let optional = OpcodeList::parse(&opcode_list(&[opcode(8, V1_3, 1, &map_polynomial([0, 0, 1, 1, 0, 1, 1, 1], &[0.0]))]))?;
    assert!(required.needs_processing(DngVersionNumber::from([1, 2, 0, 0])));
    assert!(!required.needs_processing(DngVersionNumber::from([1, 3, 0, 0])));
    assert!(!optional.needs_processing(DngVersionNumber::from([1, 2, 0, 0])));
    Ok(())
  }

  #[test]
  fn unsupported_opcodes() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut image = float_image(2, 2, 1, 0.25);
    let required = OpcodeList::parse(&opcode_list(&[opcode(99, V1_3, 0, &[])]))?;
    assert!(required.apply(&mut image, OpcodeStage::List2).is_err());
    // Optional opcodes are skipped, also if newer than the reader
    let newer = 0x02000000;
    let optional = OpcodeList::parse(&opcode_list(&[
      opcode(99, V1_3, 1, &[]),
      opcode(8, newer, 1, &map_polynomial([0, 0, 2, 2, 0, 1, 1, 1], &[0.0])),
      opcode(8, V1_3, 1, &map_polynomial([0, 0, 1, 1, 0, 1, 1, 1], &[1.0])),
    ]))?;
    optional.apply(&mut image, OpcodeStage::List2)?;
    assert_eq!(floats(&image), [1.0, 0.25, 0.25, 0.25]);
    let too_new = OpcodeList::parse(&opcode_list(&[opcode(8, newer, 0, &map_polynomial([0, 0, 2, 2, 0, 1, 1, 1], &[0.0]))]))?;
    assert!(too_new.apply(&mut image, OpcodeStage::List2).is_err());
    Ok(())
  }

  #[test]
  fn scale_deltas() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let area = [0, 0, 2, 2, 0, 1, 1, 1];
    let mut list = OpcodeList::parse(&opcode_list(&[
      opcode(10, V1_3, 0, &line_table(area, &[1000.0, -3.0e38])),
      opcode(11, V1_3, 0, &line_table(area, &[3.0e38, 0.5])),
      opcode(12, V1_3, 0, &line_table(area, &[2.0, 4.0])),
    ]))?;
    list.scale_deltas(2.0);
    assert!(matches!(&list.opcodes()[0], Opcode::DeltaPerRow(op) if op.deltas == [2000.0, f32::MIN]));
    assert!(matches!(&list.opcodes()[1], Opcode::DeltaPerColumn(op) if op.deltas == [f32::MAX, 1.0]));
    assert!(matches!(&list.opcodes()[2], Opcode::ScalePerRow(op) if op.scales == [2.0, 4.0]));
    // Scaled values are serialized and parsed again
    assert_eq!(OpcodeList::parse(&list.to_bytes()?)?, list);
    Ok(())
  }

  #[test]
  fn opcode_selector_parse_and_display() -> std::result::Result<(), Box<dyn std::error::Error>> {
    for (value, stage, id, display) in [
      ("WarpRectilinear", None, 1, "WarpRectilinear"),
      ("warprectilinear", None, 1, "WarpRectilinear"),
      ("14", None, 14, "WarpRectilinear2"),
      ("51966", None, 51966, "51966"),
      ("2:GainMap", Some(OpcodeStage::List2), 9, "2:GainMap"),
      ("3:1", Some(OpcodeStage::List3), 1, "3:WarpRectilinear"),
      ("1:51966", Some(OpcodeStage::List1), 51966, "1:51966"),
    ] {
      let selector: OpcodeSelector = value.parse()?;
      assert_eq!(selector, OpcodeSelector { stage, id }, "{}", value);
      assert_eq!(selector.to_string(), display);
      assert_eq!(display.parse::<OpcodeSelector>()?, selector);
    }
    for value in ["Foo", "4:GainMap", "x:GainMap", "", "2:"] {
      assert!(value.parse::<OpcodeSelector>().is_err(), "{}", value);
    }
    Ok(())
  }

  #[test]
  fn remove_opcodes() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let gain_map = opcode(9, V1_3, 0, &gain_map([0, 0, 0, 0, 0, 1, 1, 1], (1, 1), (1.0, 1.0), (0.0, 0.0), 1, &[1.0]));
    let poly = opcode(8, V1_3, 0, &map_polynomial([0, 0, 0, 0, 0, 1, 1, 1], &[0.0, 1.0]));
    let unknown = opcode(51966, V1_3, 0, &[]);
    let mut image = float_image(2, 2, 1, 0.0);
    let lists = || -> Result<_> { Ok(Some(OpcodeList::parse(&opcode_list(&[gain_map.clone(), poly.clone(), unknown.clone()]))?)) };
    let ids = |list: &Option<OpcodeList>| list.as_ref().map(|list| list.opcodes().iter().map(Opcode::id).collect::<Vec<_>>());
    // A list prefix selects the opcode list
    (image.dng_opcodes_1, image.dng_opcodes_2, image.dng_opcodes_3) = (lists()?, lists()?, lists()?);
    image.remove_opcodes(&["2:GainMap".parse()?, "51966".parse()?]);
    assert_eq!(ids(&image.dng_opcodes_1), Some(vec![9, 8]));
    assert_eq!(ids(&image.dng_opcodes_2), Some(vec![8]));
    assert_eq!(ids(&image.dng_opcodes_3), Some(vec![9, 8]));
    // Lists without opcodes left are removed
    image.remove_opcodes(&["GainMap".parse()?, "3:MapPolynomial".parse()?]);
    assert_eq!(ids(&image.dng_opcodes_1), Some(vec![8]));
    assert_eq!(ids(&image.dng_opcodes_2), Some(vec![8]));
    assert_eq!(ids(&image.dng_opcodes_3), None);
    Ok(())
  }
}
