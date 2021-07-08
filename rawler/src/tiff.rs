// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::{
  collections::BTreeMap,
  ffi::CString,
  io::{Cursor, Read, Seek, SeekFrom, Write},
};

use byteorder::{BigEndian, LittleEndian, NativeEndian, ReadBytesExt, WriteBytesExt};
use log::debug;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

use crate::{bits::Endian, tags::TiffTagEnum};

const TYPE_BYTE: u16 = 1;
const TYPE_ASCII: u16 = 2;
const TYPE_SHORT: u16 = 3;
const TYPE_LONG: u16 = 4;
const TYPE_RATIONAL: u16 = 5;
const TYPE_SBYTE: u16 = 6;
const TYPE_UNDEFINED: u16 = 7;
const TYPE_SSHORT: u16 = 8;
const TYPE_SLONG: u16 = 9;
const TYPE_SRATIONAL: u16 = 10;
const TYPE_FLOAT: u16 = 11;
const TYPE_DOUBLE: u16 = 12;

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

  /// Error on internal cursor type
  #[error("I/O error")]
  Io(#[from] std::io::Error),
}

/// Result type for Compressor results
type Result<T> = std::result::Result<T, TiffError>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
  pub tag: u16,
  pub value: Value,
  // Embedded value for writer, offset for reader
  pub embedded: Option<u32>,
}

// 0-1-2-3-4-5-6-7-8-9-10-11-12-13
const DATASHIFTS: [u8; 14] = [0, 0, 0, 1, 2, 3, 0, 0, 1, 2, 3, 2, 3, 2];

impl Entry {
  pub fn value_type(&self) -> u16 {
    self.value.value_type()
  }

  pub fn count(&self) -> u32 {
    self.value.count() as u32
  }

  pub fn parse<R: Read + Seek>(reader: &mut EndianReader<R>, corr: i32) -> Result<Entry> {
    let pos = reader.position()?;

    let tag = reader.read_u16()?;
    let typ = reader.read_u16()?;
    let count = reader.read_u32()?;

    debug!("Tag: {:#x}, Typ: {:#x}, count: {}", tag, typ, count);

    // If we don't know the type assume byte data (undefined)
    let compat_typ = if typ == 0 || typ > 12 { 7 } else { typ };

    let bytesize: usize = (count as usize) << DATASHIFTS[compat_typ as usize];
    let data_offset: u32 = if bytesize <= 4 {
      reader.position()?
    } else {
      apply_corr(reader.read_u32()?, corr)
    };

    reader.goto(data_offset)?;
    let entry = match typ {
      TYPE_BYTE => {
        let mut v = vec![0; count as usize];
        reader.read_u8_into(&mut v)?;
        Entry {
          tag,
          value: Value::Byte(v),
          embedded: None,
        }
      }
      TYPE_ASCII => {
        let mut v = vec![0; count as usize];
        reader.read_u8_into(&mut v)?;
        Entry {
          tag,
          value: Value::Ascii(TiffAscii::new_from_raw(&v)),
          embedded: None,
        }
      }
      TYPE_SHORT => {
        let mut v = vec![0; count as usize];
        reader.read_u16_into(&mut v)?;
        Entry {
          tag,
          value: Value::Short(v),
          embedded: None,
        }
      }
      TYPE_LONG => {
        let mut v = vec![0; count as usize];
        reader.read_u32_into(&mut v)?;
        Entry {
          tag,
          value: Value::Long(v),
          embedded: None,
        }
      }
      TYPE_RATIONAL => {
        let mut tmp = vec![0; count as usize *2]; // Rational is 2x u32
        reader.read_u32_into(&mut tmp)?;

        let mut v = Vec::with_capacity(count as usize);
        for i in (0..count as usize).step_by(2) {
          v.push(Rational::new(tmp[i], tmp[i + 1]));
        }
        Entry {
          tag,
          value: Value::Rational(v),
          embedded: None,
        }
      }
      TYPE_SBYTE => {
        let mut v = vec![0; count as usize];
        reader.read_i8_into(&mut v)?;
        Entry {
          tag,
          value: Value::SByte(v),
          embedded: None,
        }
      }
      TYPE_UNDEFINED => {
        let mut v = vec![0; count as usize];
        reader.read_u8_into(&mut v)?;
        Entry {
          tag,
          value: Value::Undefined(v),
          embedded: None,
        }
      }
      TYPE_SSHORT => {
        let mut v = vec![0; count as usize];
        reader.read_i16_into(&mut v)?;
        Entry {
          tag,
          value: Value::SShort(v),
          embedded: None,
        }
      }
      TYPE_SLONG => {
        let mut v = vec![0; count as usize];
        reader.read_i32_into(&mut v)?;
        Entry {
          tag,
          value: Value::SLong(v),
          embedded: None,
        }
      }
      TYPE_SRATIONAL => {
        let mut tmp = vec![0; count as usize*2]; // SRational is 2x i32
        reader.read_i32_into(&mut tmp)?;

        let mut v = Vec::with_capacity(count as usize);
        for i in (0..count as usize).step_by(2) {
          v.push(SRational::new(tmp[i], tmp[i + 1]));
        }
        Entry {
          tag,
          value: Value::SRational(v),
          embedded: None,
        }
      }
      TYPE_FLOAT => {
        let mut v = vec![0.0; count as usize];
        reader.read_f32_into(&mut v)?;
        Entry {
          tag,
          value: Value::Float(v),
          embedded: None,
        }
      }
      TYPE_DOUBLE => {
        let mut v = vec![0.0; count as usize];
        reader.read_f64_into(&mut v)?;
        Entry {
          tag,
          value: Value::Double(v),
          embedded: None,
        }
      }
      x => {
        let mut v = vec![0; count as usize];
        reader.read_u8_into(&mut v)?;
        Entry {
          tag,
          value: Value::Unknown(x, v),
          embedded: None,
        }
      }
    };
    reader.goto(pos + 12)?; // Size of IFD entry
    Ok(entry)
  }
}

