// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::{
    ffi::CString, fmt::Display,
  };

  use byteorder::{NativeEndian, WriteBytesExt};
  use serde::{Deserialize, Deserializer, Serialize, Serializer};


use super::{TiffError, Result, WriteAndSeek};

/// Type to represent tiff values of type `RATIONAL`
#[derive(Clone, Debug, Default, PartialEq, Copy)]
pub struct Rational {
  pub n: u32,
  pub d: u32,
}

impl Display for Rational {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{}/{}", self.n, self.d))
    }
}

impl Display for SRational {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
      f.write_fmt(format_args!("{}/{}", self.n, self.d))
  }
}

impl From<Rational> for f32 {
    fn from(v: Rational) -> Self {
        (v.n as f32) / (v.d as f32)
    }
}

impl From<SRational> for f32 {
    fn from(v: SRational) -> Self {
        (v.n as f32) / (v.d as f32)
    }
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
  pub fn as_string(&self) -> Option<&String> {
    match self {
      Self::Ascii(v) => Some(&v.strings()[0]),
      _ => None,
    }
  }

  pub fn get_string(&self) -> Result<&String> {
    match self {
      Self::Ascii(v) => Ok(&v.strings()[0]),
      _ => todo!(),
    }
  }

  pub fn get_usize(&self, idx: usize) -> Result<usize> {
    match self {
        Value::Byte(v) => Ok(v[idx] as usize),
        Value::Short(v) => Ok(v[idx] as usize),
        Value::Long(v) => Ok(v[idx] as usize),
        Value::SByte(v) => Ok(v[idx] as usize),
        Value::SShort(v) => Ok(v[idx] as usize),
        Value::SLong(v) => Ok(v[idx] as usize),
        _ => todo!(),
    }
  }

  pub fn get_u16(&self, idx: usize) -> Result<Option<u16>> {
    match self {
        Value::Byte(v) => Ok(v.get(idx).map(ToOwned::to_owned).map(Into::into)),
        Value::Short(v) => Ok(v.get(idx).map(ToOwned::to_owned).map(Into::into)),
        Value::Long(v) => Ok(v.get(idx).map(ToOwned::to_owned).map(|v| v as u16)),
        Value::SByte(v) => Ok(v.get(idx).map(ToOwned::to_owned).map(|v| v as u16)),
        Value::SShort(v) => Ok(v.get(idx).map(ToOwned::to_owned).map(|v| v as u16)),
        Value::SLong(v) => Ok(v.get(idx).map(ToOwned::to_owned).map(|v| v as u16)),
        _ => Err(TiffError::General(format!("Can not use get_u16() for tiff entry value {:?}", self)))
    }
  }

  pub fn get_u32(&self, idx: usize) -> Result<Option<u32>> {
    match self {
        Value::Byte(v) => Ok(v.get(idx).map(ToOwned::to_owned).map(Into::into)),
        Value::Short(v) => Ok(v.get(idx).map(ToOwned::to_owned).map(Into::into)),
        Value::Long(v) => Ok(v.get(idx).map(ToOwned::to_owned).map(Into::into)),
        Value::SByte(v) => Ok(v.get(idx).map(ToOwned::to_owned).map(|v| v as u32)),
        Value::SShort(v) => Ok(v.get(idx).map(ToOwned::to_owned).map(|v| v as u32)),
        Value::SLong(v) => Ok(v.get(idx).map(ToOwned::to_owned).map(|v| v as u32)),
        _ => Err(TiffError::General(format!("Can not use get_u32() for tiff entry value {:?}", self)))
    }
  }

  pub fn get_f32(&self, idx: usize) -> Result<Option<f32>> {
    match self {
        Value::Byte(v) => Ok(v.get(idx).map(ToOwned::to_owned).map(|v| v as f32)),
        Value::Short(v) => Ok(v.get(idx).map(ToOwned::to_owned).map(|v| v as f32)),
        Value::Long(v) => Ok(v.get(idx).map(ToOwned::to_owned).map(|v| v as f32)),
        Value::Rational(v) => Ok(v.get(idx).map(ToOwned::to_owned).map(Into::into)),
        Value::SByte(v) => Ok(v.get(idx).map(ToOwned::to_owned).map(|v| v as f32)),
        Value::SShort(v) => Ok(v.get(idx).map(ToOwned::to_owned).map(|v| v as f32)),
        Value::SLong(v) => Ok(v.get(idx).map(ToOwned::to_owned).map(|v| v as f32)),
        Value::SRational(v) => Ok(v.get(idx).map(ToOwned::to_owned).map(Into::into)),
        Value::Float(v) => Ok(v.get(idx).map(ToOwned::to_owned)),
        Value::Double(v) =>  Ok(v.get(idx).map(ToOwned::to_owned).map(|v| v as f32)),
        _ => todo!(),
    }
  }


  pub fn visual_rep(&self, limit: usize) -> String {
    match self {
        Value::Byte(v) => v.iter().take(limit).map(|a| format!("{:X}", a)).collect::<Vec::<String>>().join(" "),
        Value::Short(v) => v.iter().take(limit).map(|a| format!("{}", a)).collect::<Vec::<String>>().join(" "),
        Value::Long(v) => v.iter().take(limit).map(|a| format!("{}", a)).collect::<Vec::<String>>().join(" "),
        Value::Rational(v) => v.iter().take(limit).map(|a| format!("{}", a)).collect::<Vec::<String>>().join(" "),
        Value::SByte(v) => v.iter().take(limit).map(|a| format!("{}", a)).collect::<Vec::<String>>().join(" "),
        Value::SShort(v) => v.iter().take(limit).map(|a| format!("{}", a)).collect::<Vec::<String>>().join(" "),
        Value::SLong(v) => v.iter().take(limit).map(|a| format!("{}", a)).collect::<Vec::<String>>().join(" "),
        Value::SRational(v) => v.iter().take(limit).map(|a| format!("{}", a)).collect::<Vec::<String>>().join(" "),
        Value::Float(v) => v.iter().take(limit).map(|a| format!("{}", a)).collect::<Vec::<String>>().join(" "),
        Value::Double(v) =>  v.iter().take(limit).map(|a| format!("{}", a)).collect::<Vec::<String>>().join(" "),
        Value::Undefined(v) => v.iter().take(limit).map(|a| format!("{:X}", a)).collect::<Vec::<String>>().join(" "),
        Value::Unknown(_t, v) => v.iter().take(limit).map(|a| format!("{:X}", a)).collect::<Vec::<String>>().join(" "),
        Value::Ascii(v) => v.first().clone()
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

  pub fn value_type_name(&self) -> String {
    match self {
      Self::Byte(_) => { "BYTE".into()}
      Self::Ascii(_) => { "ASCII".into()}
      Self::Short(_) => { "SHORT".into()}
      Self::Long(_) => { "LONG".into()}
      Self::Rational(_) => { "RATIONAL".into()}
      Self::SByte(_)  => { "SBYTE".into()}
      Self::Undefined(_)=> { "UNDEF".into()}
      Self::SShort(_)  => { "SSHORT".into()}
      Self::SLong(_)=> { "SLONG".into()}
      Self::SRational(_)  => { "SRATIONAL".into()}
      Self::Float(_)=> { "FLOAT".into()}
      Self::Double(_)  => { "DOUBLE".into()}
      Self::Unknown(t, _)=> {format!("UNKNOWN ({})", t)}
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
    let s = ::std::str::from_utf8(&raw[0..nul_range_end]).unwrap_or("!!!INVALID UTF8!!!");
    strings.push(String::from(s));
    //nul_range_end += 1;
    // }

    Self { strings }
  }
}



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