// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use bitstream_io::{BitRead, BitReader};
use byteorder::{BigEndian, ReadBytesExt};
use log::debug;
use std::io::{Cursor, Read, Seek};
use thiserror::Error;

use crate::formats::bmff::ext_cr3::cmp1::Cmp1Box;

mod decoder;
mod header;

/// BitPump for Big Endian bit streams
type BitPump<'a> = BitReader<Cursor<&'a [u8]>, bitstream_io::BigEndian>;

/// Error variants for compressor
#[derive(Debug, Error)]
pub enum CrxError {
  /// Overflow of input, size constraints...
  #[error("Overflow error: {}", _0)]
  Overflow(String),

  /// General error
  #[error("General error: {}", _0)]
  General(String),

  /// General error
  #[error("Unsupported format: {}", _0)]
  Unsupp(String),

  /// Error on internal cursor type
  #[error("I/O error")]
  Io(#[from] std::io::Error),
}

/// Result type for Compressor results
type Result<T> = std::result::Result<T, CrxError>;

/// Codec parameters for decoding
#[derive(Default, Debug)]
pub struct CodecParams {
  sample_precision: u8,
  image_width: usize,
  image_height: usize,
  plane_count: u8,
  plane_width: usize,
  plane_height: usize,
  subband_count: u8,
  levels: u8,
  n_bits: u8,
  enc_type: u8,
  tile_cols: usize,
  tile_rows: usize,
  tile_width: usize,
  tile_height: usize,
  mdat_hdr_size: u32,
}

impl CodecParams {
  #[inline(always)]
  fn get_header<'a>(&self, mdat: &'a [u8]) -> &'a [u8] {
    &mdat[..self.mdat_hdr_size as usize]
  }

  #[inline(always)]
  fn get_data<'a>(&self, mdat: &'a [u8]) -> &'a [u8] {
    &mdat[self.mdat_hdr_size as usize..]
  }

  fn resolution(&self) -> usize {
    self.image_width * self.image_height
  }
}

#[derive(Debug)]
pub struct Tile {
  pub id: usize,
  pub ind: u16,
  pub size: u16,
  pub tile_size: u32,
  pub flags: u32,
  pub counter: u32,
  pub tail_sign: u32,
  pub data_offset: usize,
  pub planes: Vec<Plane>,
  pub width: usize,
  pub height: usize,
  pub qp_data: Option<TileQPData>,
}

