// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

// Original Crx decoder crx.cpp was written by Alexey Danilchenko for libraw.
// Rewritten in Rust by Daniel Vogelbacher, based on logic found in
// crx.cpp and documentation done by Laurent Clévy (https://github.com/lclevy/canon_cr3).

// Crx is based on JPEG-LS from ITU T.78 and described in US patent US 2016/0323602 A1.
// It has two modes:
//  - Lossless compression
//  - Lossy compression
// For lossless (only LL band exists) and LL band from lossy compression,
// prediction is used combined with adaptive Golomb-Rice entropy encoding for compression.
// For lossy bands other than LL a special value rounding is introduced
// into Golomb-Rice encoded values.
//
// LL band compression uses for the first image line run-length encoding
// from JPEG-LS and adaptive Golomb-Rice encoding. For other lines,
// MED (Median Edge Detection) is added to the encoding routines.
//
// For Lossy compression, image input is wavelet-transformed into subbands.
// The LL band for low frequency part
// The LH band for horizontal-direction frequency characteristic
// The HL band for vertical-direction frequency characteristic
// The HH band for oblique-direction frequency characteristic
//
// Transformation is directed by i, number of wavelet transformations.
// Crx uses i=3, so the output is LL(3), LH(3), HL(3), HH(3), LH(2), HL(2), HH(2), LH(1), HL(1), HH(1)
//
// TODO: Subband encoding for other than LL ???

use bitstream_io::{BitRead, BitReader};
use log::debug;
use std::io::Cursor;
use thiserror::Error;

use crate::formats::bmff::ext_cr3::cmp1::Cmp1Box;

use self::{
  decoder::{predict_k_param_max, PREDICT_K_MAX},
  mdat::Tile,
};

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
  #[allow(dead_code)] // TODO: rounded_bits is used for 5/3
  rounded_bits: i32,
  cur_line: usize,
  line_buf: [Vec<i32>; 2],
  line_pos: usize,
  #[allow(dead_code)]
  line_len: usize,
  s_param: u32,
  k_param: u32,
  supports_partial: bool,
  /// Holds the decoding buffer for a single row
  //dec_buf: Vec<i32>,
  /// Bitstream from MDAT
  bitpump: BitPump<'mdat>,
}

impl<'mdat> BandParam<'mdat> {
  /// Get coefficent `a` from line buffer
  ///  c b d  (buf 0)
  ///  a x n  (buf 1)
  fn coeff_a(&self) -> i32 {
    self.line_buf[1][self.line_pos - 1]
  }

  /// Get coefficent `b` from line buffer
  ///  c b d  (buf 0)
  ///  a x n  (buf 1)
  fn coeff_b(&self) -> i32 {
    self.line_buf[0][self.line_pos]
  }

  /// Get coefficent `c` from line buffer
  ///  c b d  (buf 0)
  ///  a x n  (buf 1)
  fn coeff_c(&self) -> i32 {
    self.line_buf[0][self.line_pos - 1]
  }

  /// Get coefficent `d` from line buffer
  ///  c b d  (buf 0)
  ///  a x n  (buf 1)
  fn coeff_d(&self) -> i32 {
    self.line_buf[0][self.line_pos + 1]
  }

  fn decoded_buf(&self) -> &[i32] {
    &self.line_buf[1][1..1 + self.subband_width]
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

  /// Golomb-Rice decoding
  /// https://w3.ual.es/~vruiz/Docencia/Apuntes/Coding/Text/03-symbol_encoding/09-Golomb_coding/index.html
  /// escape and esc_bits are used to interrupt decoding when
  /// a value is not encoded using Golomb-Rice but directly encoded
  /// by esc_bits bits.
  fn rice_decode(&mut self, k: u32, escape: u32, esc_bits: u32) -> Result<u32> {
    // q, quotient = n//m, with m = 2^k (Rice coding)
    let prefix = self.bitstream_zeros()?;
    if prefix >= escape {
      // n
      Ok(self.bitstream_get_bits(esc_bits)?)
    } else if k > 0 {
      // Golomb-Rice coding : n = q * 2^k + r, with r is next k bits. r is n - (q*2^k)
      Ok((prefix << k) | self.bitstream_get_bits(k)?)
    } else {
      // q
      Ok(prefix)
    }
  }

  /// Adaptive Golomb-Rice decoding, by adapting k value
  /// Sometimes adapting is based on the next coefficent (n) instead
  /// of current (x) coefficent. So you can disable it with `adapt_k`
  /// and update k later.
  fn adaptive_rice_decode(&mut self, adapt_k: bool) -> Result<u32> {
    let val = self.rice_decode(self.k_param, 41, 21)?;
    if adapt_k {
      self.k_param = predict_k_param_max(self.k_param, val, PREDICT_K_MAX);
    }
    Ok(val)
  }
}

pub fn decompress_crx_image(buf: &[u8], cmp1: &Cmp1Box) -> Result<Vec<u16>> {
  let image = CodecParams::new(cmp1)?;
  debug!("{:?}", image);
  let result = image.decode(buf)?;
  Ok(result)
}
