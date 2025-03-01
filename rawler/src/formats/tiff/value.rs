// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use byteorder::{NativeEndian, WriteBytesExt};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{convert::Infallible, ffi::CString, fmt::Display, io::Write, num::TryFromIntError};

use super::{Result, TiffError};

/// Type to represent tiff values of type `RATIONAL`
#[derive(Clone, Debug, Default, Copy)]
pub struct Rational {
  pub n: u32,
  pub d: u32,
}

impl PartialEq for Rational {
  fn eq(&self, other: &Self) -> bool {
    let n1: u64 = self.n as u64 * other.d as u64;
    let n2: u64 = self.d as u64 * other.n as u64;
    n1.eq(&n2)
  }
}

impl Eq for Rational {}

#[allow(clippy::non_canonical_partial_ord_impl)]
impl PartialOrd for Rational {
  fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
    let n1: u64 = self.n as u64 * other.d as u64;
    let n2: u64 = self.d as u64 * other.n as u64;
    Some(n1.cmp(&n2))
  }
}

impl Ord for Rational {
  fn cmp(&self, other: &Self) -> std::cmp::Ordering {
    let n1: u64 = self.n as u64 * other.d as u64;
    let n2: u64 = self.d as u64 * other.n as u64;
    n1.cmp(&n2)
  }
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

impl TryFrom<Rational> for usize {
  type Error = TryFromIntError;

  fn try_from(value: Rational) -> std::result::Result<Self, Self::Error> {
    Ok(((value.n as f32) / (value.d as f32)) as usize) // TODO
  }
}

impl TryFrom<Rational> for u8 {
  type Error = TryFromIntError;

  fn try_from(value: Rational) -> std::result::Result<Self, Self::Error> {
    Ok(((value.n as f32) / (value.d as f32)) as u8) // TODO
  }
}

impl TryFrom<Rational> for u16 {
  type Error = TryFromIntError;

  fn try_from(value: Rational) -> std::result::Result<Self, Self::Error> {
    Ok(((value.n as f32) / (value.d as f32)) as u16) // TODO
  }
}

impl TryFrom<Rational> for u32 {
  type Error = TryFromIntError;

  fn try_from(value: Rational) -> std::result::Result<Self, Self::Error> {
    Ok(((value.n as f32) / (value.d as f32)) as u32) // TODO
  }
}

impl TryFrom<Rational> for u64 {
  type Error = TryFromIntError;

  fn try_from(value: Rational) -> std::result::Result<Self, Self::Error> {
    Ok(((value.n as f32) / (value.d as f32)) as u64) // TODO
  }
}

impl TryFrom<Rational> for i8 {
  type Error = TryFromIntError;

  fn try_from(value: Rational) -> std::result::Result<Self, Self::Error> {
    Ok(((value.n as f32) / (value.d as f32)) as i8) // TODO
  }
}

impl TryFrom<Rational> for i16 {
  type Error = TryFromIntError;

  fn try_from(value: Rational) -> std::result::Result<Self, Self::Error> {
    Ok(((value.n as f32) / (value.d as f32)) as i16) // TODO
  }
}

impl TryFrom<Rational> for i32 {
  type Error = TryFromIntError;

  fn try_from(value: Rational) -> std::result::Result<Self, Self::Error> {
    Ok(((value.n as f32) / (value.d as f32)) as i32) // TODO
  }
}

impl TryFrom<Rational> for i64 {
  type Error = TryFromIntError;

  fn try_from(value: Rational) -> std::result::Result<Self, Self::Error> {
    Ok(((value.n as f32) / (value.d as f32)) as i64) // TODO
  }
}

impl TryFrom<Rational> for f32 {
  type Error = TryFromIntError;

  fn try_from(value: Rational) -> std::result::Result<Self, Self::Error> {
    Ok(((value.n as f32) / (value.d as f32)) as f32) // TODO
  }
}

impl TryFrom<SRational> for usize {
  type Error = TryFromIntError;

