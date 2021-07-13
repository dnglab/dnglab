// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use bitstream_io::BitReader;
use log::debug;
use rayon::prelude::*;
use std::{io::Cursor, time::Instant};

use super::{BandParam, CodecParams, CrxError, Plane, Result, Tile};

#[cfg_attr(rustfmt, rustfmt_skip)]
const _EX_COEF_NUM_TBL:[i32; 144] = [
    1, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0,
    1, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0,
    1, 1, 1, 1, 0, 0, 1, 0, 1, 0, 0, 0, 1, 2, 2, 1, 0, 0, 1, 1, 1, 1, 0, 0,
    1, 1, 1, 1, 0, 0, 1, 0, 1, 0, 0, 0, 1, 2, 2, 1, 0, 0, 1, 1, 1, 1, 0, 0,
    1, 1, 1, 1, 1, 1, 1, 0, 1, 0, 1, 0, 1, 2, 2, 2, 2, 1, 1, 1, 1, 2, 2, 1,
    1, 1, 1, 2, 2, 1, 1, 0, 1, 1, 1, 1, 1, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1];

#[cfg_attr(rustfmt, rustfmt_skip)]
const _Q_STEP_TBL:[i32; 8] = [0x28, 0x2D, 0x33, 0x39, 0x40, 0x48, 0x00, 0x00];

#[cfg_attr(rustfmt, rustfmt_skip)]
const JS:[u32; 32] = [1,     1,     1,     1,     2,      2,      2,      2,
                      4,     4,     4,     4,     8,      8,      8,      8,
                      0x10,  0x10,  0x20,  0x20,  0x40,   0x40,   0x80,   0x80,
                      0x100, 0x200, 0x400, 0x800, 0x1000, 0x2000, 0x4000, 0x8000];

#[cfg_attr(rustfmt, rustfmt_skip)]
const J:[u32; 32] = [0, 0, 0, 0, 1,    1,    1,    1,    2,    2,   2,
                     2, 3, 3, 3, 3,    4,    4,    5,    5,    6,   6,
                     7, 7, 8, 9, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F];

const PREDICT_K_MAX: u32 = 15;

impl CodecParams {
  /// Decode MDAT section into a single CFA image
  ///
  /// Decoding processes all planes in all tiles and assembles the
  /// decoded planes into proper tile output position and CFA pattern.
  pub fn decode(mut self, mdat: &[u8]) -> Result<Vec<u16>> {
    // Build nested Tiles/Planes/Bands
    let tiles = self.parse_header(mdat)?;

    // CRAW is unsupported
    if self.levels > 0 {
      return Err(CrxError::Unsupp("CRAW".into()));
    }

    // cfa output is of final resolution
    let mut cfa: Vec<u16> = vec![0; self.resolution()];

    // Iterator over all planes
    let plane_iter = tiles.par_iter().enumerate().flat_map(|(tile_id, tile)| {
      tile
        .planes
        .par_iter()
        .enumerate()
        .map(move |(plane_id, plane)| (tile_id, tile, plane_id, plane))
    });

    let bufs: Vec<(usize, usize, Result<Vec<u16>>)> = plane_iter
      .map(|(tile_id, tile, plane_id, plane)| (tile_id, plane_id, self.decode_plane(tile, plane, mdat)))
      .collect();

    // Integrate planes into final CFA
    let instant = Instant::now();
    for (tile_id, plane_id, buf) in bufs {
      let plane_buf = buf?;
      //dump_image_u16(&plane_buf, self.plane_width, self.plane_height, format!("/tmp/tile_{}_plane_{}.tiff", tile_id, plane_id));
      debug!("BUF: tile: {} plane: {}", tile_id, plane_id);
      assert_eq!(plane_buf.len(), (self.plane_height * self.plane_width) as usize);

      self.integrate_cfa(&mut cfa, tile_id, plane_id, &plane_buf)?;
    }
    debug!("CFA build: {} s", instant.elapsed().as_secs_f32());
    Ok(cfa)
  }

