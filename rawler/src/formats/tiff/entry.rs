// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::io::{Read, Seek};

use log::debug;
use serde::{Deserialize, Serialize};

use crate::{
  bits::Endian,
  formats::tiff::{apply_corr, reader::ReadByteOrder, Rational, SRational, TiffAscii, Value},
};

use super::{reader::EndianReader, Result};

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
  pub tag: u16,
  pub value: Value,
  // Embedded value for writer, offset for reader
  // This is only None when building an IFD for writing.
  pub embedded: Option<u32>, // TODO: rename it
}

impl std::ops::Deref for Entry {
  type Target = Value;

  fn deref(&self) -> &Self::Target {
    &self.value
  }
}

pub struct RawEntry<'a> {
  pub entry: &'a Entry,
  pub endian: Endian,
  pub data: Vec<u8>,
}

impl<'a> RawEntry<'a> {
  pub fn get_force_u32(&self, idx: usize) -> u32 {
    self.endian.read_u32(&self.data, idx * 4)
  }

  pub fn get_force_u16(&self, idx: usize) -> u16 {
    self.endian.read_u16(&self.data, idx * 2)
  }
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

  /// Returns the offset
  /// It is already corrected by `corr` but needs to be summed
  /// with `base` offset.
  pub fn offset(&self) -> Option<usize> {
    self.embedded.map(|v| v as usize)
  }

  pub fn parse<R: Read + Seek>(reader: &mut EndianReader<R>, base: u32, corr: i32, tag: u16) -> Result<Entry> {
    let pos = reader.position()? - 2; // TODO -2 because tag is already read

    let typ = reader.read_u16()?;
    let count = reader.read_u32()?;

    debug!("Tag: {:#x}, Typ: {:#x}, count: {}", tag, typ, count);

    // If we don't know the type assume byte data (undefined)
    let compat_typ = if typ == 0 || typ > 12 { 7 } else { typ };

    let bytesize: usize = (count as usize) << DATASHIFTS[compat_typ as usize];
    let offset: u32 = if bytesize <= 4 {
      reader.position()? - base
    } else {
      apply_corr(reader.read_u32()?, corr)
    };

    reader.goto(base + offset)?;
    let entry = match typ {
      TYPE_BYTE => {
        let mut v = vec![0; count as usize];
        reader.read_u8_into(&mut v)?;
        Entry {
          tag,
          value: Value::Byte(v),
          embedded: Some(offset),
        }
      }
      TYPE_ASCII => {
        let mut v = vec![0; count as usize];
        reader.read_u8_into(&mut v)?;
        Entry {
          tag,
          value: Value::Ascii(TiffAscii::new_from_raw(&v)),
          embedded: Some(offset),
        }
      }
      TYPE_SHORT => {
        let mut v = vec![0; count as usize];
        reader.read_u16_into(&mut v)?;
        Entry {
          tag,
          value: Value::Short(v),
          embedded: Some(offset),
        }
      }
      TYPE_LONG => {
        let mut v = vec![0; count as usize];
        reader.read_u32_into(&mut v)?;
        Entry {
          tag,
          value: Value::Long(v),
          embedded: Some(offset),
        }
      }
      TYPE_RATIONAL => {
        let mut tmp = vec![0; count as usize * 2]; // Rational is 2x u32
        reader.read_u32_into(&mut tmp)?;

        let mut v = Vec::with_capacity(count as usize);
        for i in (0..count as usize).step_by(2) {
          v.push(Rational::new(tmp[i], tmp[i + 1]));
        }
        Entry {
          tag,
          value: Value::Rational(v),
          embedded: Some(offset),
        }
      }
      TYPE_SBYTE => {
        let mut v = vec![0; count as usize];
        reader.read_i8_into(&mut v)?;
        Entry {
          tag,
          value: Value::SByte(v),
          embedded: Some(offset),
        }
      }
      TYPE_UNDEFINED => {
        let mut v = vec![0; count as usize];
        reader.read_u8_into(&mut v)?;
        Entry {
          tag,
          value: Value::Undefined(v),
          embedded: Some(offset),
        }
      }
      TYPE_SSHORT => {
        let mut v = vec![0; count as usize];
        reader.read_i16_into(&mut v)?;
        Entry {
          tag,
          value: Value::SShort(v),
          embedded: Some(offset),
        }
      }
      TYPE_SLONG => {
        let mut v = vec![0; count as usize];
        reader.read_i32_into(&mut v)?;
        Entry {
          tag,
          value: Value::SLong(v),
          embedded: Some(offset),
        }
      }
      TYPE_SRATIONAL => {
        let mut tmp = vec![0; count as usize * 2]; // SRational is 2x i32
        reader.read_i32_into(&mut tmp)?;

        let mut v = Vec::with_capacity(count as usize);
        for i in (0..count as usize).step_by(2) {
          v.push(SRational::new(tmp[i], tmp[i + 1]));
        }
        Entry {
          tag,
          value: Value::SRational(v),
          embedded: Some(offset),
        }
      }
      TYPE_FLOAT => {
        let mut v = vec![0.0; count as usize];
        reader.read_f32_into(&mut v)?;
        Entry {
          tag,
          value: Value::Float(v),
          embedded: Some(offset),
        }
      }
      TYPE_DOUBLE => {
        let mut v = vec![0.0; count as usize];
        reader.read_f64_into(&mut v)?;
        Entry {
          tag,
          value: Value::Double(v),
          embedded: Some(offset),
        }
      }
      x => {
        let mut v = vec![0; count as usize];
        reader.read_u8_into(&mut v)?;
        Entry {
          tag,
          value: Value::Unknown(x, v),
          embedded: Some(offset),
        }
      }
    };
    reader.goto(pos + 12)?; // Size of IFD entry
    Ok(entry)
  }

  pub fn type_name(&self) -> String{
    self.value.value_type_name()
  }
}