/*
impl From<Value> for Entry {
  fn from(value: Value) -> Self {
    Entry { value, embedded: None }
  }
}
 */

/// Type to represent tiff values of type `RATIONAL`
#[derive(Clone, Debug, Default, PartialEq, Copy)]
pub struct Rational {
  pub n: u32,
  pub d: u32,
}

impl Rational {
  pub fn new(n: u32, d: u32) -> Self {
    Self { n, d }
  }

  pub fn new_f32(n: f32, d: u32) -> Self {
    Self { n: (n * d as f32) as u32, d }
  }

  pub fn new_f64(n: f32, d: u32) -> Self {
    Self { n: (n * d as f32) as u32, d }
  }
}

impl Serialize for Rational {
  fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
  where
    S: Serializer,
  {
    let s = format!("{}/{}", self.n, self.d);
    serializer.serialize_str(&s)
  }
}

impl<'de> Deserialize<'de> for Rational {
  fn deserialize<D>(deserializer: D) -> std::result::Result<Rational, D::Error>
  where
    D: Deserializer<'de>,
  {
    use serde::de::Error;
    let s = String::deserialize(deserializer)?;
    let values: Vec<&str> = s.split("/").collect();
    if values.len() != 2 {
      Err(D::Error::custom(format!("Invalid rational value: {}", s)))
    } else {
      Ok(Rational::new(
        values[0].parse::<u32>().map_err(D::Error::custom)?,
        values[1].parse::<u32>().map_err(D::Error::custom)?,
      ))
    }
  }
}

/// Type to represent tiff values of type `SRATIONAL`
#[derive(Clone, Debug, Default, PartialEq, Copy)]
pub struct SRational {
  pub n: i32,
  pub d: i32,
}

impl SRational {
  pub fn new(n: i32, d: i32) -> Self {
    Self { n, d }
  }
}

impl Serialize for SRational {
  fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
  where
    S: Serializer,
  {
    let s = format!("{}/{}", self.n, self.d);
    serializer.serialize_str(&s)
  }
}

impl<'de> Deserialize<'de> for SRational {
  fn deserialize<D>(deserializer: D) -> std::result::Result<SRational, D::Error>
  where
    D: Deserializer<'de>,
  {
    use serde::de::Error;
    let s = String::deserialize(deserializer)?;
    let values: Vec<&str> = s.split("/").collect();
    if values.len() != 2 {
      Err(D::Error::custom(format!("Invalid srational value: {}", s)))
    } else {
      Ok(SRational::new(
        values[0].parse::<i32>().map_err(D::Error::custom)?,
        values[1].parse::<i32>().map_err(D::Error::custom)?,
      ))
    }
  }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
  /// 8-bit unsigned integer
  Byte(Vec<u8>),
  /// 8-bit byte that contains a 7-bit ASCII code; the last byte must be zero
  Ascii(TiffAscii),
  /// 16-bit unsigned integer
  Short(Vec<u16>),
  /// 32-bit unsigned integer
  Long(Vec<u32>),
  /// Fraction stored as two 32-bit unsigned integers
  Rational(Vec<Rational>),
  /// 8-bit signed integer
  SByte(Vec<i8>),
  /// 8-bit byte that may contain anything, depending on the field
  Undefined(Vec<u8>),
  /// 16-bit signed integer
  SShort(Vec<i16>),
  /// 32-bit signed integer
  SLong(Vec<i32>),
  /// Fraction stored as two 32-bit signed integers
  SRational(Vec<SRational>),
  /// 32-bit IEEE floating point
  Float(Vec<f32>),
  /// 64-bit IEEE floating point
  Double(Vec<f64>),
  /// Unknown type, wrapped in u8
  Unknown(u16, Vec<u8>),
}

impl Value {
  pub fn as_string(&self) -> Result<String> {
    match self {
      Self::Ascii(v) => Ok(v.strings()[0].clone()),
      _ => Err(TiffError::General("String value not available".into())),
    }
  }

  pub fn count(&self) -> usize {
    match self {
      Self::Byte(v) => v.len(),
      Self::Ascii(v) => v.count(),
      Self::Short(v) => v.len(),
      Self::Long(v) => v.len(),
      Self::Rational(v) => v.len(),
      Self::SByte(v) => v.len(),
      Self::Undefined(v) => v.len(),
      Self::SShort(v) => v.len(),
      Self::SLong(v) => v.len(),
      Self::SRational(v) => v.len(),
      Self::Float(v) => v.len(),
      Self::Double(v) => v.len(),
      Self::Unknown(_, v) => v.len(),
    }
  }

  pub fn byte_size(&self) -> usize {
    match self {
      Self::Byte(v) => v.len() * std::mem::size_of::<u8>(),
      Self::Ascii(v) => v.count(),
      Self::Short(v) => v.len() * std::mem::size_of::<u16>(),
      Self::Long(v) => v.len() * std::mem::size_of::<u32>(),
      Self::Rational(v) => v.len() * 8,
      Self::SByte(v) => v.len() * std::mem::size_of::<i8>(),
      Self::Undefined(v) => v.len() * std::mem::size_of::<u8>(),
      Self::SShort(v) => v.len() * std::mem::size_of::<i16>(),
      Self::SLong(v) => v.len() * std::mem::size_of::<i32>(),
      Self::SRational(v) => v.len() * 8,
      Self::Float(v) => v.len() * std::mem::size_of::<f32>(),
      Self::Double(v) => v.len() * std::mem::size_of::<f64>(),
      Self::Unknown(_, v) => v.len() * std::mem::size_of::<u8>(),
    }
  }

