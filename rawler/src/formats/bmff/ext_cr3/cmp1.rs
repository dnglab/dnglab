use std::io::{Read, Seek, SeekFrom};

use byteorder::{BigEndian, ReadBytesExt};
use serde::{Serialize};

use super::super::{BoxHeader, FourCC, ReadBox, Result};

#[derive(Debug, Clone, PartialEq, Serialize)]
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
  pub unknown2: u32,
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
    let unknown2 = reader.read_u32::<BigEndian>()?;

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
      unknown2,
    })
  }
}
