// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::super::{BoxHeader, FourCC, ReadBox, Result};
use byteorder::{BigEndian, ReadBytesExt};
use serde::{Serialize, Deserialize};
use std::io::{Read, Seek, SeekFrom};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CcdtBox {
  pub header: BoxHeader,
  pub image_type: u64,
  pub dual_pixel: u32,
  pub trak_index: u32,
}

impl CcdtBox {
  pub const TYP: FourCC = FourCC::with(['C', 'C', 'D', 'T']);
}

impl<R: Read + Seek> ReadBox<&mut R> for CcdtBox {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {
    let image_type = reader.read_u64::<BigEndian>()?;
    let dual_pixel = reader.read_u32::<BigEndian>()?;
    let trak_index = reader.read_u32::<BigEndian>()?;

    reader.seek(SeekFrom::Start(header.end_offset()))?;

    Ok(Self {
      header,
      image_type,
      dual_pixel,
      trak_index,
    })
  }
}
