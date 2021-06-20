// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::super::{BoxHeader, FourCC, ReadBox, Result};
use byteorder::{BigEndian, ReadBytesExt};
use serde::Serialize;
use std::io::{Read, Seek, SeekFrom};

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct CtmdBox {
  pub header: BoxHeader,
  //pub version: u8,
  //pub flags: u32,
  pub reserved: [u8; 6],
  pub data_ref_index: u16,
  pub rec_count: u32,
  pub records: Vec<CtmdRecord>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct CtmdRecord {
  pub unknown1: u8, // 0x00, sometimes 0x01
  pub unknown2: u8, // 0x00, seomtimes 0x01
  pub rec_type: u16,
  pub rec_size: u32,
}

impl CtmdBox {
  pub const TYP: FourCC = FourCC::with(['C', 'T', 'M', 'D']);
}

impl<R: Read + Seek> ReadBox<&mut R> for CtmdBox {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {
    //let (version, flags) = read_box_header_ext(reader)?;

    //let mut reserved = [0_u8; 6];
    //reader.read_exact(&mut reserved)?;

    //let _reference_index = reader.read_u16::<BigEndian>()?;
    let mut reserved = [0_u8; 6];
    reader.read_exact(&mut reserved)?;
    let data_ref_index = reader.read_u16::<BigEndian>()?;
    let rec_count = reader.read_u32::<BigEndian>()?;

    let mut records = Vec::with_capacity(rec_count as usize);

    //let mut current = reader.seek(SeekFrom::Current(0))?;

    for _ in 0..rec_count {
      // get box?

      let record = CtmdRecord {
        unknown1: reader.read_u8()?,
        unknown2: reader.read_u8()?,
        rec_type: reader.read_u16::<BigEndian>()?,
        rec_size: reader.read_u32::<BigEndian>()?,
      };
      records.push(record);
      //current = reader.seek(SeekFrom::Current(0))?;
    }

    assert!(reader.seek(SeekFrom::Current(0))? == header.end_offset());

    reader.seek(SeekFrom::Start(header.end_offset()))?;

    Ok(Self {
      header,
      //version,
      //flags,
      reserved,
      data_ref_index,
      rec_count,
      records,
    })
  }
}
