// SPDX-License-Identifier: MIT
// Copyright 2020 Alfred Gutierrez
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::{BoxHeader, FourCC, ReadBox, Result};
use serde::Serialize;
use std::io::{Read, Seek};

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct FreeBox {
  pub header: BoxHeader,
}

impl FreeBox {
  pub const TYP: FourCC = FourCC::with(['f', 'r', 'e', 'e']);
}

impl<R: Read + Seek> ReadBox<&mut R> for FreeBox {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {
    header.finish(&mut *reader)?;

    Ok(Self { header })
  }
}