  /// Decode a single plane
  ///
  /// A plane is a monochrome image, a CFA image raw has
  /// normally 4 planes for R G1 G2 B or some other CFA pattern.
  pub fn decode_plane(&self, tile: &Tile, plane: &Plane, mdat: &[u8]) -> Result<Vec<u16>> {
    // Some checks for correct input
    assert!(self.plane_height > 0);
    assert!(self.plane_width > 0);

    // Reference to data section in MDAT
    // All calculated offsets are relative to the data section.
    let data = self.get_data(mdat);

    // Plane decoder returns a vector of exactly the size
    // of a plane (w*h).
    // We reserve only the correct capacity, values are pushed
    // into while decoding.
    let mut outbuf = Vec::with_capacity((self.plane_height * self.plane_width) as usize);

    let tile_mdat_offset =
      tile.data_offset + tile.qp_data.as_ref().map(|qp| qp.mdat_qp_data_size + qp.mdat_extra_size as u32).unwrap_or(0) as usize + plane.data_offset;

    let band = &plane.subbands.get(0).ok_or(CrxError::General("Subband #0 not found".into()))?;
    let band_mdat_offset = tile_mdat_offset + band.data_offset;
    debug!("band mdat offset: {}", band_mdat_offset);
    let band_buf = &data[band_mdat_offset..];

    // Line length is subband + one additional pixel at start end end
    let line_len = 1 + plane.subbands[0].width + 1;

    let bitpump = BitReader::endian(Cursor::new(band_buf), bitstream_io::BigEndian);

    let mut param = BandParam {
      subband_width: plane.subbands[0].width,
      subband_height: plane.subbands[0].height,
      rounded_bits_mask: if plane.support_partial { plane.rounded_bits_mask } else { 0 },
      rounded_bits: 0,
      cur_line: 0,
      line_buf: vec![0; line_len * 2], // fill for two buffered lines
      line_len,
      line0_pos: 0,
      line1_pos: 0,
      line2_pos: 0,
      s_param: 0,
      k_param: 0,
      supports_partial: if plane.support_partial { true } else { false }, // TODO: only for subbandnum == 0
      bitpump,
      dec_buf: vec![0; plane.subbands[0].width],
    };



    //debug!("Param: {:?}", param);

    for _ in 0..band.height {
      self.decode_line(&mut param)?;
      assert_eq!(param.dec_buf.len(), param.subband_width as usize);
      self.convert_plane_line(param.dec_buf.as_slice(), &mut outbuf);
    }

    assert_eq!(outbuf.len(), (self.plane_height * self.plane_width) as usize);

    Ok(outbuf)
  }

  /// Predict K parameter without a maximum constraint
  #[inline(always)]
  fn _predict_k_param(prev_k: u32, bit_code: u32) -> u32 {
    Self::predict_k_param_max(prev_k, bit_code, 0)
  }

  /// Predict K parameter with maximum constraint
  #[inline(always)]
  fn predict_k_param_max(prev_k: u32, bit_code: u32, max_val: u32) -> u32 {
    // K is is range 0..=15
    assert!(prev_k <= PREDICT_K_MAX);
    assert!(max_val <= PREDICT_K_MAX);

    // Calculate new K
    let new_k = if max_val == 0 {
      1 // no prediction
    } else {
      let p: u32 = 2_u32.pow(prev_k);
      let bp: u32 = bit_code >> prev_k;
      let new_k_param = prev_k
        + if bp > 2 {
          if bp > 5 {
            2
          } else {
            1
          }
        } else {
          0
        };
      if bit_code < (p / 2) {
        assert_ne!(new_k_param, 0);
        std::cmp::min(new_k_param - 1, max_val) // p >> 1
      } else {
        std::cmp::min(new_k_param, max_val)
      }
    };
    //debug!("Predict K: {} for prev: {}, bitcode: {}, max: {}", new_k, prev_k, bit_code, max_val);
    new_k
  }

  /// Decode a single L1 symbol
  #[allow(non_snake_case)]
  fn decode_symbol_L1(&self, param: &mut BandParam, do_median_pred: bool, not_eol: bool) -> Result<()> {
    if do_median_pred {
      let delta: i32 = *param.get_line0(1) - *param.get_line0(0);
      let lookup = ((((*param.get_line0(0) < *param.get_line1(0)) as usize) ^ ((delta < 0) as usize)) << 1)
        + (((*param.get_line1(0) < *param.get_line0(1)) as usize) ^ ((delta < 0) as usize));

      *param.get_line1(1) = match lookup {
        0 | 1 => delta + *param.get_line1(0),
        2 => *param.get_line1(0),
        3 => *param.get_line0(1),
        _ => return Err(CrxError::General(format!("Crx decoder error while decode symbol L1"))),
      };
    } else {
      *param.get_line1(1) = *param.get_line0(1);
    }

    // get next error symbol
    let mut bit_code = param.next_error_symbol()?;

    // add converted (+/-) error code to predicted value
    *param.get_line1(1) += error_code_signed(bit_code);

    // for not end of the line - use one symbol ahead to estimate next K
    if not_eol {
      let next_delta: i32 = (*param.get_line0(2) - *param.get_line0(1)) << 1;
      bit_code = (bit_code + next_delta.abs() as u32) >> 1;
      param.advance_buf0();
    }

    // update K parameter
    param.k_param = Self::predict_k_param_max(param.k_param, bit_code, PREDICT_K_MAX);
    param.advance_buf1();

    Ok(())
  }

