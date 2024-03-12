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
pub enum CompressionMethod {
  None = 1,
  Huffman = 2,
  Fax3 = 3,
  Fax4 = 4,
  LZW = 5,
  JPEG = 6,
  // "Extended JPEG" or "new JPEG" style
  ModernJPEG = 7,
  Deflate = 8,
  OldDeflate = 0x80B2,
  PackBits = 0x8005,
}

impl From<CompressionMethod> for Value {
  fn from(value: CompressionMethod) -> Self {
    Value::Short(vec![value as u16])
  }
}

#[allow(clippy::upper_case_acronyms)]
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
}

impl From<PhotometricInterpretation> for Value {
  fn from(value: PhotometricInterpretation) -> Self {
    Value::Short(vec![value as u16])
  }
}

pub enum PreviewColorSpace {
  Unknown = 0,
  GrayGamma = 1,
  SRgb = 2,
  AdobeRGB = 3,
  ProPhotoRGB = 5,
}

impl From<PreviewColorSpace> for Value {
  fn from(value: PreviewColorSpace) -> Self {
    Value::Long(vec![value as u32])
  }
}

pub enum PlanarConfiguration {
  Chunky = 1,
  Planar = 2,
}

impl From<PlanarConfiguration> for Value {
  fn from(value: PlanarConfiguration) -> Self {
    Value::Short(vec![value as u16])
  }
}

pub enum Predictor {
  None = 1,
  Horizontal = 2,
}

impl From<Predictor> for Value {
  fn from(value: Predictor) -> Self {
    Value::Short(vec![value as u16])
  }
}

/// Type to represent resolution units
pub enum ResolutionUnit {
  None = 1,
  Inch = 2,
  Centimeter = 3,
}

impl From<ResolutionUnit> for Value {
  fn from(value: ResolutionUnit) -> Self {
    Value::Short(vec![value as u16])
  }
}

#[allow(clippy::upper_case_acronyms)]
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

fn apply_corr(offset: u32, corr: i32) -> u32 {
  ((offset as i64) + (corr as i64)) as u32
}

pub struct DirReader {}

fn read_from_file<R: Read + Seek>(file: &mut R, offset: u32, size: usize) -> Result<Vec<u8>> {
  file.seek(SeekFrom::Start(offset as u64))?;
  let mut buf = vec![0; size];
  file.read_exact(&mut buf)?;
  Ok(buf)
}

#[cfg(test)]
mod tests {
  use std::io::{Cursor, Seek, SeekFrom};

  use crate::{formats::tiff::reader::TiffReader, tags::TiffCommonTag};

  use super::*;

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