  pub fn as_embedded(&self) -> Result<u32> {
    if self.count() == 0 {
      // TODO: is zero count allowed?
      return Err(TiffError::General("Entry as count == 0".into()));
    }
    if self.byte_size() > 4 {
      return Err(TiffError::Overflow(format!("Invalid data")));
    } else {
      match self {
        Self::Byte(v) => Ok(
          (*v.get(0).unwrap_or(&0) as u32)
            | ((*v.get(1).unwrap_or(&0) as u32) << 8)
            | ((*v.get(2).unwrap_or(&0) as u32) << 16)
            | ((*v.get(3).unwrap_or(&0) as u32) << 24),
        ),
        Self::Ascii(v) => {
          //let cstr = CString::new(v.as_str()).unwrap();
          let v = v.as_vec_with_nul();
          Ok(
            (*v.get(0).unwrap_or(&0) as u32)
              | ((*v.get(1).unwrap_or(&0) as u32) << 8)
              | ((*v.get(2).unwrap_or(&0) as u32) << 16)
              | ((*v.get(3).unwrap_or(&0) as u32) << 24),
          )
        }
        Self::Short(v) => Ok((v[0] as u32) | (*v.get(1).unwrap_or(&0) as u32) << 16),
        Self::Long(v) => Ok(v[0]),
        Self::SByte(v) => Ok(
          (*v.get(0).unwrap_or(&0) as u32)
            | ((*v.get(1).unwrap_or(&0) as u32) << 8)
            | ((*v.get(2).unwrap_or(&0) as u32) << 16)
            | ((*v.get(3).unwrap_or(&0) as u32) << 24),
        ),
        Self::Undefined(v) => Ok(
          (*v.get(0).unwrap_or(&0) as u32)
            | ((*v.get(1).unwrap_or(&0) as u32) << 8)
            | ((*v.get(2).unwrap_or(&0) as u32) << 16)
            | ((*v.get(3).unwrap_or(&0) as u32) << 24),
        ),
        Self::SShort(v) => Ok((v[0] as u32) | (*v.get(1).unwrap_or(&0) as u32) << 16),
        Self::SLong(v) => Ok(v[0] as u32),
        Self::Float(v) => Ok(v[0] as u32),
        Self::Unknown(_, v) => Ok(
          (*v.get(0).unwrap_or(&0) as u32)
            | ((*v.get(1).unwrap_or(&0) as u32) << 8)
            | ((*v.get(2).unwrap_or(&0) as u32) << 16)
            | ((*v.get(3).unwrap_or(&0) as u32) << 24),
        ),
        _ => {
          panic!("unsupported: {:?}", self);
        }
      }
    }
  }

  pub fn write(&self, w: &mut dyn WriteAndSeek) -> Result<()> {
    match self {
      Self::Byte(val) => {
        w.write_all(val)?;
      }
      Self::Ascii(val) => {
        //let cstr = CString::new(val.as_str()).unwrap();
        let bytes = val.as_vec_with_nul();
        w.write_all(&bytes)?;
      }
      Self::Short(val) => {
        for x in val {
          w.write_u16::<NativeEndian>(*x)?;
        }
      }
      Self::Long(val) => {
        for x in val {
          w.write_u32::<NativeEndian>(*x)?;
        }
      }
      Self::Rational(val) => {
        for x in val {
          w.write_u32::<NativeEndian>(x.n)?;
          w.write_u32::<NativeEndian>(x.d)?;
        }
      }
      Self::SByte(val) => {
        for x in val {
          w.write_i8(*x)?;
        }
      }
      Self::Undefined(val) => {
        w.write_all(&val)?;
      }
      Self::SShort(val) => {
        for x in val {
          w.write_i16::<NativeEndian>(*x)?;
        }
      }
      Self::SLong(val) => {
        for x in val {
          w.write_i32::<NativeEndian>(*x)?;
        }
      }
      Self::SRational(val) => {
        for x in val {
          w.write_i32::<NativeEndian>(x.n)?;
          w.write_i32::<NativeEndian>(x.d)?;
        }
      }
      Self::Float(val) => {
        for x in val {
          w.write_f32::<NativeEndian>(*x)?;
        }
      }
      Self::Double(val) => {
        for x in val {
          w.write_f64::<NativeEndian>(*x)?;
        }
      }
      Self::Unknown(_, val) => {
        w.write_all(val)?;
      }
    }
    Ok(())
  }

  pub fn value_type(&self) -> u16 {
    match self {
      Self::Byte(_) => 1,
      Self::Ascii(_) => 2,
      Self::Short(_) => 3,
      Self::Long(_) => 4,
      Self::Rational(_) => 5,
      Self::SByte(_) => 6,
      Self::Undefined(_) => 7,
      Self::SShort(_) => 8,
      Self::SLong(_) => 9,
      Self::SRational(_) => 10,
      Self::Float(_) => 11,
      Self::Double(_) => 12,
      Self::Unknown(t, _) => t.clone(),
    }
  }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TiffAscii {
  strings: Vec<String>,
}

impl TiffAscii {
  pub fn new<T: AsRef<str>>(value: T) -> Self {
    Self {
      strings: vec![String::from(value.as_ref())],
    }
  }

  pub fn new_from_vec(values: Vec<String>) -> Self {
    Self { strings: values }
  }

  pub fn strings(&self) -> &Vec<String> {
    &self.strings
  }

  pub fn first(&self) -> &String {
    &self.strings[0]
  }

  pub fn count(&self) -> usize {
    self.strings.iter().map(|s| s.len() + 1).sum::<usize>()
  }

