// SPDX-License-Identifier: MIT
// Copyright 2020 Alfred Gutierrez
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::{BmffError, BoxHeader, FourCC, ReadBox, Result};
use byteorder::{BigEndian, ReadBytesExt};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FtypBox {
  pub header: BoxHeader,
  pub major_brand: FourCC,
  pub minor_version: u32,
  pub compatible_brands: Vec<FourCC>,
}

impl FtypBox {
  pub const TYP: FourCC = FourCC::with(['f', 't', 'y', 'p']);
}

impl<R: Read + Seek> ReadBox<&mut R> for FtypBox {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {
    let major = reader.read_u32::<BigEndian>()?;
    let minor = reader.read_u32::<BigEndian>()?;
    if header.size % 4 != 0 {
      return Err(BmffError::Parse("invalid ftyp size".into()));
    }
    // `size - 16` underflows for a corrupt ftyp smaller than its fixed header
    // (8) + major (4) + minor (4) = 16 bytes. A valid ftyp is always >= 16, so
    // this check never fires on well-formed input.
    let brand_count = header
      .size
      .checked_sub(16)
      .ok_or_else(|| BmffError::Parse("invalid ftyp size (too small)".into()))?
      / 4; // header + major + minor

    let mut brands = Vec::new();
    for _ in 0..brand_count {
      let b = reader.read_u32::<BigEndian>()?;
      brands.push(From::from(b));
    }

    reader.seek(SeekFrom::Start(header.end_offset()))?;

    Ok(Self {
      header,
      major_brand: From::from(major),
      minor_version: minor,
      compatible_brands: brands,
    })
  }
}