impl Tile {
  pub fn new<R: Read + Seek>(id: usize, hdr: &mut R, ind: u16, tile_offset: usize) -> Result<Self> {
    let size = hdr.read_u16::<BigEndian>().unwrap();
    let tile_size = hdr.read_u32::<BigEndian>().unwrap();
    let flags = hdr.read_u32::<BigEndian>().unwrap();
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
  pub fn extra_size(&self) -> u32 {
    match self.qp_data.as_ref() {
      Some(qp_data) => {
        qp_data.mdat_qp_data_size + qp_data.mdat_extra_size as u32
      }
      None => 0
    }
  }
}

#[derive(Debug)]
pub struct TileQPData {
  pub mdat_qp_data_size: u32,
  pub mdat_extra_size: u16,
  pub terminator: u16,
}

#[derive(Debug)]
pub struct Plane {
  pub id: usize,
  pub ind: u16,
  pub size: u16,
  pub plane_size: u32,
  pub flags: u32,
  pub counter: u32,
  pub support_partial: bool,
  pub rounded_bits_mask: i32,
  pub data_offset: usize,
  pub parent_offset: usize,
  pub subbands: Vec<Subband>,
}

impl Plane {
  pub fn new<R: Read + Seek>(id: usize, hdr: &mut R, ind: u16, parent_offset: usize, plane_offset: usize) -> Result<Self> {
    let size = hdr.read_u16::<BigEndian>().unwrap();
    let plane_size = hdr.read_u32::<BigEndian>().unwrap();
    let flags = hdr.read_u32::<BigEndian>().unwrap();
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

#[derive(Debug)]
pub struct Subband {
  pub id: usize,
  pub ind: u16,
  pub size: u16,
  pub subband_size: u32,
  pub flags: u32,
  pub counter: u32,
  pub support_partial: bool,
  pub q_param: u32,
  pub unknown: u32,
  pub q_step_base: u32,
  pub q_step_multi: u16,

  pub data_offset: usize,
  pub parent_offset: usize,
  pub data_size: u64, // bit count?
  pub width: usize,
  pub height: usize,
}

impl Subband {
  pub fn new<R: Read + Seek>(id: usize, hdr: &mut R, ind: u16, parent_offset: usize, band_offset: usize) -> Result<Self> {
    let size = hdr.read_u16::<BigEndian>().unwrap();
    let subband_size = hdr.read_u32::<BigEndian>().unwrap();

    assert!((size == 8 && ind == 0xFF03) || (size == 16 && ind == 0xFF13));

    let flags = hdr.read_u32::<BigEndian>().unwrap();
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
    let data_size: u64 = (subband_size - (flags & 0x7FFFF)) as u64;
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
/// Parameter for a single Subband
struct BandParam<'mdat> {
  subband_width: usize,
  subband_height: usize,
  rounded_bits_mask: i32,
  rounded_bits: i32,
  cur_line: usize,
  line_buf: Vec<i32>,
  line_len: usize,
  line0_pos: usize,
  line1_pos: usize,
  line2_pos: usize,

  s_param: u32,
  k_param: u32,
  supports_partial: bool,
  /// Holds the decoding buffer for a single row
  dec_buf: Vec<i32>,
  /// Bitstream from MDAT
  bitpump: BitPump<'mdat>,
}

impl<'mdat> BandParam<'mdat> {
  #[inline(always)]
  fn get_line0(&mut self, idx: usize) -> &mut i32 {
    &mut self.line_buf[self.line0_pos + idx]
  }

  #[inline(always)]
  fn get_line1(&mut self, idx: usize) -> &mut i32 {
    &mut self.line_buf[self.line1_pos + idx]
  }

  #[inline(always)]
  fn _get_line2(&mut self, idx: usize) -> &mut i32 {
    &mut self.line_buf[self.line2_pos + idx]
  }

  #[inline(always)]
  fn advance_buf0(&mut self) {
    self.line0_pos += 1;
    //self.buf0[self.line0_pos-1]
  }

  #[inline(always)]
  fn advance_buf1(&mut self) {
    self.line1_pos += 1;
    //self.buf1[self.line1_pos-1]
  }

  #[inline(always)]
  fn _advance_buf2(&mut self) {
    self.line2_pos += 1;
    //.buf2[self.line2_pos-1]
  }


  /// Return the positive number of 0-bits in bitstream.
  /// All 0-bits are consumed.
  #[inline(always)]
  fn bitstream_zeros(&mut self) -> Result<u32> {
    Ok(self.bitpump.read_unary1()?)
  }

  /// Return the requested bits
  // All bits are consumed.
  // The maximum number of bits are 32
  #[inline(always)]
  fn bitstream_get_bits(&mut self, bits: u32) -> Result<u32> {
    assert!(bits <= 32);
    Ok(self.bitpump.read(bits)?)
  }

  /// Get next error symbol
  /// This is Golomb-Rice encoded (not 100% sure)
  fn next_error_symbol(&mut self) -> Result<u32> {
    let mut bit_code = self.bitstream_zeros()?;
    if bit_code >= 41 {
      bit_code = self.bitstream_get_bits(21)?;
    } else if self.k_param > 0 {
      bit_code = self.bitstream_get_bits(self.k_param)? | (bit_code << self.k_param);
    }
    Ok(bit_code)
  }
}

pub fn decompress_crx_image(buf: &[u8], cmp1: &Cmp1Box) -> Result<Vec<u16>> {
  let image = CodecParams::new(cmp1)?;
  debug!("{:?}", image);
  let result = image.decode(buf)?;
  Ok(result)
}