  pub fn as_vec_with_nul(&self) -> Vec<u8> {
    let mut out = Vec::new();
    for s in &self.strings {
      let cstr = CString::new(s.as_bytes()).unwrap();
      out.extend_from_slice(cstr.to_bytes_with_nul());
    }
    out
  }

  pub fn new_from_raw(raw: &[u8]) -> Self {
    let mut strings = Vec::new();
    let mut nul_range_end = 0;

    // TODO: fixme multiple strings
    //while nul_range_end < raw.len() {
    nul_range_end = raw[nul_range_end..].iter().position(|&c| c == b'\0').unwrap_or(raw.len()); // default to length if no `\0` present
    let s = ::std::str::from_utf8(&raw[0..nul_range_end]).unwrap();
    strings.push(String::from(s));
    //nul_range_end += 1;
    // }

    Self { strings }
  }
}

pub struct DataOffset {
  pub offset: usize,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct IFD {
  offset: u32,
  next_ifd: u32,
  entries: BTreeMap<u16, Entry>,
  //sub: BTreeMap<u16, Vec<IFD>>,
}

pub trait ReadByteOrder {
  fn read_u8(&mut self) -> std::io::Result<u8>;
  fn read_i8(&mut self) -> std::io::Result<i8>;
  fn read_u16(&mut self) -> std::io::Result<u16>;
  fn read_i16(&mut self) -> std::io::Result<i16>;
  fn read_u32(&mut self) -> std::io::Result<u32>;
  fn read_i32(&mut self) -> std::io::Result<i32>;
  fn read_u64(&mut self) -> std::io::Result<u64>;
  fn read_i64(&mut self) -> std::io::Result<i64>;
  fn read_f32(&mut self) -> std::io::Result<f32>;
  fn read_f64(&mut self) -> std::io::Result<f64>;

  fn read_u8_into(&mut self, dst: &mut [u8]) -> std::io::Result<()>;
  fn read_i8_into(&mut self, dst: &mut [i8]) -> std::io::Result<()>;
  fn read_u16_into(&mut self, dst: &mut [u16]) -> std::io::Result<()>;
  fn read_i16_into(&mut self, dst: &mut [i16]) -> std::io::Result<()>;
  fn read_u32_into(&mut self, dst: &mut [u32]) -> std::io::Result<()>;
  fn read_i32_into(&mut self, dst: &mut [i32]) -> std::io::Result<()>;
  fn read_u64_into(&mut self, dst: &mut [u64]) -> std::io::Result<()>;
  fn read_i64_into(&mut self, dst: &mut [i64]) -> std::io::Result<()>;
  fn read_f32_into(&mut self, dst: &mut [f32]) -> std::io::Result<()>;
  fn read_f64_into(&mut self, dst: &mut [f64]) -> std::io::Result<()>;
}

pub struct EndianReader<'a, R: Read + Seek + 'a> {
  endian: Endian,
  inner: &'a mut R,
}

impl<'a, R: Read + Seek + 'a> EndianReader<'a, R> {
  pub fn new(inner: &'a mut R, endian: Endian) -> Self {
    Self { endian, inner }
  }

  pub fn into_inner(self) -> &'a mut R {
    self.inner
  }

  pub fn position(&mut self) -> Result<u32> {
    Ok(self.inner.stream_position().map(|v| v as u32)?)
  }

  // TODO: try_from?

  pub fn goto(&mut self, offset: u32) -> Result<()> {
    self.inner.seek(SeekFrom::Start(offset as u64))?;
    Ok(())

    // TODO: try_from?
  }
}

