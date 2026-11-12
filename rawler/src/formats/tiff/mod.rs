// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::io::{Read, Seek, SeekFrom};

use thiserror::Error;

pub mod entry;
pub mod file;
pub mod ifd;
pub mod reader;
pub mod value;
pub mod writer;

pub use entry::Entry;
pub use ifd::IFD;
pub use reader::GenericTiffReader;
pub use value::{IntoTiffValue, Rational, SRational, TiffAscii, Value};
pub use writer::{DirectoryWriter, TiffWriter};

const TIFF_MAGIC: u16 = 42;

#[allow(clippy::upper_case_acronyms)]
#[derive(Debug, Copy, Clone, PartialEq, enumn::N)]
#[repr(u16)]
pub enum CompressionMethod {
  None = 1,
  Huffman = 2,
  Fax3 = 3,
  Fax4 = 4,
  LZW = 5,
  OldJPEG = 6, // TODO: RENAME
  // "Extended JPEG" or "new JPEG" style
  ModernJPEG = 7,
  Deflate = 8,
  OldDeflate = 0x80B2,
  PackBits = 0x8005,
  // DNG Extensions
  LossyJPEG = 34892,
  JPEGXL = 52546,
}

impl ExtractFromIFD for CompressionMethod {
  fn extract(ifd: &IFD) -> Result<Option<Self>> {
    let tag = TiffCommonTag::Compression;
    if let Some(entry) = ifd.get_entry(tag) {
      let x = entry.value.force_u16(0);
      Ok(Some(
        Self::n(x).ok_or_else(|| TiffError::General(format!("Unknown value {} for {:?}", x, tag)))?,
      ))
    } else {
      Ok(None)
    }
  }
}

impl From<CompressionMethod> for Value {
  fn from(value: CompressionMethod) -> Self {
    Value::Short(vec![value as u16])
  }
}

#[allow(clippy::upper_case_acronyms)]
#[derive(Debug, Copy, Clone, PartialEq, enumn::N)]
#[repr(u16)]
pub enum PhotometricInterpretation {
  WhiteIsZero = 0,
  BlackIsZero = 1,
  RGB = 2,
  RGBPalette = 3,
  TransparencyMask = 4,
  CMYK = 5,
  YCbCr = 6,
  CIELab = 8,
  // Defined by DNG
  CFA = 32803,
  LinearRaw = 34892,
  Depth = 51177,
  /// Semantic mask
  PhotometricMask = 52527,
}

impl ExtractFromIFD for PhotometricInterpretation {
  fn extract(ifd: &IFD) -> Result<Option<Self>> {
    let tag = TiffCommonTag::PhotometricInt;
    if let Some(entry) = ifd.get_entry(tag) {
      let x = entry.value.force_u16(0);
      Ok(Some(
        Self::n(x).ok_or_else(|| TiffError::General(format!("Unknown value {} for {:?}", x, tag)))?,
      ))
    } else {
      Ok(None)
    }
  }
}

impl From<PhotometricInterpretation> for Value {
  fn from(value: PhotometricInterpretation) -> Self {
    Value::Short(vec![value as u16])
  }
}

#[derive(Debug, Copy, Clone, PartialEq, enumn::N)]
#[repr(u16)]
pub enum PreviewColorSpace {
  Unknown = 0,
  GrayGamma = 1,
  SRgb = 2,
  AdobeRGB = 3,
  ProPhotoRGB = 5,
}

impl ExtractFromIFD for PreviewColorSpace {
  fn extract(ifd: &IFD) -> Result<Option<Self>> {
    let tag = DngTag::PreviewColorSpace;
    if let Some(entry) = ifd.get_entry(tag) {
      let x = entry.value.force_u16(0);
      Ok(Some(
        Self::n(x).ok_or_else(|| TiffError::General(format!("Unknown value {} for {:?}", x, tag)))?,
      ))
    } else {
      Ok(None)
    }
  }
}

impl From<PreviewColorSpace> for Value {
  fn from(value: PreviewColorSpace) -> Self {
    Value::Long(vec![value as u32])
  }
}

#[derive(Debug, Copy, Clone, PartialEq, enumn::N)]
#[repr(u16)]
pub enum PlanarConfiguration {
  Chunky = 1,
  Planar = 2,
}

impl ExtractFromIFD for PlanarConfiguration {
  fn extract(ifd: &IFD) -> Result<Option<Self>> {
    let tag = ExifTag::PlanarConfiguration;
    if let Some(entry) = ifd.get_entry(tag) {
      let x = entry.value.force_u16(0);
      Ok(Some(
        Self::n(x).ok_or_else(|| TiffError::General(format!("Unknown value {} for {:?}", x, tag)))?,
      ))
    } else {
      Ok(None)
    }
  }
}

