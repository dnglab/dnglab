// SPDX-License-Identifier: MIT
// Copyright 2020 Alfred Gutierrez
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::{read_box_header_ext, BoxHeader, FourCC, ReadBox, Result};
use byteorder::{BigEndian, ReadBytesExt};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct StszBox {
  pub header: BoxHeader,
  pub version: u8,
  pub flags: u32,
  pub sample_size: u32,
  pub sample_count: u32,
  pub sample_sizes: Vec<u32>,
}

impl StszBox {
  pub const TYP: FourCC = FourCC::with(['s', 't', 's', 'z']);

  pub fn sample_size(&self, sample: u32) -> u32 {
    if self.sample_size > 0 {
      self.sample_size
    } else {
      self.sample_sizes[sample as usize - 1]
    }
  }
}

impl<R: Read + Seek> ReadBox<&mut R> for StszBox {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {
    let (version, flags) = read_box_header_ext(reader)?;

    let sample_size = reader.read_u32::<BigEndian>()?;
    let sample_count = reader.read_u32::<BigEndian>()?;
    let mut sample_sizes = Vec::with_capacity(sample_count as usize);
    if sample_size == 0 {
      for _ in 0..sample_count {
        let sample_number = reader.read_u32::<BigEndian>()?;
        sample_sizes.push(sample_number);
      }
    } else {
      // If the sample_size is non-zero, it is the only sample_size
      sample_sizes.push(sample_size);
    }

    reader.seek(SeekFrom::Start(header.end_offset()))?;

    Ok(Self {
      header,
      version,
      flags,
      sample_size,
      sample_count,
      sample_sizes,
    })
  }
}
