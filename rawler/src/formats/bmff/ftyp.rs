// SPDX-License-Identifier: MIT
// Copyright 2020 Alfred Gutierrez
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::{BmffError, BoxHeader, FourCC, ReadBox, Result};
use byteorder::{BigEndian, ReadBytesExt};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek};

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
    // The compatible brands follow the major brand and the minor version
    let payload = header.size - header.header_len;
    if payload < 8 || payload % 4 != 0 {
      return Err(BmffError::Parse(format!("invalid ftyp size {}", header.size)));
    }
    let brand_count = (payload - 8) / 4;

    let mut brands = Vec::new();
    for _ in 0..brand_count {
      let b = reader.read_u32::<BigEndian>()?;
      brands.push(From::from(b));
    }

    header.finish(&mut *reader)?;

    Ok(Self {
      header,
      major_brand: From::from(major),
      minor_version: minor,
      compatible_brands: brands,
    })
  }
}
