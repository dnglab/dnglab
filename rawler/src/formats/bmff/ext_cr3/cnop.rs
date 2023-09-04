// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::super::{BoxHeader, FourCC, ReadBox, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CnopBox {
  pub header: BoxHeader,
  pub data: Vec<u8>,
}

impl CnopBox {
  pub const TYP: FourCC = FourCC::with(['C', 'N', 'O', 'P']);
}

impl<R: Read + Seek> ReadBox<&mut R> for CnopBox {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {
    let current = reader.stream_position()?;
    let data_len = header.end_offset() - current;
    let mut data = vec![0; data_len as usize];
    reader.read_exact(&mut data)?;

    reader.seek(SeekFrom::Start(header.end_offset()))?;

    Ok(Self { header, data })
  }
}
