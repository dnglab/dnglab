use std::io::{Read, Seek, SeekFrom};


use serde::{Serialize};

use super::{read_box_header_ext, BoxHeader, FourCC, ReadBox, Result};

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct TkhdBox {
  pub header: BoxHeader,
  pub version: u8,
  pub flags: u32,
}

impl TkhdBox {
  pub const TYP: FourCC = FourCC::with(['t', 'k', 'h', 'd']);
}

impl<R: Read + Seek> ReadBox<&mut R> for TkhdBox {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {
    let (version, flags) = read_box_header_ext(reader)?;

    // TODO?

    reader.seek(SeekFrom::Start(header.end_offset()))?;

    Ok(Self {
      header,
      version,
      flags,
    })
  }
}
