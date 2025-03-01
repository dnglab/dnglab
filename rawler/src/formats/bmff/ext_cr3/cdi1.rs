// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::{
  super::{BmffError, BoxHeader, FourCC, ReadBox, Result, read_box_header_ext},
  iad1::Iad1Box,
};
use log::debug;
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cdi1Box {
  pub header: BoxHeader,
  pub version: u8,
  pub flags: u32,
  pub iad1: Iad1Box,
}

impl Cdi1Box {
  pub const TYP: FourCC = FourCC::with(['C', 'D', 'I', '1']);
}

impl<R: Read + Seek> ReadBox<&mut R> for Cdi1Box {
  fn read_box(mut reader: &mut R, header: BoxHeader) -> Result<Self> {
    let (version, flags) = read_box_header_ext(reader)?;

    let mut iad1 = None;

    let mut current = reader.stream_position()?;

    while current < header.end_offset() {
      // get box?

      let header = BoxHeader::parse(&mut reader)?;

      match header.typ {
        Iad1Box::TYP => {
          iad1 = Some(Iad1Box::read_box(&mut reader, header)?);
        }

        _ => {
          debug!("Vendor box found in CDI1: {:?}", header.typ);
          return Err(BmffError::Parse(format!("Invalid box found in CDI1: {:?}", header.typ)));
        }
      }

      current = reader.stream_position()?;
    }

    reader.seek(SeekFrom::Start(header.end_offset()))?;

    Ok(Self {
      header,
      version,
      flags,
      iad1: iad1.ok_or_else(|| BmffError::Parse("IAD1 box not found, corrupt file?".into()))?,
    })
  }
}
