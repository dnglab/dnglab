// SPDX-License-Identifier: MIT
// Copyright 2020 Alfred Gutierrez
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::{read_box_header_ext, BoxHeader, FourCC, ReadBox, Result};
use byteorder::{BigEndian, ReadBytesExt};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct VmhdBox {
  pub header: BoxHeader,
  pub version: u8,
  pub flags: u32,
  pub graphics_mode: u16,
  pub op_color: RgbColor,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RgbColor {
  pub red: u16,
  pub green: u16,
  pub blue: u16,
}

impl VmhdBox {
  pub const TYP: FourCC = FourCC::with(['v', 'm', 'h', 'd']);
}

impl<R: Read + Seek> ReadBox<&mut R> for VmhdBox {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {
    let (version, flags) = read_box_header_ext(reader)?;

    let graphics_mode = reader.read_u16::<BigEndian>()?;
    let op_color = RgbColor {
      red: reader.read_u16::<BigEndian>()?,
      green: reader.read_u16::<BigEndian>()?,
      blue: reader.read_u16::<BigEndian>()?,
    };

    reader.seek(SeekFrom::Start(header.end_offset()))?;

    Ok(Self {
      header,
      version,
      flags,
      graphics_mode,
      op_color,
    })
  }
}