  /// Get symbol count for run-length decoding
  fn symbol_count_runlength(&self, param: &mut BandParam, length: u32) -> Result<u32> {
    let mut n_syms: u32 = 1;
    while param.bitstream_get_bits(1)? == 1 {
      n_syms += JS[param.s_param as usize];
      if n_syms > length {
        n_syms = length;
        break;
      }
      if param.s_param < 31 {
        param.s_param += 1;
      }
      if n_syms == length {
        break;
      }
    } // End while
    if n_syms < length {
      if J[param.s_param as usize] != 0 {
        n_syms += param.bitstream_get_bits(J[param.s_param as usize])?;
      }
      if param.s_param > 0 {
        param.s_param -= 1;
      }
      if n_syms > length {
        return Err(CrxError::General(format!("Crx decoder error while decoding line")));
      }
    }
    Ok(n_syms)
  }

  /// Decode top line
  fn decode_top_line(&self, param: &mut BandParam) -> Result<()> {
    *param.get_line1(0) = 0;

    let mut length = param.subband_width as u32;

    while length > 1 {
      if *param.get_line1(0) != 0 {
        // Re-use value
        *param.get_line1(1) = *param.get_line1(0);
      } else {
        if param.bitstream_get_bits(1)? == 1 {
          let n_syms = self.symbol_count_runlength(param, length)?;
          length = length.saturating_sub(n_syms);
          // copy symbol n_syms times
          for _ in 0..n_syms {
            *param.get_line1(1) = *param.get_line1(0);
            param.advance_buf1();
          }
          if length <= 0 {
            break;
          }
        } // if bitstream == 1

        *param.get_line1(1) = 0;
      }

      let bit_code = param.next_error_symbol()?;

      //debug!("k_param: {}, bit_code: {}", param.k_param, bit_code);
      *param.get_line1(1) += error_code_signed(bit_code);
      param.k_param = Self::predict_k_param_max(param.k_param, bit_code, PREDICT_K_MAX);
      param.advance_buf1();

      length = length.saturating_sub(1);
    }

    if length == 1 {
      // Copy previous and add error correction
      *param.get_line1(1) = *param.get_line1(0);
      let bit_code = param.next_error_symbol()?;
      *param.get_line1(1) += error_code_signed(bit_code);
      param.advance_buf1();
      // Predict new K
      param.k_param = Self::predict_k_param_max(param.k_param, bit_code, PREDICT_K_MAX);
    }

    *param.get_line1(1) = *param.get_line1(0) + 1;
    Ok(())
  }

  /// Decode a line which is not a top line
  fn decode_nontop_line(&self, param: &mut BandParam) -> Result<()> {
    let mut length = param.subband_width as u32;

    // copy down from line0 to line1
    *param.get_line1(0) = *param.get_line0(1);

    while length > 1 {
      if *param.get_line1(0) != *param.get_line0(1) || *param.get_line1(0) != *param.get_line0(2) {
        self.decode_symbol_L1(param, true, true)?;
      } else {
        if param.bitstream_get_bits(1)? == 1 {
          let n_syms = self.symbol_count_runlength(param, length)?;
          length = length.saturating_sub(n_syms);
          // copy symbol n_syms times
          for _ in 0..n_syms {
            *param.get_line1(1) = *param.get_line1(0);
            param.advance_buf1();
          }
          // Forward line0 position as line1 position is forwarded, too
          param.line0_pos += n_syms as usize;
        } // if bitstream == 1

        if length > 0 {
          self.decode_symbol_L1(param, false, length > 1)?;
        }
      }

      length = length.saturating_sub(1);
    } // end while

    if length == 1 {
      self.decode_symbol_L1(param, true, false)?;
    }
    *param.get_line1(1) = *param.get_line1(0) + 1;
    Ok(())
  }

