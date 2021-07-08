// SPDX-License-Identifier: MIT
// Copyright 2020 Alfred Gutierrez
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::{co64::Co64Box, stsc::StscBox, stsd::StsdBox, stsz::StszBox, stts::SttsBox, vendor::VendorBox, BmffError, BoxHeader, FourCC, ReadBox, Result};
use log::debug;
use serde::{Serialize, Deserialize};
use std::io::{Read, Seek, SeekFrom};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct StblBox {
  pub header: BoxHeader,
  pub stsd: StsdBox,
  pub stts: SttsBox,
  //pub ctts: Option<CttsBox>,
  pub stsc: StscBox,
  pub stsz: StszBox,
  pub co64: Option<Co64Box>,
  pub vendor: Vec<VendorBox>,
}

impl StblBox {
  pub const TYP: FourCC = FourCC::with(['s', 't', 'b', 'l']);
}

impl<R: Read + Seek> ReadBox<&mut R> for StblBox {
  fn read_box(mut reader: &mut R, header: BoxHeader) -> Result<Self> {
    let mut stsd = None;
    let mut stts = None;
    // let mut ctts = None;
    //let mut stss = None;
    let mut stsc = None;
    let mut stsz = None;
    //let mut stco = None;
    let mut co64 = None;

    let mut vendors = Vec::new();

    let mut current = reader.seek(SeekFrom::Current(0))?;

    while current < header.end_offset() {
      // get box?

      let header = BoxHeader::parse(&mut reader)?;

      //let ftyp = Some(FtypBox::read_box(&mut file, header)?);

      match header.typ {
        StsdBox::TYP => {
          stsd = Some(StsdBox::read_box(&mut reader, header)?);
        }
        SttsBox::TYP => {
          stts = Some(SttsBox::read_box(&mut reader, header)?);
        }
        StscBox::TYP => {
          stsc = Some(StscBox::read_box(&mut reader, header)?);
        }
        StszBox::TYP => {
          stsz = Some(StszBox::read_box(&mut reader, header)?);
        }
        Co64Box::TYP => {
          co64 = Some(Co64Box::read_box(&mut reader, header)?);
        }

        _ => {
          debug!("Vendor box found in stbl: {:?}", header.typ);
          let vendor = VendorBox::read_box(&mut reader, header)?;
          vendors.push(vendor);
        }
      }

      current = reader.seek(SeekFrom::Current(0))?;
    }

    reader.seek(SeekFrom::Start(header.end_offset()))?;

    Ok(Self {
      header,
      stsd: stsd.ok_or(BmffError::Parse("stsd box not found, corrupt file?".into()))?,
      stts: stts.ok_or(BmffError::Parse("stts box not found, corrupt file?".into()))?,
      stsc: stsc.ok_or(BmffError::Parse("stsc box not found, corrupt file?".into()))?,
      stsz: stsz.ok_or(BmffError::Parse("stsz box not found, corrupt file?".into()))?,
      co64,
      vendor: vendors,
    })
  }
}
