// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::super::{BoxHeader, FourCC, ReadBox, Result};
use serde::Serialize;
use std::io::{Read, Seek, SeekFrom};

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct Cmt3Box {
  pub header: BoxHeader,
  #[serde(skip_serializing)]
  pub data: Vec<u8>,
}

impl Cmt3Box {
  pub const TYP: FourCC = FourCC::with(['C', 'M', 'T', '3']);
}

impl<R: Read + Seek> ReadBox<&mut R> for Cmt3Box {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {
    let current = reader.seek(SeekFrom::Current(0))?;
    let data_len = header.end_offset() - current;
    let mut data = Vec::with_capacity(data_len as usize);
    reader.read_exact(&mut data)?;

    reader.seek(SeekFrom::Start(header.end_offset()))?;

    Ok(Self { header, data })
  }
}
