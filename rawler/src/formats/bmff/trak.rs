// SPDX-License-Identifier: MIT
// Copyright 2020 Alfred Gutierrez
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::{BmffError, BoxHeader, FourCC, ReadBox, Result, mdia::MdiaBox, tkhd::TkhdBox, vendor::VendorBox};
use log::debug;
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TrakBox {
  pub header: BoxHeader,
  pub tkhd: TkhdBox,

  //#[serde(skip_serializing_if = "Option::is_none")]
  //pub edts: Option<EdtsBox>,
  pub mdia: MdiaBox,
  pub vendor: Vec<VendorBox>,
}

impl TrakBox {
  pub const TYP: FourCC = FourCC::with(['t', 'r', 'a', 'k']);
}

impl<R: Read + Seek> ReadBox<&mut R> for TrakBox {
  fn read_box(mut reader: &mut R, header: BoxHeader) -> Result<Self> {
    let mut tkhd = None;
    let mut mdia = None;

    let mut vendors = Vec::new();

    let mut current = reader.stream_position()?;

    while current < header.end_offset() {
      // get box?

      let header = BoxHeader::parse(&mut reader)?;

      //let ftyp = Some(FtypBox::read_box(&mut file, header)?);

      match header.typ {
        TkhdBox::TYP => {
          tkhd = Some(TkhdBox::read_box(&mut reader, header)?);
        }
        MdiaBox::TYP => {
          mdia = Some(MdiaBox::read_box(&mut reader, header)?);
        }

        _ => {
          debug!("Vendor box found in trak: {:?}", header.typ);
          let vendor = VendorBox::read_box(&mut reader, header)?;
          vendors.push(vendor);
        }
      }

      current = reader.stream_position()?;
    }

    reader.seek(SeekFrom::Start(header.end_offset()))?;

    Ok(Self {
      header,
      tkhd: tkhd.ok_or_else(|| BmffError::Parse("tkhd box not found, corrupt file?".into()))?,
      mdia: mdia.ok_or_else(|| BmffError::Parse("mdia box not found, corrupt file?".into()))?,
      vendor: vendors,
    })
  }
}
