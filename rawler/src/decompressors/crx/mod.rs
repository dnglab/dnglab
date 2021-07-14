// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use bitstream_io::{BitRead, BitReader};
use log::debug;
use std::io::Cursor;
use thiserror::Error;

use crate::formats::bmff::ext_cr3::cmp1::Cmp1Box;

use self::mdat::Tile;

mod decoder;
mod mdat;

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

  /// Create new codec parameters
  pub fn new(cmp1: &Cmp1Box) -> Result<Self> {
    const INCR_BIT_TABLE: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 1, 0];

    if cmp1.n_planes != 4 {
      return Err(CrxError::General(format!("Plane configration {} is not supported", cmp1.n_planes)));
    }

    let tile_cols: usize = (cmp1.f_width / cmp1.tile_width) as usize;
    let tile_rows: usize = (cmp1.f_height / cmp1.tile_height) as usize;
    assert!(tile_cols > 0);
    assert!(tile_rows > 0);

    let params = Self {
      sample_precision: cmp1.n_bits as u8 + INCR_BIT_TABLE[4 * cmp1.enc_type as usize + 2] + 1,
      image_width: cmp1.f_width as usize,
      image_height: cmp1.f_height as usize,
      plane_count: cmp1.n_planes as u8,
      plane_width: if cmp1.n_planes == 4 {
        cmp1.f_width as usize / tile_cols / 2
      } else {
        cmp1.f_width as usize / tile_cols
      },
      plane_height: if cmp1.n_planes == 4 {
        cmp1.f_height as usize / tile_rows / 2
      } else {
        cmp1.f_height as usize / tile_rows
      },
      // 3 bands per level + one last LL
      // only 1 band for zero levels (uncompressed)
      subband_count: 3 * cmp1.image_levels as u8 + 1,
      levels: cmp1.image_levels as u8,
      n_bits: cmp1.n_bits as u8,
      enc_type: cmp1.enc_type as u8,
      tile_cols,
      tile_rows,
      tile_width: cmp1.tile_width as usize,
      tile_height: cmp1.tile_height as usize,
      mdat_hdr_size: cmp1.mdat_hdr_size,
    };

    if params.tile_cols > 0xff {
      return Err(CrxError::General(format!("Tile column count {} is not supported", tile_cols)));
    }
    if params.tile_rows > 0xff {
      return Err(CrxError::General(format!("Tile row count {} is not supported", tile_rows)));
    }
    if params.tile_width < 0x16 || params.tile_height < 0x16 || params.plane_width > 0x7FFF || params.plane_height > 0x7FFF {
      return Err(CrxError::General(format!("Invalid params for band decoding")));
    }

    Ok(params)
  }

  /// Process tiles and update values
  pub(super) fn process_tiles(&mut self, tiles: &mut Vec<Tile>) {
    let tile_count = tiles.len();
    // Update each tile
    for cur_tile in tiles.iter_mut() {
      if (cur_tile.id + 1) % self.tile_cols != 0 {
        // not the last tile in a tile row
        cur_tile.width = self.tile_width;
        if self.tile_cols > 1 {
          cur_tile.tiles_right = true;
          if cur_tile.id % self.tile_cols != 0 {
            // not the first tile in tile row
            cur_tile.tiles_left = true;
          }
        }
      } else {
        // last tile in a tile row
        cur_tile.width = self.tile_width;
        //tiles[curTile].width = self.plane_width - self.tile_width * (self.tile_cols - 1);
        if self.tile_cols > 1 {
          cur_tile.tiles_left = true;
        }
      }
      if (cur_tile.id) < (tile_count - self.tile_cols) {
        // in first tile row
        cur_tile.height = self.tile_height;
        if self.tile_rows > 1 {
          cur_tile.tiles_bottom = true;
          if cur_tile.id >= self.tile_cols {
            cur_tile.tiles_top = true;
          }
        }
      } else {
        // non first tile row
        cur_tile.height = self.tile_height;
        //tiles[curTile].height = self.plane_height - self.tile_height * (self.tile_rows - 1);
        if self.tile_rows > 1 {
          cur_tile.tiles_top = true;
        }
      }
    }
    // process subbands
    for tile in tiles {
      //println!("{}", tile.descriptor_line());
      //println!("Tw: {}, Th: {}", tile.width, tile.height);
      let mut plane_sizes = 0;
      for plane in &mut tile.planes {
        //println!("{}", plane.descriptor_line());
        let mut band_sizes = 0;
        for band in &mut plane.subbands {
          band_sizes += band.subband_size;
          //band.width = tile.width;
          //band.height = tile.height;
          band.width = self.plane_width;
          band.height = self.plane_height;
          // FIXME: ExCoef
          //println!("{}", band.descriptor_line());
          //println!("    Bw: {}, Bh: {}", band.width, band.height);
        }
        assert_eq!(plane.plane_size, band_sizes);
        plane_sizes += plane.plane_size;
      }
      // Tile may contain some extra bytes for quantization
      // This extra size must be subtracted before comaring to the
      // sum of plane sizes.
      assert_eq!(tile.tile_size - tile.extra_size(), plane_sizes);
    }
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