impl From<PlanarConfiguration> for Value {
  fn from(value: PlanarConfiguration) -> Self {
    Value::Short(vec![value as u16])
  }
}

#[derive(Debug, Copy, Clone, PartialEq, enumn::N)]
#[repr(u16)]
pub enum Predictor {
  None = 1,
  Horizontal = 2,
  FloatingPoint = 3,
}

impl ExtractFromIFD for Predictor {
  fn extract(ifd: &IFD) -> Result<Option<Self>> {
    let tag = TiffCommonTag::Predictor;
    if let Some(entry) = ifd.get_entry(tag) {
      let x = entry.value.force_u16(0);
      Ok(Some(
        Self::n(x).ok_or_else(|| TiffError::General(format!("Unknown value {} for {:?}", x, tag)))?,
      ))
    } else {
      Ok(None)
    }
  }
}

impl From<Predictor> for Value {
  fn from(value: Predictor) -> Self {
    Value::Short(vec![value as u16])
  }
}

/// Type to represent resolution units
#[derive(Debug, Copy, Clone, PartialEq, enumn::N)]
#[repr(u16)]
pub enum ResolutionUnit {
  None = 1,
  Inch = 2,
  Centimeter = 3,
}

impl ExtractFromIFD for ResolutionUnit {
  fn extract(ifd: &IFD) -> Result<Option<Self>> {
    let tag = TiffCommonTag::ResolutionUnit;
    if let Some(entry) = ifd.get_entry(tag) {
      let x = entry.value.force_u16(0);
      Ok(Some(
        Self::n(x).ok_or_else(|| TiffError::General(format!("Unknown value {} for {:?}", x, tag)))?,
      ))
    } else {
      Ok(None)
    }
  }
}

impl From<ResolutionUnit> for Value {
  fn from(value: ResolutionUnit) -> Self {
    Value::Short(vec![value as u16])
  }
}

#[allow(clippy::upper_case_acronyms)]
#[derive(Debug, Copy, Clone, PartialEq, enumn::N)]
#[repr(u16)]
pub enum SampleFormat {
  Uint = 1,
  Int = 2,
  IEEEFP = 3,
  Void = 4,
}

impl From<SampleFormat> for Value {
  fn from(value: SampleFormat) -> Self {
    Value::Short(vec![value as u16])
  }
}

impl ExtractFromIFD for SampleFormat {
  fn extract(ifd: &IFD) -> Result<Option<Self>> {
    let tag = TiffCommonTag::SampleFormat;
    if let Some(entry) = ifd.get_entry(tag) {
      let x = entry.value.force_u16(0);
      Ok(Some(
        Self::n(x).ok_or_else(|| TiffError::General(format!("Unknown value {} for {:?}", x, tag)))?,
      ))
    } else {
      Ok(None)
    }
  }
}

pub trait ExtractFromIFD
where
  Self: Sized,
{
  fn extract(ifd: &IFD) -> Result<Option<Self>>;
}

/// Error variants for compressor
#[derive(Debug, Error)]
pub enum TiffError {
  /// Overflow of input, size constraints...
  #[error("Overflow error: {}", _0)]
  Overflow(String),

  #[error("General error: {}", _0)]
  General(String),

  #[error("Format mismatch: {}", _0)]
  FormatMismatch(String),

  /// Error on internal cursor type
  #[error("I/O error: {:?}", _0)]
  Io(#[from] std::io::Error),
}

/// Result type for Compressor results
pub type Result<T> = std::result::Result<T, TiffError>;

use crate::tags::{DngTag, ExifTag, TiffCommonTag};

/*
impl From<Value> for Entry {
  fn from(value: Value) -> Self {
    Entry { value, embedded: None }
  }
}
 */

pub struct DataOffset {
  pub offset: usize,
}

/// Apply the correction `corr` to an `offset` from the file
fn apply_corr(offset: u32, corr: i32) -> Result<u32> {
  u32::try_from(offset as i64 + corr as i64).map_err(|_| TiffError::Overflow(format!("Offset {} with correction {} is out of range", offset, corr)))
}

/// `a + b` for offsets from the file, an error on overflow
fn offset_add(a: u32, b: u32) -> Result<u32> {
  a.checked_add(b)
    .ok_or_else(|| TiffError::Overflow(format!("Offset {} + {} exceeds 4 GiB", a, b)))
}

pub struct DirReader {}

fn read_from_file<R: Read + Seek>(file: &mut R, offset: u64, size: usize) -> Result<Vec<u8>> {
  file.seek(SeekFrom::Start(offset))?;
  let mut buf = vec![0; size];
  file.read_exact(&mut buf)?;
  Ok(buf)
}

#[cfg(test)]
mod tests {
  use std::io::{Cursor, Seek, SeekFrom};