  fn try_from(value: SRational) -> std::result::Result<Self, Self::Error> {
    Ok(((value.n as f32) / (value.d as f32)) as usize) // TODO
  }
}

impl TryFrom<SRational> for u8 {
  type Error = TryFromIntError;

  fn try_from(value: SRational) -> std::result::Result<Self, Self::Error> {
    Ok(((value.n as f32) / (value.d as f32)) as u8) // TODO
  }
}

impl TryFrom<SRational> for u16 {
  type Error = TryFromIntError;

  fn try_from(value: SRational) -> std::result::Result<Self, Self::Error> {
    Ok(((value.n as f32) / (value.d as f32)) as u16) // TODO
  }
}

impl TryFrom<SRational> for u32 {
  type Error = TryFromIntError;

  fn try_from(value: SRational) -> std::result::Result<Self, Self::Error> {
    Ok(((value.n as f32) / (value.d as f32)) as u32) // TODO
  }
}

impl TryFrom<SRational> for u64 {
  type Error = TryFromIntError;

  fn try_from(value: SRational) -> std::result::Result<Self, Self::Error> {
    Ok(((value.n as f32) / (value.d as f32)) as u64) // TODO
  }
}

impl TryFrom<SRational> for i8 {
  type Error = TryFromIntError;

  fn try_from(value: SRational) -> std::result::Result<Self, Self::Error> {
    Ok(((value.n as f32) / (value.d as f32)) as i8) // TODO
  }
}

impl TryFrom<SRational> for i16 {
  type Error = TryFromIntError;

  fn try_from(value: SRational) -> std::result::Result<Self, Self::Error> {
    Ok(((value.n as f32) / (value.d as f32)) as i16) // TODO
  }
}

impl TryFrom<SRational> for i32 {
  type Error = TryFromIntError;

  fn try_from(value: SRational) -> std::result::Result<Self, Self::Error> {
    Ok(((value.n as f32) / (value.d as f32)) as i32) // TODO
  }
}

impl TryFrom<SRational> for i64 {
  type Error = TryFromIntError;

  fn try_from(value: SRational) -> std::result::Result<Self, Self::Error> {
    Ok(((value.n as f32) / (value.d as f32)) as i64) // TODO
  }
}

impl TryFrom<SRational> for f32 {
  type Error = TryFromIntError;

  fn try_from(value: SRational) -> std::result::Result<Self, Self::Error> {
    Ok(((value.n as f32) / (value.d as f32)) as f32) // TODO
  }
}

impl From<u16> for Rational {
  fn from(value: u16) -> Self {
    Self::new(value as u32, 1)
  }
}

impl From<u8> for Rational {
  fn from(value: u8) -> Self {
    Self::new(value as u32, 1)
  }
}

impl From<u32> for Rational {
  fn from(value: u32) -> Self {
    Self::new(value, 1)
  }
}

impl Rational {
  pub fn new(n: u32, d: u32) -> Self {
    Self { n, d }
  }

  pub fn new_f32(n: f32, d: u32) -> Self {
    Self::new((n * d as f32) as u32, d)
  }

  pub fn new_f64(n: f64, d: u32) -> Self {
    Self::new((n * d as f64) as u32, d)
  }

