use std::io::{Read, Seek, SeekFrom};


use serde::{Serialize};

use super::{ BoxHeader, FourCC, ReadBox, Result};

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct MdatBox {
    pub header: BoxHeader,
}

impl MdatBox {
    pub const TYP: FourCC = FourCC::with(['m', 'd', 'a', 't']);
}

impl<R: Read + Seek> ReadBox<&mut R> for MdatBox {
    fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {

        reader.seek(SeekFrom::Start(header.end_offset()))?;

      Ok(Self {
        header,
      })
    }
  }
