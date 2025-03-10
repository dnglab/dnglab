// SPDX-License-Identifier: MIT
// Copyright 2020 Alfred Gutierrez
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::{BmffError, BoxHeader, FourCC, ReadBox, Result, ext_cr3::cr3desc::Cr3DescBox, mvhd::MvhdBox, trak::TrakBox, vendor::VendorBox};
use log::debug;
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MoovBox {
  pub header: BoxHeader,
  pub mvhd: MvhdBox,

  //#[serde(skip_serializing_if = "Option::is_none")]
  //pub mvex: Option<MvexBox>,
  #[serde(rename = "trak")]
  pub traks: Vec<TrakBox>,

  // UUID Box 85c0b687-820f-11e0-8111-f4ce462b6a48
  pub cr3desc: Option<Cr3DescBox>,

  pub vendor: Vec<VendorBox>,
}

impl MoovBox {
  pub const TYP: FourCC = FourCC::with(['m', 'o', 'o', 'v']);
}

impl<R: Read + Seek> ReadBox<&mut R> for MoovBox {
  fn read_box(mut reader: &mut R, header: BoxHeader) -> Result<Self> {
    let mut mvhd = None;
    let mut traks = Vec::new();

    let mut cr3desc = None;

    let mut vendors = Vec::new();

    let mut current = reader.stream_position()?;

    while current < header.end_offset() {
      // get box?

      let header = BoxHeader::parse(&mut reader)?;

      match header.typ {
        MvhdBox::TYP => {
          mvhd = Some(MvhdBox::read_box(&mut reader, header)?);
        }
        TrakBox::TYP => {
          let trak = TrakBox::read_box(&mut reader, header)?;
          traks.push(trak);
        }

        _ => {
          if let Some(uuid) = header.uuid {
            if uuid == Uuid::from_bytes(Cr3DescBox::UUID) {
              cr3desc = Some(Cr3DescBox::read_box(&mut reader, header)?);
            }
          } else {
            debug!("Vendor box found in moov: {:?}", header.typ);
            let vendor = VendorBox::read_box(&mut reader, header)?;
            vendors.push(vendor);
          }
        }
      }

      current = reader.stream_position()?;
    }

    reader.seek(SeekFrom::Start(header.end_offset()))?;

    Ok(Self {
      header,
      mvhd: mvhd.ok_or_else(|| BmffError::Parse("mdhd box not found, corrupt file?".into()))?,
      traks,
      cr3desc,
      vendor: vendors,
    })
  }
}
