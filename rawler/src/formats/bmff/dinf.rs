use std::io::{Read, Seek, SeekFrom};


use serde::{Serialize};

use super::{BoxHeader, FourCC, ReadBox, Result};

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct DinfBox {
  pub header: BoxHeader,
}


impl DinfBox {
  pub const TYP: FourCC = FourCC::with(['d', 'i', 'n', 'f']);
}

impl<R: Read + Seek> ReadBox<&mut R> for DinfBox {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {


    reader.seek(SeekFrom::Start(header.end_offset()))?;

    Ok(Self {
      header,
    })
  }
}
