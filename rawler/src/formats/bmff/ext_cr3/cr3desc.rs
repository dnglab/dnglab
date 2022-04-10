// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::{
  super::{BmffError, BoxHeader, ReadBox, Result},
  cctp::CctpBox,
  cmt1::Cmt1Box,
  cmt2::Cmt2Box,
  cmt3::Cmt3Box,
  cmt4::Cmt4Box,
  cncv::CncvBox,
  cnop::CnopBox,
  ctbo::CtboBox,
  thmb::ThmbBox,
};
use crate::formats::bmff::{free::FreeBox, skip::SkipBox};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cr3DescBox {
  pub header: BoxHeader,
  pub cncv: CncvBox,
  pub cctp: CctpBox,
  pub ctbo: CtboBox,
  pub cmt1: Cmt1Box,
  pub cmt2: Cmt2Box,
  pub cmt3: Cmt3Box,
  pub cmt4: Cmt4Box,
  pub thmb: ThmbBox,
}

impl Cr3DescBox {
  //pub const TYP: FourCC = FourCC::with(['u', 'u', 'i', 'd']);
  pub const UUID: [u8; 16] = [0x85, 0xc0, 0xb6, 0x87, 0x82, 0x0f, 0x11, 0xe0, 0x81, 0x11, 0xf4, 0xce, 0x46, 0x2b, 0x6a, 0x48];
}

impl<R: Read + Seek> ReadBox<&mut R> for Cr3DescBox {
  fn read_box(mut reader: &mut R, header: BoxHeader) -> Result<Self> {
    let mut cncv = None;
    let mut cctp = None;
    let mut ctbo = None;
    let mut cmt1 = None;
    let mut cmt2 = None;
    let mut cmt3 = None;
    let mut cmt4 = None;
    let mut thmb = None;

    let mut current = reader.seek(SeekFrom::Current(0))?;

    while current < header.end_offset() {
      // get box?

      let header = BoxHeader::parse(&mut reader)?;

      match header.typ {
        CncvBox::TYP => {
          cncv = Some(CncvBox::read_box(&mut reader, header)?);
        }
        CctpBox::TYP => {
          cctp = Some(CctpBox::read_box(&mut reader, header)?);
        }
        CtboBox::TYP => {
          ctbo = Some(CtboBox::read_box(&mut reader, header)?);
        }
        Cmt1Box::TYP => {
          cmt1 = Some(Cmt1Box::read_box(&mut reader, header)?);
        }
        Cmt2Box::TYP => {
          cmt2 = Some(Cmt2Box::read_box(&mut reader, header)?);
        }
        Cmt3Box::TYP => {
          cmt3 = Some(Cmt3Box::read_box(&mut reader, header)?);
        }
        Cmt4Box::TYP => {
          cmt4 = Some(Cmt4Box::read_box(&mut reader, header)?);
        }
        CnopBox::TYP => {
          let _ignore = Some(CnopBox::read_box(&mut reader, header)?);
        }
        ThmbBox::TYP => {
          thmb = Some(ThmbBox::read_box(&mut reader, header)?);
        }
        SkipBox::TYP => {
          let _ignore = SkipBox::read_box(&mut reader, header)?;
        }
        FreeBox::TYP => {
          let _ignore = FreeBox::read_box(&mut reader, header)?;
        }

        _ => return Err(BmffError::Parse(format!("Unknown box in Cr3desc: {}", header.typ))),
      }

      current = reader.seek(SeekFrom::Current(0))?;
    }

    reader.seek(SeekFrom::Start(header.end_offset()))?;

    Ok(Self {
      header,
      cncv: cncv.ok_or_else(|| BmffError::Parse("cncv box not found, corrupt file?".into()))?,
      cctp: cctp.ok_or_else(|| BmffError::Parse("cctp box not found, corrupt file?".into()))?,
      ctbo: ctbo.ok_or_else(|| BmffError::Parse("ctbo box not found, corrupt file?".into()))?,
      cmt1: cmt1.ok_or_else(|| BmffError::Parse("cmt1 box not found, corrupt file?".into()))?,
      cmt2: cmt2.ok_or_else(|| BmffError::Parse("cmt2 box not found, corrupt file?".into()))?,
      cmt3: cmt3.ok_or_else(|| BmffError::Parse("cmt3 box not found, corrupt file?".into()))?,
      cmt4: cmt4.ok_or_else(|| BmffError::Parse("cmt4 box not found, corrupt file?".into()))?,
      thmb: thmb.ok_or_else(|| BmffError::Parse("thmb box not found, corrupt file?".into()))?,
    })
  }
}