impl<'a, R: Read + Seek + 'a> ReadByteOrder for EndianReader<'a, R> {
  fn read_u16(&mut self) -> std::io::Result<u16> {
    match self.endian {
      Endian::Little => self.inner.read_u16::<LittleEndian>(),
      Endian::Big => self.inner.read_u16::<BigEndian>(),
    }
  }

  fn read_u8(&mut self) -> std::io::Result<u8> {
    self.inner.read_u8()
  }

  fn read_i8(&mut self) -> std::io::Result<i8> {
    match self.endian {
      Endian::Little => self.inner.read_i8(),
      Endian::Big => self.inner.read_i8(),
    }
  }

  fn read_i16(&mut self) -> std::io::Result<i16> {
    match self.endian {
      Endian::Little => self.inner.read_i16::<LittleEndian>(),
      Endian::Big => self.inner.read_i16::<BigEndian>(),
    }
  }

  fn read_u32(&mut self) -> std::io::Result<u32> {
    match self.endian {
      Endian::Little => self.inner.read_u32::<LittleEndian>(),
      Endian::Big => self.inner.read_u32::<BigEndian>(),
    }
  }

  fn read_i32(&mut self) -> std::io::Result<i32> {
    match self.endian {
      Endian::Little => self.inner.read_i32::<LittleEndian>(),
      Endian::Big => self.inner.read_i32::<BigEndian>(),
    }
  }

  fn read_u64(&mut self) -> std::io::Result<u64> {
    match self.endian {
      Endian::Little => self.inner.read_u64::<LittleEndian>(),
      Endian::Big => self.inner.read_u64::<BigEndian>(),
    }
  }

  fn read_i64(&mut self) -> std::io::Result<i64> {
    match self.endian {
      Endian::Little => self.inner.read_i64::<LittleEndian>(),
      Endian::Big => self.inner.read_i64::<BigEndian>(),
    }
  }

  fn read_f32(&mut self) -> std::io::Result<f32> {
    match self.endian {
      Endian::Little => self.inner.read_f32::<LittleEndian>(),
      Endian::Big => self.inner.read_f32::<BigEndian>(),
    }
  }

  fn read_f64(&mut self) -> std::io::Result<f64> {
    match self.endian {
      Endian::Little => self.inner.read_f64::<LittleEndian>(),
      Endian::Big => self.inner.read_f64::<BigEndian>(),
    }
  }

  fn read_u8_into(&mut self, dst: &mut [u8]) -> std::io::Result<()> {
    self.inner.read_exact(dst)
  }

  fn read_i8_into(&mut self, dst: &mut [i8]) -> std::io::Result<()> {
    self.inner.read_i8_into(dst)
  }

  fn read_u16_into(&mut self, dst: &mut [u16]) -> std::io::Result<()> {
    match self.endian {
      Endian::Little => self.inner.read_u16_into::<LittleEndian>(dst),
      Endian::Big => self.inner.read_u16_into::<BigEndian>(dst),
    }
  }

  fn read_i16_into(&mut self, dst: &mut [i16]) -> std::io::Result<()> {
    match self.endian {
      Endian::Little => self.inner.read_i16_into::<LittleEndian>(dst),
      Endian::Big => self.inner.read_i16_into::<BigEndian>(dst),
    }
  }

  fn read_u32_into(&mut self, dst: &mut [u32]) -> std::io::Result<()> {
    match self.endian {
      Endian::Little => self.inner.read_u32_into::<LittleEndian>(dst),
      Endian::Big => self.inner.read_u32_into::<BigEndian>(dst),
    }
  }

  fn read_i32_into(&mut self, dst: &mut [i32]) -> std::io::Result<()> {
    match self.endian {
      Endian::Little => self.inner.read_i32_into::<LittleEndian>(dst),
      Endian::Big => self.inner.read_i32_into::<BigEndian>(dst),
    }
  }

  fn read_u64_into(&mut self, dst: &mut [u64]) -> std::io::Result<()> {
    match self.endian {
      Endian::Little => self.inner.read_u64_into::<LittleEndian>(dst),
      Endian::Big => self.inner.read_u64_into::<BigEndian>(dst),
    }
  }

  fn read_i64_into(&mut self, dst: &mut [i64]) -> std::io::Result<()> {
    match self.endian {
      Endian::Little => self.inner.read_i64_into::<LittleEndian>(dst),
      Endian::Big => self.inner.read_i64_into::<BigEndian>(dst),
    }
  }

  fn read_f32_into(&mut self, dst: &mut [f32]) -> std::io::Result<()> {
    match self.endian {
      Endian::Little => self.inner.read_f32_into::<LittleEndian>(dst),
      Endian::Big => self.inner.read_f32_into::<BigEndian>(dst),
    }
  }

  fn read_f64_into(&mut self, dst: &mut [f64]) -> std::io::Result<()> {
    match self.endian {
      Endian::Little => self.inner.read_f64_into::<LittleEndian>(dst),
      Endian::Big => self.inner.read_f64_into::<BigEndian>(dst),
    }
  }
}

impl IFD {
  pub fn new<R: Read + Seek>(reader: &mut R, offset: u32, corr: i32, endian: Endian) -> Result<Self> {
    reader.seek(SeekFrom::Start(offset as u64))?;
    let mut reader = EndianReader::new(reader, endian);
    let entry_count = reader.read_u16()?;
    let mut entries = BTreeMap::new();
    for _ in 0..entry_count {
      //let embedded = reader.read_u32()?;
      let entry = Entry::parse(&mut reader, corr)?;
      entries.insert(entry.tag, entry);
    }
    let next_ifd = reader.read_u32()?;
    Ok(Self {
      offset,
      next_ifd: if next_ifd == 0 { 0 } else { apply_corr(next_ifd, corr) },
      entries,
      //sub: BTreeMap::new(),
    })
  }

  pub fn entry_count(&self) -> u16 {
    self.entries.len() as u16
  }

  pub fn next_ifd(&self) -> u32 {
    self.next_ifd
  }

  pub fn entries(&self) -> &BTreeMap<u16, Entry> {
    &self.entries
  }

  pub fn get_entry<T: TiffTagEnum>(&self, tag: T) -> Option<&Entry> {
    self.entries.get(&tag.into())
  }

  pub fn has_entry<T: TiffTagEnum>(&self, tag: T) -> bool {
    self.get_entry(tag).is_some()
  }
}

fn apply_corr(offset: u32, corr: i32) -> u32 {
  ((offset as i64) + (corr as i64)) as u32
}

/// Reader for TIFF files
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TiffReader {
  /// Chain of all IFDs in TIFF
  pub chain: Vec<IFD>,
  /// Offset correction value
  pub corr: i32,
}

impl TiffReader {
  /// Check if buffer looks like a TIFF file
  pub fn is_tiff<T: AsRef<[u8]>>(buffer: T) -> bool {
    let buffer = buffer.as_ref();
    buffer[0] == 0x49 || buffer[0] == 0x4d // TODO
  }

  /// Construct a TIFF reader from a byte buffer
  ///
  /// Byte buffer must be a full TIFF file structure, endianess is detected from TIFF
  /// header.
  ///
  /// `corr` is a correction value that should be applied to offsets received
  /// from file structure.
  pub fn new_with_buffer<T: AsRef<[u8]>>(buffer: T, corr: i32, max_chained: Option<usize>) -> Result<Self> {
    let mut cursor = Cursor::new(buffer.as_ref());
    Self::new(&mut cursor, corr, max_chained)
  }

