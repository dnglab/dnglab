use std::io::{Read, Seek, SeekFrom};


use serde::{Serialize};

use super::super::{BoxHeader, FourCC, ReadBox, Result};

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct CncvBox {
  pub header: BoxHeader,
  pub compressor: String,
}

impl CncvBox {
  pub const TYP: FourCC = FourCC::with(['C', 'N', 'C', 'V']);
}

impl<R: Read + Seek> ReadBox<&mut R> for CncvBox {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {
    let mut buf = [0_u8; 30];

    reader.read_exact(&mut buf)?;

    let compressor = String::from_utf8_lossy(&buf);

    reader.seek(SeekFrom::Start(header.end_offset()))?;

    Ok(Self {
      header,
      compressor: compressor.into(),
    })
  }
}
