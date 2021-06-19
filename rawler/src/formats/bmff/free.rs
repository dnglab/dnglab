use std::{io::{Read, Seek, SeekFrom}};


use serde::{Serialize};

use super::{BoxHeader, FourCC, ReadBox, Result};

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct FreeBox {
  pub header: BoxHeader,
}

impl FreeBox {
    pub const TYP: FourCC = FourCC::with(['f', 'r', 'e', 'e']);
}

impl<R: Read + Seek> ReadBox<&mut R> for FreeBox {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {

    reader.seek(SeekFrom::Start(header.end_offset()))?;

    Ok(Self {
      header,
    })
  }
}