  /// Construct a TIFF reader from Read capable objects
  ///
  /// `corr` is a correction value that should be applied to offsets received
  /// from file structure.
  pub fn new<R: Read + Seek>(file: &mut R, corr: i32, max_chained: Option<usize>) -> Result<Self> {
    let endian = match file.read_u16::<LittleEndian>()? {
      0x4949 => Endian::Little,
      0x4d4d => Endian::Big,
      x => {
        return Err(TiffError::General(format!("TIFF: don't know marker 0x{:x}", x)));
      }
    };
    let mut reader = EndianReader::new(file, endian);
    let magic = reader.read_u16()?;
    if magic != 42 {
      return Err(TiffError::General(format!("Invalid magic marker for TIFF: {}", magic)));
    }
    let mut next_ifd = reader.read_u32()?;
    if next_ifd == 0 {
      return Err(TiffError::General(format!("Invalid TIFF header, contains no root IFD")));
    }

    let reader = reader.into_inner();

    let mut chain = Vec::new();
    while next_ifd != 0 {
      // TODO: check if offset is in range
      let ifd = IFD::new(reader, apply_corr(next_ifd, corr), corr, endian)?;
      if ifd.entries.is_empty() {
        return Err(TiffError::General(format!("TIFF is invalid, IFD must contain at least one entry")));
      }
      next_ifd = ifd.next_ifd;
      chain.push(ifd);
      if let Some(max) = max_chained {
        if chain.len() > max {
          break;
        }
      }
    }

    if chain.is_empty() {
      return Err(TiffError::General(format!("TIFF is invalid, must contain at least one IFD")));
    }

    Ok(Self { chain, corr })
  }

  pub fn root_ifd(&self) -> &IFD {
    if self.chain.is_empty() {
      panic!("TIFF must have at least one root IFD but the IFD list is empty");
    }
    &self.chain[0]
  }

  pub fn get_entry<T: TiffTagEnum>(&self, tag: T) -> Option<&Entry> {
    for ifd in &self.chain {
      match ifd.get_entry(tag) {
        Some(x) => return Some(x),
        None => {}
      }
    }
    None
  }

  pub fn has_entry<T: TiffTagEnum>(&self, tag: T) -> bool {
    self.get_entry(tag).is_some()
  }

  pub fn find_ifds_with_tag<T: TiffTagEnum>(&self, tag: T) -> Vec<&IFD> {
    let mut ifds = Vec::new();
    for ifd in &self.chain {
      if ifd.has_entry(tag) {
        ifds.push(ifd);
      }
    }
    ifds
  }

  pub fn find_first_ifd_with_tag<T: TiffTagEnum>(&self, tag: T) -> Option<&IFD> {
    let ifds = self.find_ifds_with_tag(tag);
    if ifds.len() == 0 {
      None
    } else {
      Some(ifds[0])
    }
  }

  pub fn get_first_entry(&self, _tag: u16) -> Option<Entry> {
    unimplemented!();
    /*
    Some(Entry {
      value: (32 as u16).into(),
      embedded: None,
    })
     */
  }

  pub fn read_data<R: Read + Seek>(&self, file: &mut R, uncorr_offset: u32, buffer: &mut [u8]) -> Result<()> {
    file.seek(SeekFrom::Start(apply_corr(uncorr_offset, self.corr) as u64))?;
    file.read_exact(buffer)?;
    Ok(())
  }
}

pub struct TiffWriter<'w> {
  ifd_location: u64,
  pub writer: &'w mut dyn WriteAndSeek,
}

pub trait WriteAndSeek: Write + Seek {}

impl<T> WriteAndSeek for T where T: Write + Seek {}

impl<'w> TiffWriter<'w> {
  pub fn new(writer: &'w mut dyn WriteAndSeek) -> Result<Self> {
    let mut tmp = Self { writer, ifd_location: 0 };
    tmp.write_header()?;
    Ok(tmp)
  }

  pub fn new_directory(&mut self) -> DirectoryWriter<'_, 'w> {
    DirectoryWriter::new(self)
  }

  fn write_header(&mut self) -> Result<()> {
    #[cfg(target_endian = "little")]
    let boi: u8 = 0x49;
    #[cfg(not(target_endian = "little"))]
    let boi: u8 = 0x4d;

    self.writer.write_all(&[boi, boi])?;
    self.writer.write_u16::<NativeEndian>(TIFF_MAGIC)?;
    self.ifd_location = self.writer.stream_position()?;
    self.writer.write_u32::<NativeEndian>(0_u32)?;

    Ok(())
  }

  pub(crate) fn pad_word_boundary(&mut self) -> Result<()> {
    if self.position()? % 4 != 0 {
      let padding = [0, 0, 0];
      let padd_len = 4 - (self.position()? % 4);
      self.writer.write_all(&padding[..padd_len as usize])?;
    }
    Ok(())
  }

  pub fn build(self, ifd0_offset: u32) -> Result<()> {
    self.writer.seek(SeekFrom::Start(self.ifd_location))?;
    self.writer.write_u32::<NativeEndian>(ifd0_offset)?;
    Ok(())
  }

  pub fn position(&mut self) -> Result<u32> {
    Ok(self.writer.stream_position().map(|v| v as u32)?) // TODO: try_from?
  }
}

pub struct DirectoryWriter<'a, 'w> {
  pub tiff: &'a mut TiffWriter<'w>,
  // We use BTreeMap to make sure tags are written in correct order
  entries: BTreeMap<u16, Entry>,
  next_ifd: u32,
}

impl<'a, 'w> DirectoryWriter<'a, 'w> {
  pub fn new(tiff: &'a mut TiffWriter<'w>) -> Self {
    Self {
      tiff,
      entries: BTreeMap::new(),
      next_ifd: 0,
    }
  }

  pub fn new_directory(&mut self) -> DirectoryWriter<'_, 'w> {
    DirectoryWriter::new(self.tiff)
  }

  pub fn entry_count(&self) -> u16 {
    self.entries.len() as u16
  }

