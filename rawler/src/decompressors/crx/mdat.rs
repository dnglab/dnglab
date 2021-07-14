// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use byteorder::{BigEndian, ReadBytesExt};
use std::io::{Cursor, Read};

use super::Result;
use crate::decompressors::crx::CrxError;

#[derive(Debug, Clone)]
pub struct Tile {
  // Header fields
  pub ind: u16,
  pub size: u16,
  pub tile_size: usize,
  pub flags: u32,
  pub qp_data: Option<TileQPData>,
  // Calculated fields
  pub id: usize,
  pub counter: u32,
  pub tail_sign: u32,
  pub data_offset: usize,
  pub width: usize,
  pub height: usize,
  pub tiles_top: bool,
  pub tiles_bottom: bool,
  pub tiles_left: bool,
  pub tiles_right: bool,
  /// Planes for tile
  pub planes: Vec<Plane>,
}

impl Tile {
  pub fn new<R: Read>(id: usize, hdr: &mut R, ind: u16, tile_offset: usize) -> Result<Self> {
    let size = hdr.read_u16::<BigEndian>()?;
    let tile_size = hdr.read_u32::<BigEndian>()? as usize;
    let flags = hdr.read_u32::<BigEndian>()?;
    //let counter = flags >> 28;
    let counter = (flags >> 16) & 0xF;
    let tail_sign = flags & 0xFFFF;
    let qp_data = if size == 16 {
      let mdat_qp_data_size = hdr.read_u32::<BigEndian>()?;
      let mdat_extra_size = hdr.read_u16::<BigEndian>()?;
      let terminator = hdr.read_u16::<BigEndian>()?;
      assert!(terminator == 0);
      Some(TileQPData {
        mdat_qp_data_size,
        mdat_extra_size,
        terminator,
      })
    } else {
      None
    };

    // TODO check on release
    assert!((size == 8 && tail_sign == 0) || (size == 16 && tail_sign == 0x4000));

    Ok(Tile {
      id,
      ind,
      size,
      tile_size,
      flags,
      counter,
      tail_sign,
      data_offset: tile_offset,
      planes: vec![],
      height: 0,
      width: 0,
      qp_data,
      tiles_top: false,
      tiles_bottom: false,
      tiles_left: false,
      tiles_right: false,
    })
  }

  pub fn descriptor_line(&self) -> String {
    let extra_data = match self.qp_data.as_ref() {
      Some(qp_data) => {
        format!(
          " qp_data_size: {:#x} extra_size: {:#x}, terminator: {:#x}",
          qp_data.mdat_qp_data_size, qp_data.mdat_extra_size, qp_data.terminator
        )
      }
      None => String::new(),
    };
    format!(
      "Tile {:#x} size: {:#x} tile_size: {:#x} flags: {:#x} counter: {:#x} tail_sign: {:#x}{}",
      self.ind,
      self.size,
      self.tile_size,
      self.flags,
      self.counter,
      self.tail_sign,
      extra_data,
      //mdatQPDataSize.unwrap_or_default()
    )
  }

  /// Tile may contain some extra data for quantization
  pub fn extra_size(&self) -> usize {
    match self.qp_data.as_ref() {
      Some(qp_data) => qp_data.mdat_qp_data_size as usize + qp_data.mdat_extra_size as usize,
      None => 0,
    }
  }
}

#[derive(Debug, Clone)]
pub struct TileQPData {
  pub mdat_qp_data_size: u32,
  pub mdat_extra_size: u16,
  pub terminator: u16,
}

#[derive(Debug, Clone)]
pub struct Plane {
  // Header fields
  pub ind: u16,
  pub size: u16,
  pub plane_size: usize,
  pub flags: u32,
  // Calculated fields
  pub id: usize,
  pub counter: u32,
  pub support_partial: bool,
  pub rounded_bits_mask: i32,
  pub data_offset: usize,
  pub parent_offset: usize,
  /// List of subbands
  pub subbands: Vec<Subband>,
}

impl Plane {
  pub fn new<R: Read>(id: usize, hdr: &mut R, ind: u16, parent_offset: usize, plane_offset: usize) -> Result<Self> {
    let size = hdr.read_u16::<BigEndian>()?;
    let plane_size = hdr.read_u32::<BigEndian>()? as usize;
    let flags = hdr.read_u32::<BigEndian>()?;
    let counter = (flags >> 28) & 0xf; // 4 bits

    //let support_partial = (flags >> 27) & 0x1; // 1 bit
    let support_partial: bool = (flags & 0x8000000) != 0;
    let rounded_bits_mask = ((flags >> 25) & 0x3) as i32; // 2 bit
    assert!(flags & 0x00FFFFFF == 0);
    Ok(Plane {
      id,
      ind,
      size,
      plane_size,
      flags,
      counter,
      support_partial,
      rounded_bits_mask,
      data_offset: plane_offset,
      parent_offset,
      subbands: vec![],
    })
  }

  pub fn descriptor_line(&self) -> String {
    format!(
      "  Plane {:#x} size: {:#x} plane_size: {:#x} flags: {:#x} counter: {:#x} support_partial: {} rounded_bits: {:#x}",
      self.ind, self.size, self.plane_size, self.flags, self.counter, self.support_partial, self.rounded_bits_mask
    )
  }
}

