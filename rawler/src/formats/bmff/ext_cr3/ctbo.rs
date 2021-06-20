// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::super::{BoxHeader, FourCC, ReadBox, Result};
use serde::Serialize;
use std::io::{Read, Seek, SeekFrom};

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct CtboBox {
  pub header: BoxHeader,
}

impl CtboBox {
  pub const TYP: FourCC = FourCC::with(['C', 'T', 'B', 'O']);
}

impl<R: Read + Seek> ReadBox<&mut R> for CtboBox {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {
    // TODO: add CTBO records

    reader.seek(SeekFrom::Start(header.end_offset()))?;

    Ok(Self { header })
  }
}