  /// Decode a single line from input band
  fn decode_line(&self, param: &mut BandParam) -> Result<()> {
    assert!(param.cur_line < param.subband_height);
    if param.cur_line == 0 {
      param.s_param = 0;
      param.k_param = 0;
      if param.supports_partial {
        if param.rounded_bits_mask <= 0 {
          param.line0_pos = 0;
          param.line1_pos = param.line0_pos + param.line_len;
          let line_pos = param.line1_pos + 1;
          self.decode_top_line(param)?;
          //band_buf.extend_from_slice(&param.line_buf[line_pos..line_pos + param.subband_width]);
          param.dec_buf.copy_from_slice(&param.line_buf[line_pos..line_pos + param.subband_width]);
          param.cur_line += 1;
        } else {
          unimplemented!()
        }
      } else {
        unimplemented!()
      }
    } else if !param.supports_partial {
      unimplemented!()
    } else if param.rounded_bits_mask <= 0 {
      if param.cur_line & 1 == 1 {
        param.line1_pos = 0;
        param.line0_pos = param.line1_pos + param.line_len;
      } else {
        param.line0_pos = 0;
        param.line1_pos = param.line0_pos + param.line_len;
      }
      let line_pos = param.line1_pos + 1;
      self.decode_nontop_line(param)?;
      //band_buf.extend_from_slice(&param.line_buf[line_pos..line_pos + param.subband_width]);
      param.dec_buf.copy_from_slice(&param.line_buf[line_pos..line_pos + param.subband_width]);
      param.cur_line += 1;
    } else {
      unimplemented!()
    }
    Ok(())
  }

  /// Convert a decoded line to plane output
  fn convert_plane_line(&self, line: &[i32], plane_buf: &mut Vec<u16>) {
    assert_eq!(self.enc_type, 0);
    assert_eq!(self.plane_count, 4);
    let median: i32 = 1 << (self.n_bits - 1);
    let max_val: i32 = (1 << self.n_bits) - 1;
    line.iter().for_each(|v| plane_buf.push(constrain(median + v, 0, max_val) as u16));
  }

  /// Integrate a plane buffer into CFA output image
  ///
  /// A plane is a single monochrome image for one of the four CFA colors.
  /// `plane_id` is 0, 1, 2 or 3 for R, G1, G2, B
  fn integrate_cfa(&self, cfa_buf: &mut [u16], tile_id: usize, plane_id: usize, plane_buf: &[u16]) -> Result<()> {
    // 2x2 pixel for RGGB
    const CFA_DIM: usize = 2;

    assert_ne!(plane_buf.len(), 0);
    assert_ne!(cfa_buf.len(), 0);
    assert!(self.tile_cols > 0);
    assert!(self.tile_rows > 0);

    if plane_id > 3 {
      return Err(CrxError::Overflow(format!(
        "More then 4 planes detected, unable to process plane_id {}",
        plane_id
      )));
    }

    let tile_row_idx = tile_id / self.tile_cols; // round down
    let tile_col_idx = tile_id % self.tile_cols; // round down

    // Offset from top
    let row_offset = tile_row_idx * self.tile_width;

    // Offset from left
    let col_offset = tile_col_idx * self.tile_width;

    let (row_shift, col_shift) = match plane_id {
      0 => (0, 0),
      1 => (0, 1),
      2 => (1, 0),
      3 => (1, 1),
      _ => {
        return Err(CrxError::General(format!("Invalid plane id")));
      }
    };

    for plane_row in 0..self.plane_height {
      // Row index into CFA for untiled full area
      let row_idx = row_offset + (plane_row * CFA_DIM) + row_shift;

      for plane_col in 0..self.plane_width {
        // Row index into CFA for untiled full area
        let col_idx = col_offset + (plane_col * CFA_DIM) + col_shift;

        // Copy from plane to CFA
        cfa_buf[(row_idx * self.image_width) + col_idx] = plane_buf[plane_row * self.plane_width + plane_col];
      }
    }

    Ok(())
  }
}

/// Constrain a given value into min/max
#[inline(always)]
fn constrain(value: i32, min: i32, max: i32) -> i32 {
  let res = if value < min {
    min
  } else if value > max {
    max
  } else {
    value
  };
  assert!(res <= u16::MAX as i32);
  res
}

/// The error code contains a sign bit at bit 0.
/// This routine converts an unsigned bit_code to the correct
/// signed integer value.
/// For this, the sign bit is inverted and XOR with
/// the shifted integer value.
fn error_code_signed(bit_code: u32) -> i32 {
  -((bit_code & 1) as i32) ^ (bit_code >> 1) as i32
}
