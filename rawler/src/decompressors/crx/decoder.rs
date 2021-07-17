// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

// Original Crx decoder crx.cpp was written by Alexey Danilchenko for libraw.
// Rewritten in Rust by Daniel Vogelbacher, based on logic found in
// crx.cpp and documentation done by Laurent Clévy (https://github.com/lclevy/canon_cr3).

use bitstream_io::BitReader;
use log::debug;
use rayon::prelude::*;
use std::{io::Cursor, time::Instant};

use crate::decompressors::crx::mdat::parse_header;

use super::{
  mdat::{Plane, Tile},
  BandParam, CodecParams, CrxError, Result,
};

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

/// See ITU T.78 Section A.2.1 Step 3
/// Initialise the variables for the run mode: RUNindex=0 and J[0..31]
#[cfg_attr(rustfmt, rustfmt_skip)]
const J: [u32; 32] = [0, 0,  0,  0,  1,  1,  1,  1,
                      2, 2,  2,  2,  3,  3,  3,  3,
                      4, 4,  5,  5,  6,  6,  7,  7,
                      8, 9, 10, 11, 12, 13, 14, 15];

/// Precalculated values for (1 << J[0..31])
#[cfg_attr(rustfmt, rustfmt_skip)]
const JSHIFT: [u32; 32] = [1 << J[0],  1 << J[1],  1 << J[2],  1 << J[3],
                           1 << J[4],  1 << J[5],  1 << J[6],  1 << J[7],
                           1 << J[8],  1 << J[9],  1 << J[10], 1 << J[11],
                           1 << J[12], 1 << J[13], 1 << J[14], 1 << J[15],
                           1 << J[16], 1 << J[17], 1 << J[18], 1 << J[19],
                           1 << J[20], 1 << J[21], 1 << J[22], 1 << J[23],
                           1 << J[24], 1 << J[25], 1 << J[26], 1 << J[27],
                           1 << J[28], 1 << J[29], 1 << J[30], 1 << J[31]];

/// Maximum value for K during Adaptive Golomb-Rice for K prediction
pub(super) const PREDICT_K_MAX: u32 = 15;

impl CodecParams {
  /// Decode MDAT section into a single CFA image
  ///
  /// Decoding processes all planes in all tiles and assembles the
  /// decoded planes into proper tile output position and CFA pattern.
  pub fn decode(mut self, mdat: &[u8]) -> Result<Vec<u16>> {
    // Build nested Tiles/Planes/Bands
    let mut tiles = parse_header(self.get_header(mdat))?;
    self.process_tiles(&mut tiles);

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
    let mut plane_buf = vec![0; (self.plane_height * self.plane_width) as usize];

    let tile_mdat_offset =
      tile.data_offset + tile.qp_data.as_ref().map(|qp| qp.mdat_qp_data_size + qp.mdat_extra_size as u32).unwrap_or(0) as usize + plane.data_offset;

    let band = &plane.subbands.get(0).ok_or(CrxError::General("Subband #0 not found".into()))?;
    let band_mdat_offset = tile_mdat_offset + band.data_offset;
    debug!("band mdat offset: {}", band_mdat_offset);
    let band_buf = &data[band_mdat_offset..];

    // Line length is subband + one additional pixel at start end end
    let line_len = 1 + plane.subbands[0].width + 1;

    let bitpump = BitReader::endian(Cursor::new(band_buf), bitstream_io::BigEndian);

    let line_buf = [vec![0; line_len], vec![0; line_len]];

    let mut param = BandParam {
      subband_width: plane.subbands[0].width,
      subband_height: plane.subbands[0].height,
      rounded_bits_mask: if plane.support_partial { plane.rounded_bits_mask } else { 0 },
      rounded_bits: 0,
      cur_line: 0,
      line_buf,
      line_pos: 0,
      line_len,
      s_param: 0,
      k_param: 0,
      supports_partial: if plane.support_partial { true } else { false }, // TODO: only for subbandnum == 0
      bitpump,
      //dec_buf: vec![0; plane.subbands[0].width],
    };

    //debug!("Param: {:?}", param);

    //println!("band height: {}", band.height);
    for i in 0..band.height {
      self.decode_line(&mut param)?;
      assert_eq!(param.decoded_buf().len(), param.subband_width as usize);
      self.convert_plane_line(param.decoded_buf(), &mut plane_buf[(i * band.width)..]);
    }

    assert_eq!(plane_buf.len(), (self.plane_height * self.plane_width) as usize);

    Ok(plane_buf)
  }

