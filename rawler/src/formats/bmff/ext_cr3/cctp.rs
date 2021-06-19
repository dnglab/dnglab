use std::io::{Read, Seek, SeekFrom};

use byteorder::{BigEndian, ReadBytesExt};
use log::debug;
use serde::{Serialize};

use super::{
  super::{read_box_header_ext, BmffError, BoxHeader, FourCC, ReadBox, Result},
  ccdt::CcdtBox,
};

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
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

    let mut current = reader.seek(SeekFrom::Current(0))?;

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

      current = reader.seek(SeekFrom::Current(0))?;
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
