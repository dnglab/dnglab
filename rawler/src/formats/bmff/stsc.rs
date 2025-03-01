// SPDX-License-Identifier: MIT
// Copyright 2020 Alfred Gutierrez
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::{BoxHeader, FourCC, ReadBox, Result, read_box_header_ext};
use byteorder::{BigEndian, ReadBytesExt};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct StscBox {
  pub header: BoxHeader,
  pub version: u8,
  pub flags: u32,
  pub entries: Vec<StscEntry>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct StscEntry {
  pub first_chunk: u32,
  pub samples_per_chunk: u32,
  pub sample_description_index: u32,
  pub first_sample: u32,
}

impl StscBox {
  pub const TYP: FourCC = FourCC::with(['s', 't', 's', 'c']);

  pub fn get_entry_for_sample(&self, sample: u32) -> &StscEntry {
    assert!(sample > 0, "Sample numbering starts with 1");
    assert_eq!(self.entries.is_empty(), false, "stsc box must contains at least one entry");
    assert_eq!(self.entries[0].first_sample, 1, "First entry must start with first sample");
    match self.entries.binary_search_by(|entry| entry.first_sample.cmp(&sample)) {
      Ok(i) => &self.entries[i],
      Err(i) => &self.entries[i - 1],
    }
  }
}

impl<R: Read + Seek> ReadBox<&mut R> for StscBox {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {
    let (version, flags) = read_box_header_ext(reader)?;

    let entry_count = reader.read_u32::<BigEndian>()?;
    let mut entries = Vec::with_capacity(entry_count as usize);

    // Reader for a single stsc entry
    let mut read_entry = || -> Result<StscEntry> {
      Ok(StscEntry {
        first_chunk: reader.read_u32::<BigEndian>()?,
        samples_per_chunk: reader.read_u32::<BigEndian>()?,
        sample_description_index: reader.read_u32::<BigEndian>()?,
        first_sample: 0,
      })
    };

    if entry_count > 0 {
      // Read first entry and hold it back
      let mut holdback = read_entry()?;
      holdback.first_sample = 1;

      for _ in 1..entry_count {
        let mut entry = read_entry()?;
        // Now we know the chunk count to calc the amount of samples in holdback
        entry.first_sample = holdback.first_sample + ((entry.first_chunk - holdback.first_sample) * holdback.samples_per_chunk);
        entries.push(holdback);
        holdback = entry;
      }
      // Finalize entry list
      entries.push(holdback);
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
