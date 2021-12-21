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

use self::{mdat::Tile, rice::RiceDecoder};
use crate::formats::bmff::ext_cr3::cmp1::Cmp1Box;
use bitstream_io::BitReader;
use log::debug;
use std::io::Cursor;
use thiserror::Error;

mod decoder;
mod idwt;
mod iquant;
mod mdat;
mod rice;
mod runlength;

/// Each level has 6*8 = 0x30 = 48 ex coef values
/// Not every level is used. For example, an image with level=3
/// only used the last level 3 values.
#[cfg_attr(rustfmt, rustfmt_skip)]
const EX_COEF_NUM_TBL:[usize; 0x30*3] = [
    // Level 1
    1, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0,
    1, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0,
    // Level 2
    1, 1, 1, 1, 0, 0, 1, 0, 1, 0, 0, 0, 1, 2, 2, 1, 0, 0, 1, 1, 1, 1, 0, 0,
    1, 1, 1, 1, 0, 0, 1, 0, 1, 0, 0, 0, 1, 2, 2, 1, 0, 0, 1, 1, 1, 1, 0, 0,
    // Level 3
    1, 1, 1, 1, 1, 1, 1, 0, 1, 0, 1, 0, 1, 2, 2, 2, 2, 1, 1, 1, 1, 2, 2, 1,
    1, 1, 1, 2, 2, 1, 1, 0, 1, 1, 1, 1, 1, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1];

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
#[derive(Default, Debug, Clone, Copy)]
pub struct CodecParams {
  #[allow(dead_code)]
  sample_precision: u8,
  image_width: usize,
  image_height: usize,
  plane_count: u8,
  //plane_width: usize,
  //plane_height: usize,
  #[allow(dead_code)]
  subband_count: u8,
  levels: usize,
  n_bits: u8,
  enc_type: u8,
  tile_cols: usize,
  tile_rows: usize,
  tile_width: usize,
  tile_height: usize,
  mdat_hdr_size: u32,
  version: u16,
}

