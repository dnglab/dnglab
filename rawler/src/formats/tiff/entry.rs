// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::io::{Read, Seek};

use log::debug;
use serde::{Deserialize, Serialize};

use crate::{
  bits::Endian,
  formats::tiff::{Rational, SRational, TiffAscii, TiffError, Value, apply_corr, offset_add, reader::ReadByteOrder},
};

use super::{Result, reader::EndianReader};

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
const TYPE_IFD: u16 = 13;

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
  /// The u32 at index `idx`, None if the data is too short
  pub fn get_u32(&self, idx: usize) -> Option<u32> {
    let start = idx.checked_mul(4)?;
    Some(self.endian.read_u32(self.data.get(start..start.checked_add(4)?)?, 0))
  }

  /// The u16 at index `idx`, None if the data is too short
  pub fn get_u16(&self, idx: usize) -> Option<u16> {
    let start = idx.checked_mul(2)?;
    Some(self.endian.read_u16(self.data.get(start..start.checked_add(2)?)?, 0))
  }

  /// The u32 at index `idx`, 0 if the data is too short
  pub fn get_force_u32(&self, idx: usize) -> u32 {
    self.get_u32(idx).unwrap_or_else(|| {
      log::error!("TIFF raw entry index {} is out of range, the data has {} bytes", idx, self.data.len());
      0
    })
  }

  /// The u16 at index `idx`, 0 if the data is too short
  pub fn get_force_u16(&self, idx: usize) -> u16 {
    self.get_u16(idx).unwrap_or_else(|| {
      log::error!("TIFF raw entry index {} is out of range, the data has {} bytes", idx, self.data.len());
      0
    })
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
    // The tag is already read
    let pos = reader
      .position()?
      .checked_sub(2)
      .ok_or_else(|| TiffError::General("TIFF entry starts before the stream".into()))?;

    let typ = reader.read_u16()?;
    let count = reader.read_u32()?;

    // If we don't know the type assume byte data (undefined)
    let compat_typ = if typ == 0 || typ > TYPE_IFD { 7 } else { typ };

    let bytesize: u64 = (count as u64) << DATASHIFTS[compat_typ as usize];
    let offset: u32 = if bytesize <= 4 {
      reader
        .position()?
        .checked_sub(base)
        .ok_or_else(|| TiffError::Overflow(format!("TIFF entry is before the base offset {}", base)))?
    } else {
      apply_corr(reader.read_u32()?, corr)?
    };

    debug!(
      "Tag: {:#x}, Typ: {:#x}, count: {}, offset: {}, base: {}, corr: {}",
      tag, typ, count, offset, base, corr
    );

    if offset == u32::MAX || base.checked_add(offset).is_none() {
      // We hit an invalid offset, ignoring this tag
      // This happens for Olympus E-P2 images in ImageProc IFD for example.
      return Err(TiffError::Overflow(format!("Offset {} is invalid for tag 0x{:X}", offset, tag)));
    }

    // The count comes from the file: check that the data is inside the stream
    // before allocating memory for it.
    let stream_len = reader.stream_len()?;
    if base as u64 + offset as u64 + bytesize > stream_len {
      return Err(TiffError::Overflow(format!(
        "Data of tag 0x{:X} ({} bytes at offset {}) exceeds the stream length {}",
        tag, bytesize, offset, stream_len
      )));
    }

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
      TYPE_LONG | TYPE_IFD => {
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
        for chunk in tmp.chunks_exact(2) {
          v.push(Rational::new(chunk[0], chunk[1]));
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
        for chunk in tmp.chunks_exact(2) {
          v.push(SRational::new(chunk[0], chunk[1]));
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
    reader.goto(offset_add(pos, 12)?)?; // Size of IFD entry
    Ok(entry)
  }

  pub fn type_name(&self) -> String {
    self.value.value_type_name()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::io::Cursor;

  #[test]
  fn ifd_type_is_read_as_long_offsets() {
    // tag 0x14a, type IFD (13), count 2, offset 14, then two offsets
    let mut data = vec![0x4a, 0x01, 13, 0, 2, 0, 0, 0, 14, 0, 0, 0];
    data.extend_from_slice(&[0, 0]);
    data.extend_from_slice(&[0x10, 0, 0, 0, 0x20, 0, 0, 0]);
    let mut cursor = Cursor::new(data);
    let mut reader = EndianReader::new(&mut cursor, Endian::Little);
    let tag = reader.read_u16().unwrap();
    let entry = Entry::parse(&mut reader, 0, 0, tag).unwrap();
    assert_eq!(entry.value, Value::Long(vec![0x10, 0x20]));
  }

  #[test]
  fn raw_entry_index_out_of_range() {
    let entry = Entry {
      tag: 0x100,
      value: Value::Undefined(vec![1, 2, 3, 4, 5, 6]),
      embedded: Some(0),
    };
    let raw = RawEntry {
      entry: &entry,
      endian: Endian::Little,
      data: vec![1, 2, 3, 4, 5, 6],
    };
    assert_eq!(raw.get_u16(2), Some(0x0605));
    assert_eq!(raw.get_u16(3), None);
    assert_eq!(raw.get_u32(0), Some(0x04030201));
    assert_eq!(raw.get_u32(1), None);
    assert_eq!(raw.get_force_u32(1), 0);
    assert_eq!(raw.get_force_u16(usize::MAX), 0);
  }

  #[test]
  fn huge_count_is_rejected_before_allocation() {
    // tag 0x100, type DOUBLE (12), count 0xFFFFFFFF, offset 12
    let data = vec![0x00, 0x01, 12, 0, 0xFF, 0xFF, 0xFF, 0xFF, 12, 0, 0, 0, 0, 0, 0, 0];
    let mut cursor = Cursor::new(data);
    let mut reader = EndianReader::new(&mut cursor, Endian::Little);
    let tag = reader.read_u16().unwrap();
    assert!(matches!(Entry::parse(&mut reader, 0, 0, tag), Err(TiffError::Overflow(_))));
  }
}