  use crate::{formats::tiff::reader::TiffReader, tags::TiffCommonTag};

  use super::*;

  /// Little-endian TIFF with the IFD 0 pointer `first` and one IFD for each
  /// element of `next`. IFD `i` is at offset `8 + 18 * i`, has one SHORT entry
  /// with the tag `0x100 + i` and the next IFD pointer `next[i]`.
  pub(crate) fn ifd_chain(first: u32, next: &[u32]) -> Vec<u8> {
    let mut data = vec![0x49, 0x49, 42, 0];
    data.extend_from_slice(&first.to_le_bytes());
    for (i, next) in next.iter().enumerate() {
      data.extend_from_slice(&1_u16.to_le_bytes());
      data.extend_from_slice(&(0x100 + i as u16).to_le_bytes());
      data.extend_from_slice(&3_u16.to_le_bytes());
      data.extend_from_slice(&1_u32.to_le_bytes());
      data.extend_from_slice(&[1, 0, 0, 0]);
      data.extend_from_slice(&next.to_le_bytes());
    }
    data
  }

  #[test]
  fn offsets_out_of_range_are_errors() {
    assert_eq!(apply_corr(10, -4).ok(), Some(6));
    assert!(apply_corr(5, -10).is_err());
    assert!(apply_corr(u32::MAX, 1).is_err());
    assert!(offset_add(u32::MAX, 1).is_err());
  }

  #[test]
  fn chained_ifds_are_corrected_once() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // The offsets in the file are 2 bytes less than the real ones
    let data = ifd_chain(6, &[24, 0]);
    let root = IFD::new_root_with_correction(&mut Cursor::new(data), 0, 0, 2, 10, &[])?;
    assert!(root.has_entry(0x100_u16));
    assert_eq!(root.chain.len(), 1);
    assert!(root.chain[0].has_entry(0x101_u16));
    Ok(())
  }

  #[test]
  fn encode_tiff_test() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut buf = Cursor::new(Vec::new());
    let mut tiff = TiffWriter::new(&mut buf)?;
    let mut dir = tiff.new_directory();

    let offset = {
      let mut dir2 = tiff.new_directory();
      dir2.add_tag(32_u16, 23_u16);
      dir2.build(&mut tiff)?
    };

    dir.add_tag(TiffCommonTag::ActiveArea, offset as u16);
    dir.add_tag(TiffCommonTag::ActiveArea, [23_u16, 45_u16]);
    dir.add_tag(TiffCommonTag::ActiveArea, &[23_u16, 45_u16][..]);
    dir.add_tag(TiffCommonTag::ActiveArea, "Fobbar");

    Ok(())
  }

  #[test]
  fn write_tiff_file_basic() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut output = Cursor::new(Vec::new());
    let mut tiff = TiffWriter::new(&mut output)?;

    let mut dir = tiff.new_directory();

    let offset = {
      let mut dir2 = tiff.new_directory();
      dir2.add_tag(32_u16, 23_u16);
      dir2.build(&mut tiff)?
    };

    dir.add_tag(TiffCommonTag::ExifIFDPointer, offset);

    dir.add_tag(TiffCommonTag::ActiveArea, [9_u16, 10_u16, 11_u16, 12, 13, 14]);
    dir.add_tag(TiffCommonTag::BlackLevels, [9_u16, 10_u16]);
    dir.add_tag(TiffCommonTag::WhiteLevel, [11_u16]);
    dir.add_tag(TiffCommonTag::BitsPerSample, [12_u32]);
    dir.add_tag(TiffCommonTag::ResolutionUnit, [-5_i32]);
    dir.add_tag(TiffCommonTag::Artist, "AT");

    tiff.build(dir)?;

    //assert!(TiffReader::is_tiff(&mut output) == true);

    let mut garbage_output: Vec<u8> = vec![0x4a, 0xee]; // Garbage
    garbage_output.extend_from_slice(&output.into_inner());

    let mut garbage_output = Cursor::new(garbage_output);

    garbage_output.seek(SeekFrom::Start(2))?;

    let reader = GenericTiffReader::new(&mut garbage_output, 1, 1, Some(16), &[])?; // 1 byte offset correction

    assert_eq!(reader.root_ifd().entry_count(), 7);
    assert!(reader.root_ifd().get_entry(TiffCommonTag::WhiteLevel).is_some());

    assert!(matches!(
      reader.root_ifd().get_entry(TiffCommonTag::ExifIFDPointer).unwrap().value,
      Value::Long { .. }
    ));
    assert!(matches!(
      reader.root_ifd().get_entry(TiffCommonTag::WhiteLevel).unwrap().value,
      Value::Short { .. }
    ));

    Ok(())
  }
}