  pub fn build(mut self) -> Result<u32> {
    if self.entries.is_empty() {
      return Err(TiffError::General(format!("IFD is empty, not allowed by TIFF specification")));
    }
    for &mut Entry {
      ref mut value,
      ref mut embedded,
      ..
    } in self.entries.values_mut()
    {
      let data_bytes = 4;

      if value.byte_size() > data_bytes {
        self.tiff.pad_word_boundary()?;
        let offset = self.tiff.position()?;
        value.write(self.tiff.writer)?;
        embedded.replace(offset as u32);
      } else {
        embedded.replace(value.as_embedded()?);
      }
    }

    self.tiff.pad_word_boundary()?;
    let offset = self.tiff.position()?;

    self.tiff.writer.write_all(&self.entry_count().to_ne_bytes())?;

    for (tag, entry) in self.entries {
      self.tiff.writer.write_u16::<NativeEndian>(tag)?;
      self.tiff.writer.write_u16::<NativeEndian>(entry.value_type())?;
      self.tiff.writer.write_u32::<NativeEndian>(entry.count())?;
      self.tiff.writer.write_u32::<NativeEndian>(entry.embedded.unwrap())?;
    }
    self.tiff.writer.write_u32::<NativeEndian>(self.next_ifd)?; // Next IFD

    Ok(offset)
  }

  pub fn add_tag<T: TiffTagEnum, V: Into<Value>>(&mut self, tag: T, value: V) -> Result<()> {
    let tag: u16 = tag.into();
    self.entries.insert(
      tag,
      Entry {
        tag,
        value: value.into(),
        embedded: None,
      },
    );
    Ok(())
  }

  pub fn add_tag_undefined<T: TiffTagEnum>(&mut self, tag: T, data: Vec<u8>) -> Result<()> {
    let tag: u16 = tag.into();
    //let data = data.as_ref();
    //let offset = self.write_data(data)?;
    self.entries.insert(
      tag,
      Entry {
        tag,
        value: Value::Undefined(data),
        embedded: None,
      },
    );
    Ok(())
  }

  pub fn add_value<T: TiffTagEnum>(&mut self, tag: T, value: Value) -> Result<()> {
    let tag: u16 = tag.into();
    self.entries.insert(tag, Entry { tag, value, embedded: None });
    Ok(())
  }

  /*
  pub fn add_entry(&mut self, entry: Entry) {
    self.ifd.insert(tag.into(), entry);
  }
   */

  pub fn write_data(&mut self, data: &[u8]) -> Result<u32> {
    self.tiff.pad_word_boundary()?;
    let offset = self.tiff.position()?;
    self.tiff.writer.write_all(data)?;
    Ok(offset)
  }

  pub fn write_data_u16_be(&mut self, data: &[u16]) -> Result<u32> {
    self.tiff.pad_word_boundary()?;
    let offset = self.tiff.position()?;
    for v in data {
      self.tiff.writer.write_u16::<LittleEndian>(*v)?;
    }
    Ok(offset)
  }
}

pub struct DirReader {}

pub trait IntoTiffValue {
  fn count(&self) -> usize;
  fn size(&self) -> usize;
  fn bytes(&self) -> usize {
    self.count() * self.size()
  }
}

impl From<Rational> for Value {
  fn from(value: Rational) -> Self {
    Value::Rational(vec![value])
  }
}

impl From<&[Rational]> for Value {
  fn from(value: &[Rational]) -> Self {
    Value::Rational(value.into())
  }
}

impl<const N: usize> From<[Rational; N]> for Value {
  fn from(value: [Rational; N]) -> Self {
    Value::Rational(value.into())
  }
}

impl From<SRational> for Value {
  fn from(value: SRational) -> Self {
    Value::SRational(vec![value])
  }
}

impl From<&[SRational]> for Value {
  fn from(value: &[SRational]) -> Self {
    Value::SRational(value.into())
  }
}

impl<const N: usize> From<[SRational; N]> for Value {
  fn from(value: [SRational; N]) -> Self {
    Value::SRational(value.into())
  }
}

impl From<&str> for Value {
  fn from(value: &str) -> Self {
    Value::Ascii(TiffAscii::new(value))
  }
}

impl From<&String> for Value {
  fn from(value: &String) -> Self {
    Value::Ascii(TiffAscii::new(value))
  }
}

impl From<String> for Value {
  fn from(value: String) -> Self {
    Value::Ascii(TiffAscii::new(&value))
  }
}

impl From<f32> for Value {
  fn from(value: f32) -> Self {
    Value::Float(vec![value])
  }
}

impl From<&[f32]> for Value {
  fn from(value: &[f32]) -> Self {
    Value::Float(value.into())
  }
}

impl<const N: usize> From<[f32; N]> for Value {
  fn from(value: [f32; N]) -> Self {
    Value::Float(value.into())
  }
}

impl From<f64> for Value {
  fn from(value: f64) -> Self {
    Value::Double(vec![value])
  }
}

impl From<&[f64]> for Value {
  fn from(value: &[f64]) -> Self {
    Value::Double(value.into())
  }
}

impl<const N: usize> From<[f64; N]> for Value {
  fn from(value: [f64; N]) -> Self {
    Value::Double(value.into())
  }
}

impl From<u8> for Value {
  fn from(value: u8) -> Self {
    Value::Byte(vec![value])
  }
}

impl From<&[u8]> for Value {
  fn from(value: &[u8]) -> Self {
    Value::Byte(value.into())
  }
}

impl<const N: usize> From<[u8; N]> for Value {
  fn from(value: [u8; N]) -> Self {
    Value::Byte(value.into())
  }
}

impl From<u16> for Value {
  fn from(value: u16) -> Self {
    Value::Short(vec![value])
  }
}