  pub fn as_f32(&self) -> f32 {
    self.n as f32 / self.d as f32
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
    let values: Vec<&str> = s.split('/').collect();
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
#[derive(Clone, Debug, Default, Copy)]
pub struct SRational {
  pub n: i32,
  pub d: i32,
}

impl SRational {
  pub fn new(n: i32, d: i32) -> Self {
    Self { n, d }
  }
}

impl PartialEq for SRational {
  fn eq(&self, other: &Self) -> bool {
    let n1: i64 = self.n as i64 * other.d as i64;
    let n2: i64 = self.d as i64 * other.n as i64;
    n1.eq(&n2)
  }
}

impl Eq for SRational {}

#[allow(clippy::non_canonical_partial_ord_impl)]
impl PartialOrd for SRational {
  fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
    let n1: i64 = self.n as i64 * other.d as i64;
    let n2: i64 = self.d as i64 * other.n as i64;
    n1.partial_cmp(&n2)
  }
}

impl Ord for SRational {
  fn cmp(&self, other: &Self) -> std::cmp::Ordering {
    let n1: i64 = self.n as i64 * other.d as i64;
    let n2: i64 = self.d as i64 * other.n as i64;
    n1.cmp(&n2)
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
    let values: Vec<&str> = s.split('/').collect();
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

pub struct ValueConvertError(pub(crate) ());

impl From<TryFromIntError> for ValueConvertError {
  fn from(_: TryFromIntError) -> Self {
    todo!()
  }
}

impl From<Infallible> for ValueConvertError {
  fn from(_: Infallible) -> Self {
    todo!()
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
  pub fn long(v: u32) -> Self {
    Self::from(v)
  }

  pub fn short(v: u16) -> Self {
    Self::from(v)
  }

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

  pub fn get_data(&self) -> &Vec<u8> {
    match self {
      Value::Ascii(data) => data.as_bytes(),
      Value::Byte(data) => data,
      Value::Undefined(data) => data,
      Value::Unknown(_, data) => data,
      _ => {
        panic!("Unable to call get_data() on this value type");
      }
    }
  }

  pub fn force_usize(&self, idx: usize) -> usize {
    match self.get_usize(idx) {
      Ok(Some(v)) => v,
      Ok(None) => {
        log::error!("TIFF value index out of range, index is {} but length is {}", idx, self.count());
        log::error!("Backtrace:\n{:?}", backtrace::Backtrace::new());
        Default::default()
      }
      Err(_) => {
        log::error!("TIFF value cast error, but forced to default value!");
        log::error!("Backtrace:\n{:?}", backtrace::Backtrace::new());
        Default::default()
      }
    }
  }

  pub fn force_u8(&self, idx: usize) -> u8 {
    match self.get_u8(idx) {
      Ok(Some(v)) => v,
      Ok(None) => {
        log::error!("TIFF value index out of range, index is {} but length is {}", idx, self.count());
        log::error!("Backtrace:\n{:?}", backtrace::Backtrace::new());
        Default::default()
      }
      Err(_) => {
        log::error!("TIFF value cast error, but forced to default value!");
        log::error!("Backtrace:\n{:?}", backtrace::Backtrace::new());
        Default::default()
      }
    }
  }

  pub fn force_u16(&self, idx: usize) -> u16 {
    match self.get_u16(idx) {
      Ok(Some(v)) => v,
      Ok(None) => {
        log::error!("TIFF value index out of range, index is {} but length is {}", idx, self.count());
        log::error!("Backtrace:\n{:?}", backtrace::Backtrace::new());
        Default::default()
      }
      Err(_) => {
        log::error!("TIFF value cast error, but forced to default value!");
        log::error!("Backtrace:\n{:?}", backtrace::Backtrace::new());
        Default::default()
      }
    }
  }

  pub fn force_u32(&self, idx: usize) -> u32 {
    match self.get_u32(idx) {
      Ok(Some(v)) => v,
      Ok(None) => {
        log::error!("TIFF value index out of range, index is {} but length is {}", idx, self.count());
        log::error!("Backtrace:\n{:?}", backtrace::Backtrace::new());
        Default::default()
      }
      Err(_) => {
        log::error!("TIFF value cast error, but forced to default value!");
        log::error!("Backtrace:\n{:?}", backtrace::Backtrace::new());
        Default::default()
      }
    }
  }

  pub fn force_u64(&self, idx: usize) -> u64 {
    match self.get_u64(idx) {
      Ok(Some(v)) => v,
      Ok(None) => {
        log::error!("TIFF value index out of range, index is {} but length is {}", idx, self.count());
        log::error!("Backtrace:\n{:?}", backtrace::Backtrace::new());
        Default::default()
      }
      Err(_) => {
        log::error!("TIFF value cast error, but forced to default value!");
        log::error!("Backtrace:\n{:?}", backtrace::Backtrace::new());
        Default::default()
      }
    }
  }

  pub fn force_i8(&self, idx: usize) -> i8 {
    match self.get_i8(idx) {
      Ok(Some(v)) => v,
      Ok(None) => {
        log::error!("TIFF value index out of range, index is {} but length is {}", idx, self.count());
        log::error!("Backtrace:\n{:?}", backtrace::Backtrace::new());
        Default::default()
      }
      Err(_) => {
        log::error!("TIFF value cast error, but forced to default value!");
        log::error!("Backtrace:\n{:?}", backtrace::Backtrace::new());
        Default::default()
      }
    }
  }

  pub fn force_i16(&self, idx: usize) -> i16 {
    match self.get_i16(idx) {
      Ok(Some(v)) => v,
      Ok(None) => {
        log::error!("TIFF value index out of range, index is {} but length is {}", idx, self.count());
        log::error!("Backtrace:\n{:?}", backtrace::Backtrace::new());
        Default::default()
      }
      Err(_) => {
        log::error!("TIFF value cast error, but forced to default value!");
        log::error!("Backtrace:\n{:?}", backtrace::Backtrace::new());
        Default::default()
      }
    }
  }

  pub fn force_i32(&self, idx: usize) -> i32 {
    match self.get_i32(idx) {
      Ok(Some(v)) => v,
      Ok(None) => {
        log::error!("TIFF value index out of range, index is {} but length is {}", idx, self.count());
        log::error!("Backtrace:\n{:?}", backtrace::Backtrace::new());
        Default::default()
      }
      Err(_) => {
        log::error!("TIFF value cast error, but forced to default value!");
        log::error!("Backtrace:\n{:?}", backtrace::Backtrace::new());
        Default::default()
      }
    }
  }

  pub fn force_i64(&self, idx: usize) -> i64 {
    match self.get_i64(idx) {
      Ok(Some(v)) => v,
      Ok(None) => {
        log::error!("TIFF value index out of range, index is {} but length is {}", idx, self.count());
        log::error!("Backtrace:\n{:?}", backtrace::Backtrace::new());
        Default::default()
      }
      Err(_) => {
        log::error!("TIFF value cast error, but forced to default value!");
        log::error!("Backtrace:\n{:?}", backtrace::Backtrace::new());
        Default::default()
      }
    }
  }

  pub fn force_f32(&self, idx: usize) -> f32 {
    match self.get_f32(idx) {
      Ok(Some(v)) => v,
      Ok(None) => {
        log::error!("TIFF value index out of range, index is {} but length is {}", idx, self.count());
        log::error!("Backtrace:\n{:?}", backtrace::Backtrace::new());
        Default::default()
      }
      Err(_) => {
        log::error!("TIFF value cast error, but forced to default value!");
        log::error!("Backtrace:\n{:?}", backtrace::Backtrace::new());
        Default::default()
      }
    }
  }

  pub fn get_usize(&self, idx: usize) -> std::result::Result<Option<usize>, ValueConvertError> {
    Ok(match self {
      Value::Byte(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Short(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Long(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SByte(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SShort(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SLong(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Rational(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SRational(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Float(v) => v.get(idx).map(ToOwned::to_owned).map(|x| x as usize),
      Value::Double(v) => v.get(idx).map(ToOwned::to_owned).map(|x| x as usize),
      Value::Ascii(_) => return Err(ValueConvertError(())),
      Value::Undefined(_) => return Err(ValueConvertError(())),
      Value::Unknown(_, _) => return Err(ValueConvertError(())),
    })
  }

  pub fn get_u8(&self, idx: usize) -> std::result::Result<Option<u8>, ValueConvertError> {
    Ok(match self {
      Value::Byte(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Short(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Long(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SByte(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SShort(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SLong(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Rational(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SRational(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Float(v) => v.get(idx).map(ToOwned::to_owned).map(|x| x as u8),
      Value::Double(v) => v.get(idx).map(ToOwned::to_owned).map(|x| x as u8),
      Value::Ascii(_) => return Err(ValueConvertError(())),
      Value::Undefined(_) => return Err(ValueConvertError(())),
      Value::Unknown(_, _) => return Err(ValueConvertError(())),
    })
  }

  pub fn get_u16(&self, idx: usize) -> std::result::Result<Option<u16>, ValueConvertError> {
    Ok(match self {
      Value::Byte(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Short(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Long(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SByte(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SShort(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SLong(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Rational(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SRational(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Float(v) => v.get(idx).map(ToOwned::to_owned).map(|x| x as u16),
      Value::Double(v) => v.get(idx).map(ToOwned::to_owned).map(|x| x as u16),
      Value::Ascii(_) => return Err(ValueConvertError(())),
      Value::Undefined(_) => return Err(ValueConvertError(())),
      Value::Unknown(_, _) => return Err(ValueConvertError(())),
    })
  }

  pub fn get_u32(&self, idx: usize) -> std::result::Result<Option<u32>, ValueConvertError> {
    Ok(match self {
      Value::Byte(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Short(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Long(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SByte(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SShort(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SLong(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Rational(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SRational(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Float(v) => v.get(idx).map(ToOwned::to_owned).map(|x| x as u32),
      Value::Double(v) => v.get(idx).map(ToOwned::to_owned).map(|x| x as u32),
      Value::Ascii(_) => return Err(ValueConvertError(())),
      Value::Undefined(_) => return Err(ValueConvertError(())),
      Value::Unknown(_, _) => return Err(ValueConvertError(())),
    })
  }

  pub fn get_u64(&self, idx: usize) -> std::result::Result<Option<u64>, ValueConvertError> {
    Ok(match self {
      Value::Byte(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Short(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Long(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SByte(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SShort(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SLong(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Rational(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SRational(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Float(v) => v.get(idx).map(ToOwned::to_owned).map(|x| x as u64),
      Value::Double(v) => v.get(idx).map(ToOwned::to_owned).map(|x| x as u64),
      Value::Ascii(_) => return Err(ValueConvertError(())),
      Value::Undefined(_) => return Err(ValueConvertError(())),
      Value::Unknown(_, _) => return Err(ValueConvertError(())),
    })
  }

  pub fn get_i8(&self, idx: usize) -> std::result::Result<Option<i8>, ValueConvertError> {
    Ok(match self {
      Value::Byte(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Short(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Long(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SByte(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SShort(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SLong(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Rational(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SRational(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Float(v) => v.get(idx).map(ToOwned::to_owned).map(|x| x as i8),
      Value::Double(v) => v.get(idx).map(ToOwned::to_owned).map(|x| x as i8),
      Value::Ascii(_) => return Err(ValueConvertError(())),
      Value::Undefined(_) => return Err(ValueConvertError(())),
      Value::Unknown(_, _) => return Err(ValueConvertError(())),
    })
  }

  pub fn get_i16(&self, idx: usize) -> std::result::Result<Option<i16>, ValueConvertError> {
    Ok(match self {
      Value::Byte(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Short(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Long(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SByte(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SShort(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SLong(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Rational(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SRational(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Float(v) => v.get(idx).map(ToOwned::to_owned).map(|x| x as i16),
      Value::Double(v) => v.get(idx).map(ToOwned::to_owned).map(|x| x as i16),
      Value::Ascii(_) => return Err(ValueConvertError(())),
      Value::Undefined(_) => return Err(ValueConvertError(())),
      Value::Unknown(_, _) => return Err(ValueConvertError(())),
    })
  }

  pub fn get_i32(&self, idx: usize) -> std::result::Result<Option<i32>, ValueConvertError> {
    Ok(match self {
      Value::Byte(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Short(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Long(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SByte(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SShort(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SLong(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Rational(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SRational(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Float(v) => v.get(idx).map(ToOwned::to_owned).map(|x| x as i32),
      Value::Double(v) => v.get(idx).map(ToOwned::to_owned).map(|x| x as i32),
      Value::Ascii(_) => return Err(ValueConvertError(())),
      Value::Undefined(_) => return Err(ValueConvertError(())),
      Value::Unknown(_, _) => return Err(ValueConvertError(())),
    })
  }

  pub fn get_i64(&self, idx: usize) -> std::result::Result<Option<i64>, ValueConvertError> {
    Ok(match self {
      Value::Byte(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Short(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Long(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SByte(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SShort(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SLong(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Rational(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SRational(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Float(v) => v.get(idx).map(ToOwned::to_owned).map(|x| x as i64),
      Value::Double(v) => v.get(idx).map(ToOwned::to_owned).map(|x| x as i64),
      Value::Ascii(_) => return Err(ValueConvertError(())),
      Value::Undefined(_) => return Err(ValueConvertError(())),
      Value::Unknown(_, _) => return Err(ValueConvertError(())),
    })
  }

  pub fn get_f32(&self, idx: usize) -> std::result::Result<Option<f32>, ValueConvertError> {
    Ok(match self {
      Value::Byte(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Short(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Long(v) => v.get(idx).map(ToOwned::to_owned).map(|x| x as f32),
      Value::SByte(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SShort(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SLong(v) => v.get(idx).map(ToOwned::to_owned).map(|x| x as f32),
      Value::Rational(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::SRational(v) => v.get(idx).map(ToOwned::to_owned).map(TryInto::try_into).transpose()?,
      Value::Float(v) => v.get(idx).map(ToOwned::to_owned).map(|x| x as f32),
      Value::Double(v) => v.get(idx).map(ToOwned::to_owned).map(|x| x as f32),
      Value::Ascii(_) => return Err(ValueConvertError(())),
      Value::Undefined(_) => return Err(ValueConvertError(())),
      Value::Unknown(_, _) => return Err(ValueConvertError(())),
    })
  }

  pub fn visual_rep(&self, limit: usize) -> String {
    match self {
      Value::Byte(v) => v.iter().take(limit).map(|a| format!("{:X}", a)).collect::<Vec<String>>().join(" "),
      Value::Short(v) => v.iter().take(limit).map(|a| format!("{}", a)).collect::<Vec<String>>().join(" "),
      Value::Long(v) => v.iter().take(limit).map(|a| format!("{}", a)).collect::<Vec<String>>().join(" "),
      Value::Rational(v) => v.iter().take(limit).map(|a| format!("{}", a)).collect::<Vec<String>>().join(" "),
      Value::SByte(v) => v.iter().take(limit).map(|a| format!("{}", a)).collect::<Vec<String>>().join(" "),
      Value::SShort(v) => v.iter().take(limit).map(|a| format!("{}", a)).collect::<Vec<String>>().join(" "),
      Value::SLong(v) => v.iter().take(limit).map(|a| format!("{}", a)).collect::<Vec<String>>().join(" "),
      Value::SRational(v) => v.iter().take(limit).map(|a| format!("{}", a)).collect::<Vec<String>>().join(" "),
      Value::Float(v) => v.iter().take(limit).map(|a| format!("{}", a)).collect::<Vec<String>>().join(" "),
      Value::Double(v) => v.iter().take(limit).map(|a| format!("{}", a)).collect::<Vec<String>>().join(" "),
      Value::Undefined(v) => v.iter().take(limit).map(|a| format!("{:X}", a)).collect::<Vec<String>>().join(" "),
      Value::Unknown(_t, v) => v.iter().take(limit).map(|a| format!("{:X}", a)).collect::<Vec<String>>().join(" "),
      Value::Ascii(v) => v.first().clone(),
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
      panic!("Embedded TIFF value data must not be empty");
      //return Err(TiffError::General("Embedded data is empty".into()));
    }
    if self.byte_size() > 4 {
      Err(TiffError::Overflow("Invalid data".to_string()))
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
        Self::Short(v) => Ok((v[0] as u32) | ((*v.get(1).unwrap_or(&0) as u32) << 16)),
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
        Self::SShort(v) => Ok((v[0] as u32) | ((*v.get(1).unwrap_or(&0) as u32) << 16)),
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

  pub fn write(&self, w: &mut dyn Write) -> Result<()> {
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
        w.write_all(val)?;
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
      Self::Unknown(t, _) => *t,
    }
  }

  pub fn value_type_name(&self) -> String {
    match self {
      Self::Byte(_) => "BYTE".into(),
      Self::Ascii(_) => "ASCII".into(),
      Self::Short(_) => "SHORT".into(),
      Self::Long(_) => "LONG".into(),
      Self::Rational(_) => "RATIONAL".into(),
      Self::SByte(_) => "SBYTE".into(),
      Self::Undefined(_) => "UNDEF".into(),
      Self::SShort(_) => "SSHORT".into(),
      Self::SLong(_) => "SLONG".into(),
      Self::SRational(_) => "SRATIONAL".into(),
      Self::Float(_) => "FLOAT".into(),
      Self::Double(_) => "DOUBLE".into(),
      Self::Unknown(t, _) => {
        format!("UNKNOWN ({})", t)
      }
    }
  }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TiffAscii {
  strings: Vec<String>,
  plain: Vec<u8>,
}

impl TiffAscii {
  pub fn new<T: AsRef<str>>(value: T) -> Self {
    Self {
      strings: vec![String::from(value.as_ref())],
      plain: Default::default(),
    }
  }

  pub fn new_from_vec(values: Vec<String>) -> Self {
    Self {
      strings: values,
      plain: Default::default(),
    }
  }

  pub fn strings(&self) -> &Vec<String> {
    &self.strings
  }

  pub fn as_bytes(&self) -> &Vec<u8> {
    &self.plain
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

    Self {
      strings,
      plain: Vec::from(raw),
    }
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
    Value::Ascii(TiffAscii::new(value))
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

impl From<Vec<i16>> for Value {
  fn from(value: Vec<i16>) -> Self {
    Value::SShort(value)
  }
}

impl From<&Vec<i16>> for Value {
  fn from(value: &Vec<i16>) -> Self {
    Value::SShort(value.clone())
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
  use super::*;

  #[test]
  fn rational_type_equal() {
    let a = Rational::new(257, 10);
    let b = Rational::new(257, 10);
    assert_eq!(a, b);

    let a = Rational::new(257, 5);
    let b = Rational::new(2570, 50);
    assert_eq!(a, b);
  }

  #[test]
  fn rational_type_nequal() {
    let a = Rational::new(257, 10);
    let b = Rational::new(2570, 10);
    assert_ne!(a, b);
  }

  #[test]
  fn rational_type_greater() {
    let a = Rational::new(200, 1);
    let b = Rational::new(300, 10);
    assert!(a > b);
  }

  #[test]
  fn rational_type_lesser() {
    let a = Rational::new(200, 1);
    let b = Rational::new(300, 10);
    assert!(b < a);
  }

  #[test]
  fn srational_type_equal() {
    let a = SRational::new(-257, 10);
    let b = SRational::new(-257, 10);
    assert_eq!(a, b);

    let a = SRational::new(-257, 10);
    let b = SRational::new(-2570, 100);
    assert_eq!(a, b);
  }

  #[test]
  fn srational_type_nequal() {
    let a = SRational::new(-257, 10);
    let b = SRational::new(-2570, 10);
    assert_ne!(a, b);
  }

  #[test]
  fn srational_type_greater() {
    let a = SRational::new(-200, 1);
    let b = SRational::new(-300, 10);
    assert!(a < b);
  }

  #[test]
  fn srational_type_lesser() {
    let a = SRational::new(-200, 1);
    let b = SRational::new(-300, 10);
    assert!(a < b);
  }
}
