// SPDX-License-Identifier: MIT
// Copyright 2020 Alfred Gutierrez
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::{BoxHeader, FourCC, ReadBox, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MdatBox {
  pub header: BoxHeader,
}

impl MdatBox {
  pub const TYP: FourCC = FourCC::with(['m', 'd', 'a', 't']);
}

impl<R: Read + Seek> ReadBox<&mut R> for MdatBox {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {
    header.finish(&mut *reader)?;

    Ok(Self { header })
  }
}
