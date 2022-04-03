// SPDX-License-Identifier: MIT
// Copyright 2020 Alfred Gutierrez
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::{read_box_header_ext, BoxHeader, FourCC, ReadBox, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HdlrBox {
  pub header: BoxHeader,
  pub version: u8,
  pub flags: u32,
}

impl HdlrBox {
  pub const TYP: FourCC = FourCC::with(['h', 'd', 'l', 'r']);
}

impl<R: Read + Seek> ReadBox<&mut R> for HdlrBox {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {
    let (version, flags) = read_box_header_ext(reader)?;

    // TODO?

    reader.seek(SeekFrom::Start(header.end_offset()))?;

    Ok(Self { header, version, flags })
  }
}