  /// Get symbol run count for run-length decoding
  /// See T.87 Section A.7.1.2 Run-length coding
  fn symbol_run_count(&self, param: &mut BandParam, remaining: u32) -> Result<u32> {
    assert!(remaining > 1);
    let mut run_cnt: u32 = 1;
    // See T.87 A.7.1.2 Code segment A.15
    // Bitstream 111110... means 5 lookups into J to decode final RUNcnt
    while run_cnt != remaining && param.bitstream_get_bits(1)? == 1 {
      // JS is precalculated (1 << J[RUNindex])
      run_cnt += JSHIFT[param.s_param as usize];
      if run_cnt > remaining {
        run_cnt = remaining;
        break;
      }
      param.s_param = std::cmp::min(param.s_param + 1, 31);
    }
    // See T.87 A.7.1.2 Code segment A.16
    if run_cnt < remaining {
      if J[param.s_param as usize] > 0 {
        run_cnt += param.bitstream_get_bits(J[param.s_param as usize])?;
      }
      param.s_param = param.s_param.saturating_sub(1); // prevent underflow
      if run_cnt > remaining {
        return Err(CrxError::General(format!("Crx decoder error while decoding line")));
      }
    }
    Ok(run_cnt)
  }

  /// Decode top line
  /// For the first line (top) in a plane, no MED is used because
  /// there is no previous line for coeffs b, c and d.
  /// So this decoding is a simplified version from decode_nontop_line().
  fn decode_top_line(&self, p: &mut BandParam) -> Result<()> {
    assert_eq!(p.line_pos, 1);
    let mut remaining = p.subband_width as u32;
    // Init coeff a (real image pixel starts at 1)
    p.line_buf[1][p.line_pos - 1] = 0; // is is [0] because at start line_pos is 1
    while remaining > 1 {
      // Loop over full width of line (backwards)
      if p.coeff_a() != 0 {
        p.line_buf[1][p.line_pos] = p.coeff_a();
      } else {
        if p.bitstream_get_bits(1)? == 1 {
          let n_syms = self.symbol_run_count(p, remaining)?;
          remaining = remaining.saturating_sub(n_syms);
          // copy symbol n_syms times
          for _ in 0..n_syms {
            p.line_buf[1][p.line_pos] = p.coeff_a();
            p.line_pos += 1;
          }
          if remaining == 0 {
            break;
          }
        } // if bitstream == 1
        p.line_buf[1][p.line_pos] = 0;
      }
      let bit_code = p.adaptive_rice_decode(true)?;
      p.line_buf[1][p.line_pos] += error_code_signed(bit_code);
      p.line_pos += 1;
      remaining = remaining.saturating_sub(1);
    }
    // Remaining pixel?
    if remaining == 1 {
      let x = p.coeff_a(); // no MED, just use coeff a
      let bit_code = p.adaptive_rice_decode(true)?;
      p.line_buf[1][p.line_pos] = x + error_code_signed(bit_code);
      p.line_pos += 1;
    }
    assert!(p.line_pos < p.line_buf[1].len());
    p.line_buf[1][p.line_pos] = p.coeff_a() + 1;
    Ok(())
  }

  /// Decode a line which is not a top line
  /// This used run length coding, Median Edge Detection (MED) and
  /// adaptive Golomb-Rice entropy encoding.
  /// Golomb-Rice becomes more efficient when using an adaptive K value
  /// instead of a fixed one.
  /// The K parameter is used as q = n >> k where n is the sample to encode.
  fn decode_nontop_line(&self, p: &mut BandParam) -> Result<()> {
    assert_eq!(p.line_pos, 1);
    let mut remaining = p.subband_width as u32;
    // Init coeff a: a = b
    p.line_buf[1][p.line_pos - 1] = p.coeff_b();
    // Loop over full width of line (backwards)
    while remaining > 1 {
      let mut x = 0;
      //  c b d
      //  a x n
      // Median Edge Detection to predict pixel x. Described in patent US2016/0323602 and T.87
      if p.coeff_a() == p.coeff_b() && p.coeff_a() == p.coeff_d() {
        // different than step [0104], where Condition: "a=c and c=b and b=d", c not used
        if p.bitstream_get_bits(1)? == 1 {
          let n_syms = self.symbol_run_count(p, remaining)?;
          remaining = remaining.saturating_sub(n_syms);
          // copy symbol n_syms times
          for _ in 0..n_syms {
            p.line_buf[1][p.line_pos] = p.coeff_a();
            p.line_pos += 1;
          }
        } // if bitstream == 1
        if remaining > 0 {
          x = p.coeff_b(); // use new coeff b because we moved line_pos!
        }
      } else {
        // no run length coding, use MED instead
        x = med(p.coeff_a(), p.coeff_b(), p.coeff_c());
      }
      if remaining > 0 {
        let mut bit_code = p.adaptive_rice_decode(false)?;
        // add converted (+/-) error code to predicted value
        p.line_buf[1][p.line_pos] = x + error_code_signed(bit_code);
        // for not end of the line - use one symbol ahead to estimate next K
        if remaining > 1 {
          let delta: i32 = (p.coeff_d() - p.coeff_b()) << 1;
          bit_code = (bit_code + delta.abs() as u32) >> 1;
        }
        p.k_param = predict_k_param_max(p.k_param, bit_code, PREDICT_K_MAX);
        p.line_pos += 1;
      }
      remaining = remaining.saturating_sub(1);
    } // end while length > 1
      // Remaining pixel?
    if remaining == 1 {
      let x = med(p.coeff_a(), p.coeff_b(), p.coeff_c());
      let bit_code = p.adaptive_rice_decode(true)?;
      // add converted (+/-) error code to predicted value
      p.line_buf[1][p.line_pos] = x + error_code_signed(bit_code);
      p.line_pos += 1;
    }
    assert!(p.line_pos < p.line_buf[1].len());
    p.line_buf[1][p.line_pos] = p.coeff_a() + 1;
    Ok(())
  }

