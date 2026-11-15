// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::super::{BoxHeader, FourCC, ReadBox, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HevcBox {
  pub header: BoxHeader,
}

impl HevcBox {
  pub const TYP: FourCC = FourCC::with(['H', 'E', 'V', 'C']);
}

impl<R: Read + Seek> ReadBox<&mut R> for HevcBox {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {
    header.finish(&mut *reader)?;

    Ok(Self { header })
  }
}
