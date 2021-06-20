// SPDX-License-Identifier: MIT
// Copyright 2020 Alfred Gutierrez
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::{read_box_header_ext, BoxHeader, FourCC, ReadBox, Result};
use byteorder::{BigEndian, ReadBytesExt};
use serde::Serialize;
use std::io::{Read, Seek, SeekFrom};

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct StscBox {
  pub header: BoxHeader,
  pub version: u8,
  pub flags: u32,
  pub entries: Vec<StscEntry>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct StscEntry {
  pub first_chunk: u32,
  pub samples_per_chunk: u32,
  pub sample_description_index: u32,
  pub first_sample: u32,
}

impl StscBox {
  pub const TYP: FourCC = FourCC::with(['s', 't', 's', 'c']);
}

impl<R: Read + Seek> ReadBox<&mut R> for StscBox {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {
    let (version, flags) = read_box_header_ext(reader)?;

    let entry_count = reader.read_u32::<BigEndian>()?;
    let mut entries = Vec::with_capacity(entry_count as usize);
    for _ in 0..entry_count {
      let entry = StscEntry {
        first_chunk: reader.read_u32::<BigEndian>()?,
        samples_per_chunk: reader.read_u32::<BigEndian>()?,
        sample_description_index: reader.read_u32::<BigEndian>()?,
        first_sample: 0,
      };
      entries.push(entry);
    }

    let mut sample_id = 1;
    for i in 0..entry_count {
      let (first_chunk, samples_per_chunk) = {
        let mut entry = entries.get_mut(i as usize).unwrap();
        entry.first_sample = sample_id;
        (entry.first_chunk, entry.samples_per_chunk)
      };
      if i < entry_count - 1 {
        let next_entry = entries.get(i as usize + 1).unwrap();
        sample_id += (next_entry.first_chunk - first_chunk) * samples_per_chunk;
      }
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
