// SPDX-License-Identifier: MIT
// Copyright 2020 Alfred Gutierrez
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::{read_box_header_ext, BoxHeader, FourCC, ReadBox, Result};
use byteorder::{BigEndian, ReadBytesExt};
use serde::Serialize;
use std::io::{Read, Seek, SeekFrom};

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct SttsBox {
  pub header: BoxHeader,
  pub version: u8,
  pub flags: u32,
  pub entries: Vec<SttsEntry>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct SttsEntry {
  pub sample_count: u32,
  pub sample_delta: u32,
}

impl SttsBox {
  pub const TYP: FourCC = FourCC::with(['s', 't', 't', 's']);
}

impl<R: Read + Seek> ReadBox<&mut R> for SttsBox {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {
    let (version, flags) = read_box_header_ext(reader)?;

    let entry_count = reader.read_u32::<BigEndian>()?;
    let mut entries = Vec::with_capacity(entry_count as usize);
    for _i in 0..entry_count {
      let entry = SttsEntry {
        sample_count: reader.read_u32::<BigEndian>()?,
        sample_delta: reader.read_u32::<BigEndian>()?,
      };
      entries.push(entry);
    }

    reader.seek(SeekFrom::Start(header.end_offset()))?;

    Ok(Self {
      header,
      version,
      flags,
      entries,
    })
  }
}