impl CodecParams {
  #[inline(always)]
  fn get_header<'a>(&self, mdat: &'a [u8]) -> &'a [u8] {
    &mdat[..self.mdat_hdr_size as usize]
  }

  /// The MDAT section contains the raw pixel data.
  /// Multiple images and data can be embedded into MDAT. The offsets
  /// and size is located in co64 and stsz BMF boxes. The raw data
  /// starts with an header block describing the data and subband offsets.
  ///
  /// MDAT Layout:
  /// |-----|-----------|-----|-----------|------------|--------------|-----|
  /// | HDR | RAW-BANDS | HDR | RAW-BANDS | JPEG Thumb | JPEG Preview | ... |
  /// |-----|-----------|-----|-----------|------------|--------------|-----|
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

    //let tile_cols: usize = (cmp1.f_width / cmp1.tile_width) as usize;
    //let tile_rows: usize = (cmp1.f_height / cmp1.tile_height) as usize;

    // Rounding for unbalanced sizes
    let tile_cols: usize = ((cmp1.f_width + cmp1.tile_width - 1) / cmp1.tile_width) as usize;
    let tile_rows: usize = ((cmp1.f_height + cmp1.tile_height - 1) / cmp1.tile_height) as usize;

    assert!(tile_cols > 0);
    assert!(tile_rows > 0);

    let params = Self {
      sample_precision: cmp1.n_bits as u8 + INCR_BIT_TABLE[4 * cmp1.enc_type as usize + 2] + 1,
      image_width: cmp1.f_width as usize,
      image_height: cmp1.f_height as usize,
      plane_count: cmp1.n_planes as u8,
      // 3 bands per level + one last LL
      // only 1 band for zero levels (uncompressed)
      subband_count: 3 * cmp1.image_levels as u8 + 1,
      levels: cmp1.image_levels as usize,
      n_bits: cmp1.n_bits as u8,
      enc_type: cmp1.enc_type as u8,
      tile_cols,
      tile_rows,
      tile_width: cmp1.tile_width as usize,
      tile_height: cmp1.tile_height as usize,
      mdat_hdr_size: cmp1.mdat_hdr_size,
      version: cmp1.version,
    };

    if params.tile_cols > 0xff {
      return Err(CrxError::General(format!("Tile column count {} is not supported", tile_cols)));
    }
    if params.tile_rows > 0xff {
      return Err(CrxError::General(format!("Tile row count {} is not supported", tile_rows)));
    }
    //if params.tile_width < 0x16 || params.tile_height < 0x16 || params.plane_width > 0x7FFF || params.plane_height > 0x7FFF {
    //  return Err(CrxError::General(format!("Invalid params for band decoding")));
    //}

    Ok(params)
  }

  /// Process tiles and update values
  pub(super) fn process_tiles(&mut self, tiles: &mut Vec<Tile>) {
    let tile_count = tiles.len();
    // Update each tile
    for cur_tile in tiles.iter_mut() {
      if (cur_tile.id + 1) % self.tile_cols != 0 {
        // not the last tile in a tile row
        cur_tile.tile_width = self.tile_width;
        cur_tile.plane_width = cur_tile.tile_width >> if self.plane_count == 4 { 1 } else { 0 };
        if self.tile_cols > 1 {
          cur_tile.tiles_right = true;
          if cur_tile.id % self.tile_cols != 0 {
            // not the first tile in tile row
            cur_tile.tiles_left = true;
          }
        }
      } else {
        // last tile in a tile row
        cur_tile.tile_width = self.image_width - self.tile_width * (self.tile_cols - 1);
        cur_tile.plane_width = cur_tile.tile_width >> if self.plane_count == 4 { 1 } else { 0 };
        if self.tile_cols > 1 {
          cur_tile.tiles_left = true;
        }
      }
      if (cur_tile.id) < (tile_count - self.tile_cols) {
        // in first tile row
        cur_tile.tile_height = self.tile_height;
        cur_tile.plane_height = cur_tile.tile_height >> if self.plane_count == 4 { 1 } else { 0 };
        if self.tile_rows > 1 {
          cur_tile.tiles_bottom = true;
          if cur_tile.id >= self.tile_cols {
            cur_tile.tiles_top = true;
          }
        }
      } else {
        // non first tile row
        cur_tile.tile_height = self.image_height - self.tile_height * (self.tile_rows - 1);
        cur_tile.plane_height = cur_tile.tile_height >> if self.plane_count == 4 { 1 } else { 0 };
        if self.tile_rows > 1 {
          cur_tile.tiles_top = true;
        }
      }
    }
    // process subbands
    for tile in tiles {
      debug!("{}", tile.descriptor_line());
      debug!("tile width: {}, tile height: {}", tile.tile_width, tile.tile_height);
      let mut plane_sizes = 0;
      self.process_subbands(tile);
      for plane in &mut tile.planes {
        debug!("{}", plane.descriptor_line());
        let mut band_sizes = 0;

        for band in &mut plane.subbands {
          debug!("{}", band.descriptor_line());
          assert!(band.subband_size != 0);
          assert_eq!(band.subband_size % 8, 0);
          band_sizes += band.subband_size;
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

  /// Process tiles and update values
  pub(super) fn process_subbands(&self, tile: &mut Tile) {
    for plane in &mut tile.planes {
      let mut band_w = tile.plane_width;
      let mut band_h = tile.plane_height;
      let mut band_width_ex_coef = 0;
      let mut band_height_ex_coef = 0;
      if self.levels > 0 {
        let row_ex_coef = &EX_COEF_NUM_TBL[0x30 * (self.levels - 1) + 6 * (tile.plane_width & 7)..];
        let col_ex_coef = &EX_COEF_NUM_TBL[0x30 * (self.levels - 1) + 6 * (tile.plane_height & 7)..];

        for lev in 0..self.levels {
          let w_odd_pixel = band_w & 1;
          let h_odd_pixel = band_h & 1;
          // With each level, width and hight are divided by 2
          band_w = (w_odd_pixel + band_w) >> 1;
          band_h = (h_odd_pixel + band_h) >> 1;

          let mut w_ex_coef0 = 0;
          let mut w_ex_coef1 = 0;
          let mut h_ex_coef0 = 0;
          let mut h_ex_coef1 = 0;
          let mut col_start = 0;
          let mut row_start = 0;
          if tile.tiles_right {
            w_ex_coef0 = row_ex_coef[2 * lev];
            w_ex_coef1 = row_ex_coef[2 * lev + 1];
          }
          if tile.tiles_left {
            w_ex_coef0 += 1;
            col_start = 1;
          }
          if tile.tiles_bottom {
            h_ex_coef0 = col_ex_coef[2 * lev];
            h_ex_coef1 = col_ex_coef[2 * lev + 1];
          }
          if tile.tiles_top {
            h_ex_coef0 += 1;
            row_start = 1;
          }

          // This sets the band width/height values.
          // Theoretically, it's just always plane_width/2.
          // But for multi-tile images, the band may contain
          // extra coefficents on any sides. Assumption is that
          // these extra coefficents are copied from the other tiles
          // over to improve compression or/and supress artefacts
          // on tile boundaries after tiles are assembled to full image.
          let i = (self.levels - lev) * 3;
          plane.subbands[i - 0].width = band_w + w_ex_coef0 - w_odd_pixel;
          plane.subbands[i - 0].height = band_h + h_ex_coef0 - h_odd_pixel;
          plane.subbands[i - 0].setup_idx(self.version, lev + 1, col_start, w_ex_coef0 - col_start, row_start, h_ex_coef0 - row_start);

          plane.subbands[i - 1].width = band_w + w_ex_coef1;
          plane.subbands[i - 1].height = band_h + h_ex_coef0 - h_odd_pixel;
          plane.subbands[i - 1].setup_idx(self.version, lev + 1, 0, w_ex_coef1, row_start, h_ex_coef0 - row_start);

          plane.subbands[i - 2].width = band_w + w_ex_coef0 - w_odd_pixel;
          plane.subbands[i - 2].height = band_h + h_ex_coef1;
          plane.subbands[i - 2].setup_idx(self.version, lev + 1, col_start, w_ex_coef0 - col_start, 0, h_ex_coef1);
        }
        band_width_ex_coef = 0;
        band_height_ex_coef = 0;
        if tile.tiles_right {
          band_width_ex_coef = row_ex_coef[2 * self.levels - 1];
        }
        if tile.tiles_bottom {
          band_height_ex_coef = col_ex_coef[2 * self.levels - 1];
        }
      }

      // LL3 band
      plane.subbands[0].width = band_width_ex_coef + band_w;
      plane.subbands[0].height = band_height_ex_coef + band_h;
      if self.levels > 0 {
        plane.subbands[0].setup_idx(self.version, self.levels, 0, band_width_ex_coef, 0, band_height_ex_coef);
      }
    }
  }
}

/// Parameter for a single Subband
struct BandParam<'mdat> {
  /// Width of the band in pixels
  subband_width: usize,
  /// Height of the band in pixels
  subband_height: usize,
  /// Mask for bit rounding (unused)
  rounded_bits_mask: i32,
  /// Bits for rounding (unused)
  #[allow(dead_code)]
  rounded_bits: i32,
  /// Current line, starting with 0
  cur_line: usize,
  /// Two lines to decode current line [1] and lookup into prev line [0]
  /// After each line iteration, the two items are just swapped.
  line_buf: [Vec<i32>; 2],
  /// Previous K values for Golomb-Rice adaptive decoding
  /// This buffer is only used for non-LL bands
  line_k: Vec<u32>,
  /// Current position
  line_pos: usize,
  /// Length of the current line (unused, but good to keep)
  #[allow(dead_code)]
  line_len: usize,
  /// Runlength control parameter
  s_param: u32,
  /// Q parameter for QP (see Subband for more details)
  /// The MDAT header contains a Q parameter which should be constant.
  /// But for some (unused) decoding routines, the Q param needs to be updated,
  /// so we need a mutable copy in the BandParam.
  pub q_param: u32,
  /// Unsure what partial means...
  supports_partial: bool,
  /// Rice decoder, provides bit access to the MDAT stream
  rice: RiceDecoder<'mdat>,
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

  /// Get decoded buffer
  fn decoded_buf(&self) -> &[i32] {
    // Skip first and last extra pixel
    &self.line_buf[1][1..1 + self.subband_width]
  }

  /// Get decoded buffer
  fn decoded_buf_mut(&mut self) -> &mut [i32] {
    // Skip first and last extra pixel
    &mut self.line_buf[1][1..1 + self.subband_width]
  }
}

/// Decompress a MDAT image buffer by given CMP1 box parameters
pub fn decompress_crx_image(buf: &[u8], cmp1: &Cmp1Box) -> Result<Vec<u16>> {
  let image = CodecParams::new(cmp1)?;
  debug!("CRX codec parameter: {:?}", image);
  image.decode(buf)
}
