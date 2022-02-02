// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::super::{BoxHeader, FourCC, ReadBox, Result};
use byteorder::{BigEndian, ReadBytesExt};
use serde::{Serialize, Deserialize};
use std::io::{Read, Seek, SeekFrom};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cmp1Box {
  pub header: BoxHeader,
  pub unknown1: i16,
  pub header_size: u16,
  pub version: u16,
  pub version_sub: u16,
  pub f_width: u32,
  pub f_height: u32,
  pub tile_width: u32,
  pub tile_height: u32,
  pub n_bits: u8,
  pub n_planes: u8,
  pub cfa_layout: u8,
  pub enc_type: u8,
  pub image_levels: u8,
  pub has_tile_cols: u8,
  pub has_tile_rows: u8,
  pub mdat_hdr_size: u32,
  pub ext_header: u32,
  pub median_bits: u8,
  // TODO: missing fields
}

impl Cmp1Box {
  pub const TYP: FourCC = FourCC::with(['C', 'M', 'P', '1']);
}

impl<R: Read + Seek> ReadBox<&mut R> for Cmp1Box {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {
    let unknown1 = reader.read_i16::<BigEndian>()?;
    let header_size = reader.read_u16::<BigEndian>()?;
    let version = reader.read_u16::<BigEndian>()?;
    let version_sub = reader.read_u16::<BigEndian>()?;
    let f_width = reader.read_u32::<BigEndian>()?;
    let f_height = reader.read_u32::<BigEndian>()?;
    let tile_width = reader.read_u32::<BigEndian>()?;
    let tile_height = reader.read_u32::<BigEndian>()?;
    let n_bits = reader.read_u8()?;

    let (n_planes, cfa_layout) = {
      let tmp = reader.read_u8()?;
      (tmp >> 4, tmp & 0xF)
    };

    let (enc_type, image_levels) = {
      let tmp = reader.read_u8()?;
      (tmp >> 4, tmp & 0xF)
    };

    let (has_tile_cols, has_tile_rows) = {
      let tmp = reader.read_u8()?;
      (tmp >> 7, tmp & 1)
    };

    let mdat_hdr_size = reader.read_u32::<BigEndian>()?;
    let ext_header = reader.read_u32::<BigEndian>()?; // 32

    // Median bit precision for enc_type 3 is not always same as
    // image bit precision. For CRM movie files, there is an extended header.
    let mut median_bits = n_bits;

    // CRM Movie file
    if (ext_header & 0x80000000 != 0) && n_planes == 4 && header.size >= 56 {
      let _unknow = reader.read_u32::<BigEndian>()?; // 36
      let _unknow = reader.read_u32::<BigEndian>()?; // 40
      let _unknow = reader.read_u32::<BigEndian>()?; // 44
      let _unknow = reader.read_u32::<BigEndian>()?; // 48
      let _unknow = reader.read_u32::<BigEndian>()?; // 52
      let use_median_bits = reader.read_u32::<BigEndian>()? & 0x40000000 != 0; // 56
      if use_median_bits && header.size > 84 {
        let _unknow = reader.read_u32::<BigEndian>()?; // 60
        let _unknow = reader.read_u32::<BigEndian>()?; // 64
        let _unknow = reader.read_u32::<BigEndian>()?; // 68
        let _unknow = reader.read_u32::<BigEndian>()?; // 72
        let _unknow = reader.read_u32::<BigEndian>()?; // 76
        let _unknow = reader.read_u32::<BigEndian>()?; // 80
        median_bits = reader.read_u8()?; // 84
      }
    }


    reader.seek(SeekFrom::Start(header.end_offset()))?;

    Ok(Self {
      header,
      unknown1,
      header_size,
      version,
      version_sub,
      f_width,
      f_height,
      tile_width,
      tile_height,
      n_bits,
      n_planes,
      cfa_layout,
      enc_type,
      image_levels,
      has_tile_cols,
      has_tile_rows,
      mdat_hdr_size,
      ext_header,
      median_bits,
    })
  }
}
