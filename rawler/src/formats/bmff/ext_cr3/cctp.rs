// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use byteorder::{BigEndian, ReadBytesExt};
use log::debug;
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};

use super::{
  super::{BmffError, BoxHeader, FourCC, ReadBox, Result, read_box_header_ext},
  ccdt::CcdtBox,
};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CctpBox {
  pub header: BoxHeader,
  pub version: u8,
  pub flags: u32,
  pub unknown1: u32,
  pub lines: u32,
  pub ccdts: Vec<CcdtBox>,
}

impl CctpBox {
  pub const TYP: FourCC = FourCC::with(['C', 'C', 'T', 'P']);
}

impl<R: Read + Seek> ReadBox<&mut R> for CctpBox {
  fn read_box(mut reader: &mut R, header: BoxHeader) -> Result<Self> {
    let (version, flags) = read_box_header_ext(reader)?;
    let unknown1 = reader.read_u32::<BigEndian>()?;
    let lines = reader.read_u32::<BigEndian>()?;

    let mut ccdts = Vec::new();

    let mut current = reader.stream_position()?;

    while current < header.end_offset() {
      // get box?

      let header = BoxHeader::parse(&mut reader)?;

      match header.typ {
        CcdtBox::TYP => {
          let ccdt = CcdtBox::read_box(&mut reader, header)?;
          ccdts.push(ccdt);
        }

        _ => {
          debug!("Vendor box found in CCTP: {:?}", header.typ);
          return Err(BmffError::Parse(format!("Invalid box found in CCTP: {:?}", header.typ)));
        }
      }

      current = reader.stream_position()?;
    }

    reader.seek(SeekFrom::Start(header.end_offset()))?;

    Ok(Self {
      header,
      version,
      flags,
      unknown1,
      lines,
      ccdts,
    })
  }
}
