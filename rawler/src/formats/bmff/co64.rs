// SPDX-License-Identifier: MIT
// Copyright 2020 Alfred Gutierrez
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use byteorder::{BigEndian, ReadBytesExt};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};

use super::{read_box_header_ext, BoxHeader, FourCC, ReadBox, Result};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Co64Box {
  pub header: BoxHeader,
  pub version: u8,
  pub flags: u32,
  pub entries: Vec<u64>,
}

impl Co64Box {
  pub const TYP: FourCC = FourCC::with(['c', 'o', '6', '4']);
}

impl<R: Read + Seek> ReadBox<&mut R> for Co64Box {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {
    let (version, flags) = read_box_header_ext(reader)?;

    let entry_count = reader.read_u32::<BigEndian>()?;
    let mut entries = Vec::with_capacity(entry_count as usize);
    for _i in 0..entry_count {
      let chunk_offset = reader.read_u64::<BigEndian>()?;
      entries.push(chunk_offset);
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
