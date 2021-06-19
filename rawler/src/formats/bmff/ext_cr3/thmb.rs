use std::io::{Read, Seek, SeekFrom};



use serde::{Serialize};

use super::{
  super::{BoxHeader, FourCC, ReadBox, Result},
};

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct ThmbBox {
  pub header: BoxHeader,
}

impl ThmbBox {
  pub const TYP: FourCC = FourCC::with(['T', 'H', 'M', 'B']);
}

impl<R: Read + Seek> ReadBox<&mut R> for ThmbBox {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {
    // TODO

    reader.seek(SeekFrom::Start(header.end_offset()))?;

    Ok(Self { header })
  }
}