impl From<&[u16]> for Value {
  fn from(value: &[u16]) -> Self {
    Value::Short(value.into())
  }
}

impl From<&Vec<u16>> for Value {
  fn from(value: &Vec<u16>) -> Self {
    Value::Short(value.clone())
  }
}

impl<const N: usize> From<[u16; N]> for Value {
  fn from(value: [u16; N]) -> Self {
    Value::Short(value.into())
  }
}

impl From<u32> for Value {
  fn from(value: u32) -> Self {
    Value::Long(vec![value])
  }
}

impl From<&[u32]> for Value {
  fn from(value: &[u32]) -> Self {
    Value::Long(value.into())
  }
}

impl From<&Vec<u32>> for Value {
  fn from(value: &Vec<u32>) -> Self {
    Value::Long(value.clone())
  }
}

impl<const N: usize> From<[u32; N]> for Value {
  fn from(value: [u32; N]) -> Self {
    Value::Long(value.into())
  }
}

impl From<i8> for Value {
  fn from(value: i8) -> Self {
    Value::SByte(vec![value])
  }
}

impl From<&[i8]> for Value {
  fn from(value: &[i8]) -> Self {
    Value::SByte(value.into())
  }
}

impl<const N: usize> From<[i8; N]> for Value {
  fn from(value: [i8; N]) -> Self {
    Value::SByte(value.into())
  }
}

impl From<i16> for Value {
  fn from(value: i16) -> Self {
    Value::SShort(vec![value])
  }
}

impl From<&[i16]> for Value {
  fn from(value: &[i16]) -> Self {
    Value::SShort(value.into())
  }
}

impl<const N: usize> From<[i16; N]> for Value {
  fn from(value: [i16; N]) -> Self {
    Value::SShort(value.into())
  }
}

impl From<i32> for Value {
  fn from(value: i32) -> Self {
    Value::SLong(vec![value])
  }
}

impl From<&[i32]> for Value {
  fn from(value: &[i32]) -> Self {
    Value::SLong(value.into())
  }
}

impl<const N: usize> From<[i32; N]> for Value {
  fn from(value: [i32; N]) -> Self {
    Value::SLong(value.into())
  }
}

#[cfg(test)]
mod tests {
  use std::io::Cursor;

  use crate::tags::TiffRootTag;

  use super::*;

  fn _transfer_entry(_in_tiff: &TiffReader, out_tiff: &mut DirectoryWriter) {
    out_tiff.add_tag(34, "Foo").unwrap();
    //out_tiff.add_entry(in_tiff.get_first_entry(23).unwrap());
  }

  #[test]
  fn encode_tiff_test() -> std::result::Result<(), Box<dyn std::error::Error>> {
    //let mut demo = Cursor::new(Vec::new());

    //let tiff_reader = TiffReader::new(&mut demo, 0, Some(16)).unwrap();

    let mut buf = Cursor::new(Vec::new());

    let mut tiff = TiffWriter::new(&mut buf).unwrap();

    let mut dir = tiff.new_directory();

    //transfer_entry(&tiff_reader, &mut dir);

    let offset = {
      let mut dir2 = dir.new_directory();
      dir2.add_tag(32 as u16, 23 as u16)?;
      dir2.build()?
    };

    dir.add_tag(TiffRootTag::ActiveArea, offset as u16)?;
    dir.add_tag(TiffRootTag::ActiveArea, [23_u16, 45_u16])?;
    dir.add_tag(TiffRootTag::ActiveArea, &[23_u16, 45_u16][..])?;
    dir.add_tag(TiffRootTag::ActiveArea, "Fobbar")?;

    Ok(())
  }

  #[test]
  fn write_tiff_file_basic() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut output = Cursor::new(Vec::new());
    let mut tiff = TiffWriter::new(&mut output).unwrap();

    let ifd_offset = {
      let mut dir = tiff.new_directory();

      let offset = {
        let mut dir2 = dir.new_directory();
        dir2.add_tag(32 as u16, 23 as u16)?;
        dir2.build()?
      };

      dir.add_tag(TiffRootTag::ExifIFDPointer, offset)?;

      dir.add_tag(TiffRootTag::ActiveArea, [9_u16, 10_u16, 11_u16, 12, 13, 14])?;
      dir.add_tag(TiffRootTag::BlackLevels, [9_u16, 10_u16])?;
      dir.add_tag(TiffRootTag::WhiteLevel, [11_u16])?;
      dir.add_tag(TiffRootTag::BitsPerSample, [12_u32])?;
      dir.add_tag(TiffRootTag::ResolutionUnit, [-5_i32])?;
      dir.add_tag(TiffRootTag::Artist, "AT")?;
      dir.build()?
    };

    tiff.build(ifd_offset)?;

    //assert!(TiffReader::is_tiff(&mut output) == true);

    let mut garbage_output: Vec<u8> = Vec::new();
    garbage_output.push(0x4a); // 1 byte garbage
    garbage_output.extend_from_slice(&output.into_inner());

    let mut garbage_output = Cursor::new(garbage_output);

    garbage_output.seek(SeekFrom::Start(1))?;

    let reader = TiffReader::new(&mut garbage_output, 1, Some(16))?; // 1 byte offset correction

    assert_eq!(reader.root_ifd().entry_count(), 7);
    assert!(reader.root_ifd().get_entry(TiffRootTag::WhiteLevel).is_some());

    assert!(matches!(
      reader.root_ifd().get_entry(TiffRootTag::ExifIFDPointer).unwrap().value,
      Value::Long { .. }
    ));
    assert!(matches!(
      reader.root_ifd().get_entry(TiffRootTag::WhiteLevel).unwrap().value,
      Value::Short { .. }
    ));

    Ok(())
  }
}
