// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use crate::{formats::bmff::BmffError, formats::tiff::TiffReader};

use super::super::{BoxHeader, FourCC, ReadBox, Result};
use serde::{Serialize, Deserialize};
use std::io::{Read, Seek, SeekFrom};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cmt3Box {
  pub header: BoxHeader,
  pub data: Vec<u8>,
  pub tiff: TiffReader,
}

impl Cmt3Box {
  pub const TYP: FourCC = FourCC::with(['C', 'M', 'T', '3']);
}

impl<R: Read + Seek> ReadBox<&mut R> for Cmt3Box {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {
    let current = reader.seek(SeekFrom::Current(0))?;
    let data_len = header.end_offset() - current;
    let mut data = vec![0; data_len as usize];
    reader.read_exact(&mut data)?;

    reader.seek(SeekFrom::Start(header.end_offset()))?;

    let tiff = TiffReader::new_with_buffer(&data, 0, None).map_err(|e| BmffError::Parse(e.to_string()))?;

    Ok(Self { header, data, tiff })
  }
}