#[derive(Debug, Clone)]
pub struct Subband {
  // Header fields
  pub ind: u16,
  pub size: u16,
  pub subband_size: usize,
  pub flags: u32,
  pub q_step_base: u32,
  pub q_step_multi: u16,
  // Calculated fields
  pub id: usize,
  pub counter: u32,
  pub support_partial: bool,
  pub q_param: u32,
  pub unknown: u32,
  pub data_offset: usize,
  pub parent_offset: usize,
  pub data_size: u64, // bit count?
  pub width: usize,
  pub height: usize,
}

impl Subband {
  pub fn new<R: Read>(id: usize, hdr: &mut R, ind: u16, parent_offset: usize, band_offset: usize) -> Result<Self> {
    let size = hdr.read_u16::<BigEndian>()?;
    let subband_size = hdr.read_u32::<BigEndian>()? as usize;

    assert!((size == 8 && ind == 0xFF03) || (size == 16 && ind == 0xFF13));

    let flags = hdr.read_u32::<BigEndian>()?;
    let counter = (flags >> 28) & 0xf; // 4 bits

    //let support_partial = (flags >> 27) & 0x1; // 1 bit
    let support_partial: bool = (flags & 0x8000000) != 0;
    let q_param = (flags >> 19) & 0xFF; // 8 bit qParam
    let unknown = flags & 0x7FFFF; // 19 bit, related to subband_size
    let mut q_step_base = 0;
    let mut q_step_multi = 0;
    if size == 16 {
      q_step_base = hdr.read_u32::<BigEndian>()?;
      q_step_multi = hdr.read_u16::<BigEndian>()?;
      let end_marker = hdr.read_u16::<BigEndian>()?;
      assert!(end_marker == 0);
    }
    //assert!(subband_size >= 0x7FFFF);
    let data_size: u64 = (subband_size as u32 - (flags & 0x7FFFF)) as u64;
    //let data_size: u64 = 0;
    //let band_height = tiles.last().unwrap().height;
    //let band_width = tiles.last().unwrap().width;

    Ok(Subband {
      id,
      ind,
      size,
      subband_size,
      flags,
      counter,
      support_partial,
      q_param,
      q_step_base,
      q_step_multi,
      unknown,
      data_offset: band_offset,
      parent_offset,
      data_size,
      width: 0,
      height: 0,
    })
  }

  // This is buggy and unsed anymore?
  pub fn data<'a>(&self, data: &'a [u8]) -> &'a [u8] {
    let offset = self.parent_offset + self.data_offset;
    &data[offset..offset + self.subband_size as usize]
  }

  pub fn descriptor_line(&self) -> String {
    format!(
      "    Subband {:#x} size: {:#x} subband_size: {:#x} flags: {:#x} counter: {:#x} support_partial: {} quant_value: {:#x} unknown: {:#x} qStepBase: {:#x} qStepMult: {:#x} ",
      self.ind, self.size, self.subband_size, self.flags, self.counter, self.support_partial, self.q_param, self.unknown, self.q_step_base, self.q_step_multi

    )
  }
}

fn next_indicator<T: Read>(hdr: &mut T) -> Result<u16> {
  hdr
    .read_u16::<BigEndian>()
    .map_err(|_| CrxError::General("Header indicator read failed".into()))
}

/// Parse MDAT header for structure of embedded data
pub(super) fn parse_header(mdat_hdr: &[u8]) -> Result<Vec<Tile>> {
  let mut hdr = Cursor::new(mdat_hdr);
  let mut tiles = Vec::new();
  let mut tile_offset: usize = 0;
  let mut plane_offset: usize = 0;
  let mut band_offset: usize = 0;

  let mut ind = next_indicator(&mut hdr)?;
  loop {
    match ind {
      0xff01 | 0xff11 => {
        let mut tile = Tile::new(tiles.len(), &mut hdr, ind, tile_offset)?;
        ind = next_indicator(&mut hdr)?;
        loop {
          match ind {
            0xff02 | 0xff12 => {
              let mut plane = Plane::new(tile.planes.len(), &mut hdr, ind, tile.data_offset, plane_offset)?;
              ind = next_indicator(&mut hdr)?;
              loop {
                match ind {
                  0xff03 | 0xff13 => {
                    let subband = Subband::new(plane.subbands.len(), &mut hdr, ind, tile.data_offset + plane.data_offset, band_offset)?;
                    band_offset += subband.subband_size;
                    plane.subbands.push(subband);
                    // Multi-tile files has no 0x0000 end marker, so we simulate it
                    // on an read error.
                    ind = next_indicator(&mut hdr).unwrap_or(0x0000);
                  }
                  _ => {
                    break;
                  }
                }
              }
              plane_offset += plane.plane_size as usize;
              band_offset = 0; // reset band offset
              tile.planes.push(plane);
            }
            _ => {
              break;
            }
          }
        }
        tile_offset += tile.tile_size;
        plane_offset = 0; // reset plane offset
        tiles.push(tile);
      }
      0x0000 => {
        break;
      }
      _ => {
        return Err(CrxError::General(format!("Unexpected header record marker: {:x?}", ind)));
      }
    }
  }
  Ok(tiles)
}