  /// Decode a single line from input band
  /// For decoding, two line buffers are required (except for the first line).
  /// After each decoding line, the two buffers are swapped, so the previous one
  /// is always in line_buf[0] (containing coefficents c, b, d) and the current
  /// line is in line_buf[1] (containing coefficents a, x, n).
  ///
  /// The line buffers has an extra sample on both ends. So the buffer layout is:
  ///
  /// |E|Samples........................|E|
  /// |c|bd                           cb|d|
  /// |a|xn                           ax|n|
  ///  ^ ^                               ^
  ///  | |                               |-- Extra sample to provide fake d coefficent
  ///  | |---- First sample value
  ///  |------ Extra sample to provide a fake a/c coefficent
  ///
  /// After line is decoded, the E samples are ignored when
  /// copied into the final plane buffer.
  ///
  /// For non-LL bands, decoding process differs a little bit
  /// because some value rounding is added. This process is not
  /// implemented yet.
  fn decode_line(&self, param: &mut BandParam) -> Result<()> {
    assert!(param.cur_line < param.subband_height);
    if param.cur_line == 0 {
      param.s_param = 0;
      param.k_param = 0;
      if param.supports_partial {
        if param.rounded_bits_mask <= 0 {
          // We start at first real pixel value
          param.line_pos = 1;
          self.decode_top_line(param)?;
          param.cur_line += 1;
        } else {
          // Used for wavelet transformed data, not supported
          unimplemented!()
        }
      } else {
        // Used for wavelet transformed data, not supported
        unimplemented!()
      }
    } else if !param.supports_partial {
      // Used for wavelet transformed data, not supported
      unimplemented!()
    } else if param.rounded_bits_mask <= 0 {
      param.line_pos = 1;
      // Swap line buffers so previous decoded (1) is now above (0)
      param.line_buf.swap(0, 1);
      self.decode_nontop_line(param)?;
      param.cur_line += 1;
    } else {
      // Used for wavelet transformed data, not supported
      unimplemented!()
    }
    Ok(())
  }

  /// Convert a decoded line to plane output
  /// Results from decode_line() are signed 32 bit integers.
  /// By using a median and max value, these are converted
  /// to unsigned 16 bit integers.
  fn convert_plane_line(&self, line: &[i32], plane_buf: &mut [u16]) {
    assert_eq!(self.enc_type, 0);
    assert_eq!(self.plane_count, 4);
    let median: i32 = 1 << (self.n_bits - 1);
    let max_val: i32 = (1 << self.n_bits) - 1;
    for (i, v) in line.iter().enumerate() {
      plane_buf[i] = constrain(median + v, 0, max_val) as u16
    }
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
fn constrain(value: i32, min: i32, max: i32) -> i32 {
  std::cmp::min(std::cmp::max(value, min), max)
  /*
  let res = if value < min {
    min
  } else if value > max {
    max
  } else {
    value
  };
  assert!(res <= u16::MAX as i32);
  res
   */
}

/// The error code contains a sign bit at bit 0.
/// Example: 10010 1 -> negative value, 10010 0 -> positive value
/// This routine converts an unsigned bit_code to the correct
/// signed integer value.
/// For this, the sign bit is inverted and XOR with
/// the shifted integer value.
fn error_code_signed(bit_code: u32) -> i32 {
  -((bit_code & 1) as i32) ^ (bit_code >> 1) as i32
}

/// Predict K parameter without a maximum constraint
pub(super) fn _predict_k_param(prev_k: u32, bit_code: u32) -> u32 {
  predict_k_param_max(prev_k, bit_code, 0)
}

/// Predict K parameter with maximum constraint
/// Golomb-Rice becomes more efficient when used with an adaptive
/// K parameter. This is done my predicting the next K value for the
/// next sample value.
pub(super) fn predict_k_param_max(prev_k: u32, value: u32, max_val: u32) -> u32 {
  // K is is range 0..=15
  assert!(prev_k <= PREDICT_K_MAX);
  assert!(max_val <= PREDICT_K_MAX);
  let mut new_k = prev_k;
  if value >> prev_k > 2 {
    new_k += 1;
  }
  if value >> prev_k > 5 {
    new_k += 1;
  }
  if value < ((1 << prev_k) >> 1) {
    new_k -= 1;
  }
  std::cmp::min(new_k, max_val)
}

/// Median Edge Detection
/// [0053] Obtains a predictive value p of the coefficient by using
/// MED prediction, thereby performing predictive coding.
pub(super) fn med(a: i32, b: i32, c: i32) -> i32 {
  if c >= std::cmp::max(a, b) {
    std::cmp::min(a, b)
  } else if c <= std::cmp::min(a, b) {
    std::cmp::max(a, b)
  } else {
    a + b - c // no edge detected
  }
}
