use std::io::{Read, Seek, SeekFrom};



use serde::{Serialize};

use super::{
  super::{ BoxHeader, FourCC, ReadBox, Result},
};

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct Cmt2Box {
  pub header: BoxHeader,
  #[serde(skip_serializing)]
  pub data: Vec<u8>,
}

impl Cmt2Box {
  pub const TYP: FourCC = FourCC::with(['C', 'M', 'T', '2']);
}

impl<R: Read + Seek> ReadBox<&mut R> for Cmt2Box {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {
    let current = reader.seek(SeekFrom::Current(0))?;
    let data_len = header.end_offset() - current;
    let mut data = Vec::with_capacity(data_len as usize);
    reader.read_exact(&mut data)?;

    reader.seek(SeekFrom::Start(header.end_offset()))?;

    Ok(Self { header, data })
  }
}
