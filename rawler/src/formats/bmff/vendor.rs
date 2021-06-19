use std::io::{Read, Seek, SeekFrom};

use serde::Serialize;

use super::{BoxHeader, ReadBox, Result};

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct VendorBox {
  pub header: BoxHeader,
}

impl<R: Read + Seek> ReadBox<&mut R> for VendorBox {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {
    reader.seek(SeekFrom::Start(header.offset + header.size))?;

    Ok(Self { header })
  }
}
