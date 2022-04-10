// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::{
  super::{BmffError, BoxHeader, FourCC, ReadBox, Result},
  cdi1::Cdi1Box,
  cmp1::Cmp1Box,
  hevc::HevcBox,
  jpeg::JpegBox,
};
use crate::formats::bmff::free::FreeBox;
use byteorder::{BigEndian, ReadBytesExt};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CrawBox {
  pub header: BoxHeader,
  //pub version: u8,
  //pub flags: u32,
  pub reserved: [u8; 6],
  pub data_ref_index: u16,
  pub unknown1: [u8; 16],
  pub width: u16,
  pub height: u16,
  pub x_resolution: [u16; 2],
  pub y_resolution: [u16; 2],
  pub unknown2: u32,
  pub unknown3: u16,
  pub compr_name: [u8; 32],
  pub bit_depth: u16,
  pub unknown4: u16,
  pub img_flags: u16,
  pub img_type: u16,

  pub jpeg: Option<JpegBox>,
  pub hevc: Option<HevcBox>,
  pub cmp1: Option<Cmp1Box>,
  pub cdi1: Option<Cdi1Box>,
}

impl CrawBox {
  pub const TYP: FourCC = FourCC::with(['C', 'R', 'A', 'W']);
}

impl<R: Read + Seek> ReadBox<&mut R> for CrawBox {
  fn read_box(mut reader: &mut R, header: BoxHeader) -> Result<Self> {
    //let (version, flags) = read_box_header_ext(reader)?;

    let mut reserved = [0_u8; 6];
    reader.read_exact(&mut reserved)?;
    let data_ref_index = reader.read_u16::<BigEndian>()?;

    let mut unknown1 = [0; 16];
    reader.read_exact(&mut unknown1)?;

    //let _sample_entry = reader.read_u64::<BigEndian>()?;

    let width = reader.read_u16::<BigEndian>()?;
    let height = reader.read_u16::<BigEndian>()?;
    let x_resolution = [reader.read_u16::<BigEndian>()?, reader.read_u16::<BigEndian>()?];
    let y_resolution = [reader.read_u16::<BigEndian>()?, reader.read_u16::<BigEndian>()?];
    let unknown2 = reader.read_u32::<BigEndian>()?;
    let unknown3 = reader.read_u16::<BigEndian>()?;
    let mut compr_name = [0_u8; 32];
    reader.read_exact(&mut compr_name)?;
    let bit_depth = reader.read_u16::<BigEndian>()?;
    let unknown4 = reader.read_u16::<BigEndian>()?;
    let img_flags = reader.read_u16::<BigEndian>()?;
    let img_type = reader.read_u16::<BigEndian>()?;

    let mut jpeg = None;
    let mut hevc = None;
    let mut cmp1 = None;
    let mut cdi1 = None;

    let mut current = reader.seek(SeekFrom::Current(0))?;

    while current < header.end_offset() {
      // get box?

      let header = BoxHeader::parse(&mut reader)?;

      match header.typ {
        JpegBox::TYP => {
          jpeg = Some(JpegBox::read_box(&mut reader, header)?);
        }
        HevcBox::TYP => {
          hevc = Some(HevcBox::read_box(&mut reader, header)?);
        }
        Cmp1Box::TYP => {
          cmp1 = Some(Cmp1Box::read_box(&mut reader, header)?);
        }
        Cdi1Box::TYP => {
          cdi1 = Some(Cdi1Box::read_box(&mut reader, header)?);
        }
        FreeBox::TYP => {
          let _ = Some(FreeBox::read_box(&mut reader, header)?);
        }
        _ => {
          //debug!("Unknown box???");
          return Err(BmffError::Parse(format!("Invalid box found in CRAW: {:?}", header.typ)));
        }
      }

      current = reader.seek(SeekFrom::Current(0))?;
    }

    reader.seek(SeekFrom::Start(header.end_offset()))?;

    Ok(Self {
      header,
      //version,
      //flags,
      reserved,
      data_ref_index,
      unknown1,
      width,
      height,
      x_resolution,
      y_resolution,
      unknown2,
      unknown3,
      compr_name,
      bit_depth,
      unknown4,
      img_flags,
      img_type,
      jpeg,
      hevc,
      cmp1,
      cdi1,
    })
  }
}
