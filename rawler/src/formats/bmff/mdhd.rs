// SPDX-License-Identifier: MIT
// Copyright 2020 Alfred Gutierrez
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::{read_box_header_ext, BoxHeader, FourCC, ReadBox, Result};
use byteorder::{BigEndian, ReadBytesExt};
use serde::Serialize;
use std::io::{Read, Seek, SeekFrom};

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct MdhdBox {
  pub header: BoxHeader,
  pub version: u8,
  pub flags: u32,
  pub creation_time: u64,
  pub modification_time: u64,
  pub timescale: u32,
  pub duration: u64,
  pub language: String,
}

impl MdhdBox {
  pub const TYP: FourCC = FourCC::with(['m', 'd', 'h', 'd']);
}

impl<R: Read + Seek> ReadBox<&mut R> for MdhdBox {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {
    let (version, flags) = read_box_header_ext(reader)?;

    let (creation_time, modification_time, timescale, duration) = if version == 1 {
      (
        reader.read_u64::<BigEndian>()?,
        reader.read_u64::<BigEndian>()?,
        reader.read_u32::<BigEndian>()?,
        reader.read_u64::<BigEndian>()?,
      )
    } else {
      assert_eq!(version, 0);
      (
        reader.read_u32::<BigEndian>()? as u64,
        reader.read_u32::<BigEndian>()? as u64,
        reader.read_u32::<BigEndian>()?,
        reader.read_u32::<BigEndian>()? as u64,
      )
    };
    let _language_code = reader.read_u16::<BigEndian>()?;
    //let language = language_string(language_code); // TODO
    let language = String::from("FIXME");

    reader.seek(SeekFrom::Start(header.end_offset()))?;

    Ok(Self {
      header,
      version,
      flags,
      creation_time,
      modification_time,
      timescale,
      duration,
      language,
    })
  }
}
