// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

// Original Crx decoder crx.cpp was written by Alexey Danilchenko for libraw.
// Rewritten in Rust by Daniel Vogelbacher, based on logic found in
// crx.cpp and documentation done by Laurent Clévy (https://github.com/lclevy/canon_cr3).

use super::{
  BandParam, BitPump, CodecParams, CrxError, Result,
  mdat::{Plane, Tile},
};
use crate::decompressors::crx::{idwt::WaveletTransform, mdat::parse_header, rice::RiceDecoder};
use itertools::izip;
use log::debug;
use rayon::prelude::*;
use std::{collections::VecDeque, time::Instant};

/// Maximum value for K during Adaptive Golomb-Rice for K prediction
pub(super) const PREDICT_K_MAX: u32 = 15;
pub(super) const PREDICT_K_ESCAPE: u32 = 41;
pub(super) const PREDICT_K_ESCBITS: u32 = 21;

/// Plane rows to decode ahead per subband. All bands are refilled at the same
/// time, so they run in parallel. Decoded lines use memory until the transform
/// has consumed them, so this is kept small compared to the plane height.
const LOOKAHEAD_ROWS: usize = 64;
const LOOKAHEAD_EXTRA_LINES: usize = 2;

struct PlaneLineIter<'a> {
  tile: &'a Tile,
  plane: &'a Plane,
  codec: CodecParams,
  params: Vec<BandParam<'a>>,
  iwt_transforms: Vec<WaveletTransform>,
  //plane_buf: Vec<i32>,
  next_row: usize,
}

impl<'a> PlaneLineIter<'a> {
  /// Create a new PlaneLine iterator for decoding
  fn new(codec: CodecParams, tile: &'a Tile, plane: &'a Plane, mdat: &'a [u8]) -> Result<Self> {
    // Some checks for correct input
    assert!(tile.plane_height > 0);
    assert!(tile.plane_width > 0);

    // Reference to data section in MDAT
    // All calculated offsets are relative to the data section.
    let data = codec.get_data(mdat);

    let plane_mdat_offset =
      tile.data_offset + tile.qp_data.as_ref().map(|qp| qp.mdat_qp_data_size + qp.mdat_extra_size as u32).unwrap_or(0) as usize + plane.data_offset;

    let mut params = Vec::with_capacity(plane.subbands.len());
    for (band_id, band) in plane.subbands.iter().enumerate() {
      let band_mdat_offset = plane_mdat_offset + band.data_offset;
      debug!("Band {} has MDAT offset: {}", band_id, band_mdat_offset);
      let band_buf = &data[band_mdat_offset..band_mdat_offset + band.data_size];
      // Line length is subband + one additional pixel at start and end
      let line_len = 1 + band.width + 1;
      let bitpump = BitPump::new(band_buf);

      let param = BandParam {
        subband_width: band.width,
        subband_height: band.height,
        rounded_bits_mask: if plane.support_partial && band_id == 0 { plane.rounded_bits_mask } else { 0 },
        rounded_bits: 0,
        cur_line: 0,
        line_buf: [vec![0; line_len], vec![0; line_len]],
        line_k: vec![0; line_len],
        line_pos: 0,
        line_len,
        s_param: 0,
        q_param: band.q_param,
        supports_partial: plane.support_partial && band_id == 0,
        rice: RiceDecoder::new(bitpump),
        decoded_ahead: VecDeque::new(),
      };
      params.push(param);
    }

    let mut iwt_transforms = Vec::with_capacity(codec.levels);

    if codec.levels > 0 {
      // create Wavelet transforms
      for level in 0..codec.levels {
        let band = 3 * level + 1;
        let (height, width) = if level >= codec.levels - 1 {
          (tile.plane_height, tile.plane_width)
        } else {
          (plane.subbands[band + 3].height, plane.subbands[band + 4].width)
        };
        iwt_transforms.push(WaveletTransform::new(height, width));
      }
      decode_ahead(&codec, tile, plane, &mut params, 0);
      codec.idwt_53_filter_init(tile, plane, &mut params, &mut iwt_transforms, codec.levels)?;
    }

    Ok(Self {
      params,
      tile,
      plane,
      codec,
      iwt_transforms,
      next_row: 0,
    })
  }

  /// Decode a single line from plane
  fn decode_plane_line(&mut self) -> Result<&[i32]> {
    if self.next_row < self.tile.plane_height {
      let row = self.next_row;
      self.next_row += 1;
      if self.codec.levels > 0 {
        if row.is_multiple_of(LOOKAHEAD_ROWS) {
          decode_ahead(&self.codec, self.tile, self.plane, &mut self.params, row);
        }
        self
          .codec
          .idwt_53_filter_decode(self.tile, self.plane, &mut self.params, &mut self.iwt_transforms, self.codec.levels - 1)?;
        self
          .codec
          .idwt_53_filter_transform(self.tile, self.plane, &mut self.params, &mut self.iwt_transforms, self.codec.levels - 1)?;
        let line_data = self.iwt_transforms[self.codec.levels - 1].getline();
        debug_assert_eq!(line_data.len(), self.tile.plane_width);
        Ok(line_data)
      } else {
        debug_assert_eq!(self.plane.subbands.len(), 1);
        let param = &mut self.params[0];
        self.codec.decode_line(param)?;
        let line_data = param.decoded_buf();
        debug_assert_eq!(line_data.len(), param.subband_width as usize);
        debug_assert_eq!(line_data.len(), self.tile.plane_width);
        Ok(line_data)
      }
    } else {
      Err(CrxError::General("All rows processed, can't decode more".to_string()))
    }
  }
}

/// Iterator over a plane, returning on each call a new decoded line
impl<'a> Iterator for PlaneLineIter<'a> {
  type Item = Result<PlaneLine>;

  fn next(&mut self) -> Option<Self::Item> {
    if self.next_row < self.tile.plane_height {
      match self.decode_plane_line() {
        Ok(line) => Some(Ok(line.into())),
        Err(e) => Some(Err(e)),
      }
    } else {
      None
    }
  }

  fn size_hint(&self) -> (usize, Option<usize>) {
    (0, Some(self.tile.plane_height))
  }
}

/// Decode the subband lines needed for the next [`LOOKAHEAD_ROWS`] plane rows
///
/// Each subband has its own bitstream, so its lines can be decoded in parallel
/// and independently of the inverse wavelet transform, which then takes them
/// from [`BandParam::decoded_ahead`]. Decoding stops at the first failed line,
/// as the error is returned when the transform reaches this line.
fn decode_ahead(codec: &CodecParams, tile: &Tile, plane: &Plane, params: &mut [BandParam], row: usize) {
  let end_row = (row + LOOKAHEAD_ROWS).min(tile.plane_height);
  params
    .par_iter_mut()
    .zip(plane.subbands.par_iter())
    .enumerate()
    .for_each(|(band_id, (param, band))| {
      if band.data_size == 0 || param.decoded_ahead.back().is_some_and(Result::is_err) {
        return;
      }
      let level = band_id.saturating_sub(1) / 3;
      let q_step = tile.q_step.as_ref().map(|f| &f[level]);
      // Bands have fewer lines than the plane has rows. The wavelet filter reads
      // a few lines ahead, without them each block would end with a serial decode.
      let end_line = ((end_row * band.height).div_ceil(tile.plane_height) + LOOKAHEAD_EXTRA_LINES).min(band.height);
      while param.cur_line < end_line {
        let line = codec.decode_next_line(band, param, q_step);
        let failed = line.is_err();
        param.decoded_ahead.push_back(line);
        if failed {
          break;
        }
      }
    });
}

/// A plane line is a vector if i32 values
type PlaneLine = Vec<i32>;

/// Wrapper for PlaneLineIter
/// Decodes a complete plane and returns it as vector of lines
fn decode_full_plane(codec: &CodecParams, tile: &Tile, plane: &Plane, mdat: &[u8]) -> Result<Vec<PlaneLine>> {
  //eprintln!("Process tile {}, plane: {}", tile.id, plane.id);
  let line_decoder = PlaneLineIter::new(*codec, tile, plane, mdat)?;
  line_decoder.collect()
}

impl CodecParams {
  /// Decode MDAT section into a single CFA image
  ///
  /// Decoding processes all planes in all tiles and assembles the
  /// decoded planes into proper tile output position and CFA pattern.
  pub fn decode(mut self, mdat: &[u8]) -> Result<Vec<u16>> {
    let instant = Instant::now();
    debug!("Tile configuration: rows: {}, columns: {}", self.tile_rows, self.tile_cols);
    // Build nested Tiles/Planes/Bands
    let mut tiles = parse_header(self.get_header(mdat))?;
    self.process_tiles(&mut tiles);
    for tile in tiles.iter_mut() {
      tile.generate_qstep_table(&self, self.get_data(mdat))?;
    }

    // cfa output is of final resolution
    let mut cfa: Vec<u16> = vec![0; self.resolution()];

    // Combine all tiles and planes into parallel iterators
    // and decode the full planes.
    let plane_bufs: Result<Vec<Vec<Vec<PlaneLine>>>> = tiles
      .par_iter()
      .map(|tile| tile.planes.par_iter().map(move |plane| decode_full_plane(&self, tile, plane, mdat)).collect())
      .collect();

    // Now we have a list of tiles->planes->plane-lines
    // and can combine them to the final CFA
    let bufs = plane_bufs?;
    for (tile_id, (tile, planes)) in tiles.iter().zip(&bufs).enumerate() {
      if planes.len() != 4 {
        return Err(CrxError::General(format!(
          "Invalid plane count {} (expected 4) for tile {}",
          planes.len(),
          tile_id
        )));
      }
      // Tile area in the CFA, must be inside the image
      let (row_offset, col_offset) = self.tile_offset(tile_id);
      if row_offset + planes[0].len() * 2 > self.image_height || col_offset + tile.plane_width * 2 > self.image_width {
        return Err(CrxError::Overflow(format!("Tile {} exceeds the image area", tile_id)));
      }
    }

    // Each CFA row is built from one line of two planes of each tile
    // in this row, so the rows can be processed in parallel.
    cfa.par_chunks_exact_mut(self.image_width).enumerate().try_for_each(|(row, cfa_row)| {
      for (tile_id, (tile, planes)) in tiles.iter().zip(&bufs).enumerate() {
        let (row_offset, col_offset) = self.tile_offset(tile_id);
        let Some(tile_row) = row.checked_sub(row_offset) else {
          continue;
        };
        let plane_row = tile_row / 2;
        if plane_row >= planes[0].len() {
          continue;
        }
        let lines: [&[i32]; 4] = std::array::from_fn(|plane| planes[plane][plane_row].as_slice());
        if lines.iter().any(|line| line.len() != tile.plane_width) {
          return Err(CrxError::General(format!("Invalid plane line width for tile {}", tile_id)));
        }
        self.convert_cfa_line(lines, tile_row % 2, &mut cfa_row[col_offset..col_offset + tile.plane_width * 2])?;
      }
      Ok(())
    })?;
    debug!("MDAT decoding and CFA build: {} s", instant.elapsed().as_secs_f32());
    Ok(cfa)
  }

  /// Decode top line without a previous K buffer
  fn decode_top_line_no_ref_prev_line(&self, p: &mut BandParam) -> Result<()> {
    debug_assert_eq!(p.line_pos, 1);
    let mut remaining = p.subband_width as u32;
    // Init coef a and c (real image pixel starts at 1)
    p.line_buf[0][p.line_pos - 1] = 0; // is [0] because at start line_pos is 1
    p.line_buf[1][p.line_pos - 1] = 0; // is [0] because at start line_pos is 1
    while remaining > 1 {
      //println!("remaining: {}", remaining);
      // Loop over full width of line (backwards)
      if p.coeff_a() != 0 {
        //println!("coeff {} is != 0", p.coeff_a());
        let bit_code = p.rice.adaptive_rice_decode(true, PREDICT_K_ESCAPE, PREDICT_K_ESCBITS, PREDICT_K_MAX)?;
        p.line_buf[1][p.line_pos] = error_code_signed(bit_code);
      } else {
        //println!("coeff {} = 0", p.coeff_a());
        if p.rice.bitstream_get_bits(1)? == 1 {
          let n_syms = self.symbol_run_count(p, remaining)?;
          //println!("found {} syms", n_syms);
          remaining = remaining.saturating_sub(n_syms);
          // copy symbol n_syms times
          for _ in 0..n_syms {
            // For the first line, run-length coding uses only the symbol
            // value 0, so we can fill the line buffer and K buffer with 0.
            p.line_buf[1][p.line_pos] = 0;
            p.line_k[p.line_pos - 1] = 0;
            p.line_pos += 1;
          }

          if remaining == 0 {
            break;
          }
        } // if bitstream == 1

        let bit_code = p.rice.adaptive_rice_decode(true, PREDICT_K_ESCAPE, PREDICT_K_ESCBITS, PREDICT_K_MAX)?;
        p.line_buf[1][p.line_pos] = error_code_signed(bit_code + 1); // Caution: + 1
        //println!("code: {}", p.line_buf[1][p.line_pos]);
      }
      p.line_k[p.line_pos - 1] = p.rice.k();
      p.line_pos += 1;
      remaining = remaining.saturating_sub(1);
    }
    // Remaining pixel?
    if remaining == 1 {
      let bit_code = p.rice.adaptive_rice_decode(true, PREDICT_K_ESCAPE, PREDICT_K_ESCBITS, PREDICT_K_MAX)?;
      p.line_buf[1][p.line_pos] = error_code_signed(bit_code);
      p.line_k[p.line_pos - 1] = p.rice.k();
      p.line_pos += 1;
    }
    debug_assert!(p.line_pos < p.line_buf[1].len());
    p.line_buf[1][p.line_pos] = 0;
    Ok(())
  }

  /// Decode nontop line with a previous K buffer
  fn decode_nontop_line_no_ref_prev_line(&self, p: &mut BandParam) -> Result<()> {
    //println!("Decode nontop {}", p.cur_line);
    debug_assert_eq!(p.line_pos, 1);
    let mut remaining = p.subband_width as u32;
    while remaining > 1 {
      // Loop over full width of line (backwards)
      if (p.coeff_d() | p.coeff_b() | p.coeff_a()) != 0 {
        let bit_code = p.rice.adaptive_rice_decode(true, PREDICT_K_ESCAPE, PREDICT_K_ESCBITS, 0)?;
        p.line_buf[1][p.line_pos] = error_code_signed(bit_code);
        if p.line_k[p.line_pos].saturating_sub(p.rice.k()) <= 1 {
          if p.rice.k() >= 15 {
            p.rice.set_k(15);
          }
        } else {
          p.rice.set_k(p.rice.k() + 1);
        }
      } else {
        if p.rice.bitstream_get_bits(1)? == 1 {
          debug_assert!(remaining != 1);
          let n_syms = self.symbol_run_count(p, remaining)?;

          remaining = remaining.saturating_sub(n_syms);
          // copy symbol n_syms times
          for _ in 0..n_syms {
            // For the first line, run-length coding uses only the symbol
            // value 0, so we can fill the line buffer and K buffer with 0.
            p.line_buf[1][p.line_pos] = 0;
            p.line_k[p.line_pos - 1] = 0;
            p.line_pos += 1;
          }
        } // if bitstream == 1

        if remaining <= 1 {
          if remaining == 1 {
            let bit_code = p.rice.adaptive_rice_decode(true, PREDICT_K_ESCAPE, PREDICT_K_ESCBITS, PREDICT_K_MAX)?;
            p.line_buf[1][p.line_pos] = error_code_signed(bit_code + 1);
            p.line_k[p.line_pos - 1] = p.rice.k();
            p.line_pos += 1;
            remaining = remaining.saturating_sub(1); // skip remaining check at end of function
          }
          break;
        } else {
          let bit_code = p.rice.adaptive_rice_decode(true, PREDICT_K_ESCAPE, PREDICT_K_ESCBITS, 0)?;
          p.line_buf[1][p.line_pos] = error_code_signed(bit_code + 1); // Caution: + 1
          if p.line_k[p.line_pos].saturating_sub(p.rice.k()) <= 1 {
            if p.rice.k() >= 15 {
              p.rice.set_k(15);
            }
          } else {
            p.rice.set_k(p.rice.k() + 1);
          }
        }
      }
      p.line_k[p.line_pos - 1] = p.rice.k();
      p.line_pos += 1;
      remaining = remaining.saturating_sub(1);
    }
    // Remaining pixel?
    if remaining == 1 {
      let bit_code = p.rice.adaptive_rice_decode(true, PREDICT_K_ESCAPE, PREDICT_K_ESCBITS, PREDICT_K_MAX)?;
      p.line_buf[1][p.line_pos] = error_code_signed(bit_code);
      p.line_k[p.line_pos - 1] = p.rice.k();
      p.line_pos += 1;
    }
    debug_assert!(p.line_pos < p.line_buf[1].len());
    Ok(())
  }

  /// Decode top line
  /// For the first line (top) in a plane, no MED is used because
  /// there is no previous line for coeffs b, c and d.
  /// So this decoding is a simplified version from decode_nontop_line().
  fn decode_top_line(&self, p: &mut BandParam) -> Result<()> {
    debug_assert_eq!(p.line_pos, 1);
    let mut remaining = p.subband_width as u32;
    // Init coeff a (real image pixel starts at 1)
    p.line_buf[1][p.line_pos - 1] = 0; // is is [0] because at start line_pos is 1
    while remaining > 1 {
      // Loop over full width of line (backwards)
      if p.coeff_a() != 0 {
        p.line_buf[1][p.line_pos] = p.coeff_a();
      } else {
        if p.rice.bitstream_get_bits(1)? == 1 {
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
      let bit_code = p.rice.adaptive_rice_decode(true, PREDICT_K_ESCAPE, PREDICT_K_ESCBITS, PREDICT_K_MAX)?;
      p.line_buf[1][p.line_pos] += error_code_signed(bit_code);
      p.line_pos += 1;
      remaining = remaining.saturating_sub(1);
    }
    // Remaining pixel?
    if remaining == 1 {
      let x = p.coeff_a(); // no MED, just use coeff a
      let bit_code = p.rice.adaptive_rice_decode(true, PREDICT_K_ESCAPE, PREDICT_K_ESCBITS, PREDICT_K_MAX)?;
      p.line_buf[1][p.line_pos] = x + error_code_signed(bit_code);
      p.line_pos += 1;
    }
    debug_assert!(p.line_pos < p.line_buf[1].len());
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
    debug_assert_eq!(p.line_pos, 1);
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
        if p.rice.bitstream_get_bits(1)? == 1 {
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
        let mut bit_code = p.rice.adaptive_rice_decode(false, PREDICT_K_ESCAPE, PREDICT_K_ESCBITS, PREDICT_K_MAX)?;
        // add converted (+/-) error code to predicted value
        p.line_buf[1][p.line_pos] = x + error_code_signed(bit_code);
        // for not end of the line - use one symbol ahead to estimate next K
        if remaining > 1 {
          let delta: i32 = (p.coeff_d() - p.coeff_b()) << 1;
          bit_code = (bit_code + delta.unsigned_abs()) >> 1;
        }
        p.rice.update_k_param(bit_code, PREDICT_K_MAX);
        p.line_pos += 1;
      }
      remaining = remaining.saturating_sub(1);
    } // end while length > 1
    // Remaining pixel?
    if remaining == 1 {
      let x = med(p.coeff_a(), p.coeff_b(), p.coeff_c());
      let bit_code = p.rice.adaptive_rice_decode(true, PREDICT_K_ESCAPE, PREDICT_K_ESCBITS, PREDICT_K_MAX)?;
      // add converted (+/-) error code to predicted value
      p.line_buf[1][p.line_pos] = x + error_code_signed(bit_code);
      p.line_pos += 1;
    }
    debug_assert!(p.line_pos < p.line_buf[1].len());
    p.line_buf[1][p.line_pos] = p.coeff_a() + 1;
    Ok(())
  }

  /// Decode a symbol x in rounded mode.
  /// Used only when levels==0 (lossless mode)
  fn decode_symbol_rounded(&self, p: &mut BandParam, use_med: bool, not_eol: bool) -> Result<()> {
    let sym = if use_med { med(p.coeff_a(), p.coeff_b(), p.coeff_c()) } else { p.coeff_b() };
    let bit_code = p.rice.adaptive_rice_decode(false, PREDICT_K_ESCAPE, PREDICT_K_ESCBITS, PREDICT_K_MAX)?;
    let mut code = error_code_signed(bit_code);
    let x = p.rounded_bits_mask * 2 * code + (code >> 31);
    p.line_buf[1][p.line_pos] = x + sym;

    if not_eol {
      if p.coeff_d() > p.coeff_b() {
        code = (p.coeff_d() - p.coeff_b() + p.rounded_bits_mask - 1) >> p.rounded_bits;
      } else {
        code = -((p.coeff_b() - p.coeff_d() + p.rounded_bits_mask) >> p.rounded_bits);
      }
      p.rice.update_k_param((bit_code + 2 * code.unsigned_abs()) >> 1, PREDICT_K_MAX);
    } else {
      p.rice.update_k_param(bit_code, PREDICT_K_MAX);
    }

    p.line_pos += 1;
    Ok(())
  }

  /// Decode a rounded line which is not a top line
  fn decode_top_line_rounded(&self, p: &mut BandParam) -> Result<()> {
    debug_assert_eq!(p.line_pos, 1);
    let mut remaining = p.subband_width as u32;
    // Init coeff a (real image pixel starts at 1)
    p.line_buf[1][p.line_pos - 1] = 0; // is is [0] because at start line_pos is 1
    while remaining > 1 {
      // Loop over full width of line (backwards)
      if p.coeff_a().abs() > p.rounded_bits_mask {
        p.line_buf[1][p.line_pos] = p.coeff_a();
      } else {
        if p.rice.bitstream_get_bits(1)? == 1 {
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
      let bit_code = p.rice.adaptive_rice_decode(true, PREDICT_K_ESCAPE, PREDICT_K_ESCBITS, PREDICT_K_MAX)?;
      let code = error_code_signed(bit_code);
      p.line_buf[1][p.line_pos] += p.rounded_bits_mask * 2 * code + (code >> 31);
      p.line_pos += 1;
      remaining = remaining.saturating_sub(1);
    }
    // Remaining pixel?
    if remaining == 1 {
      let bit_code = p.rice.adaptive_rice_decode(true, PREDICT_K_ESCAPE, PREDICT_K_ESCBITS, PREDICT_K_MAX)?;
      let code = error_code_signed(bit_code);
      p.line_buf[1][p.line_pos] += p.rounded_bits_mask * 2 * code + (code >> 31);
      p.line_pos += 1;
    }
    debug_assert!(p.line_pos < p.line_buf[1].len());
    p.line_buf[1][p.line_pos] = p.coeff_a() + 1;
    Ok(())
  }

  /// Decode a line which is not a top line
  /// This used run length coding, Median Edge Detection (MED) and
  /// adaptive Golomb-Rice entropy encoding.
  /// Golomb-Rice becomes more efficient when using an adaptive K value
  /// instead of a fixed one.
  /// The K parameter is used as q = n >> k where n is the sample to encode.
  #[allow(clippy::comparison_chain)]
  fn decode_nontop_line_rounded(&self, p: &mut BandParam) -> Result<()> {
    debug_assert_eq!(p.line_pos, 1);
    let mut remaining = p.subband_width as u32;
    let mut value_reached = false;
    p.line_buf[0][p.line_pos - 1] = p.coeff_b();
    p.line_buf[1][p.line_pos - 1] = p.coeff_b();
    // Loop over full width of line (backwards)
    while remaining > 1 {
      if (p.coeff_d() - p.coeff_b()).abs() > p.rounded_bits_mask {
        self.decode_symbol_rounded(p, true, true)?;
        value_reached = true;
      } else if value_reached || (p.coeff_c() - p.coeff_a()).abs() > p.rounded_bits_mask {
        self.decode_symbol_rounded(p, true, true)?;
        value_reached = false;
      } else {
        if p.rice.bitstream_get_bits(1)? == 1 {
          let n_syms = self.symbol_run_count(p, remaining)?;
          remaining = remaining.saturating_sub(n_syms);
          // copy symbol n_syms times
          for _ in 0..n_syms {
            p.line_buf[1][p.line_pos] = p.coeff_a();
            p.line_pos += 1;
          }
        } // if bitstream == 1
        if remaining > 1 {
          self.decode_symbol_rounded(p, false, true)?;
          value_reached = (p.coeff_b() - p.coeff_c()).abs() > p.rounded_bits_mask;
        } else if remaining == 1 {
          self.decode_symbol_rounded(p, false, false)?;
        }
      }
      remaining = remaining.saturating_sub(1);
    } // end while length > 1
    // Remaining pixel?
    if remaining == 1 {
      self.decode_symbol_rounded(p, true, false)?;
    }
    debug_assert!(p.line_pos < p.line_buf[1].len());
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
  /// because some value rounding is added.
  pub(super) fn decode_line(&self, param: &mut BandParam) -> Result<()> {
    debug_assert!(param.cur_line < param.subband_height);
    // We start at first real pixel value
    param.line_pos = 1;
    if param.cur_line == 0 {
      param.s_param = 0;
      param.rice.set_k(0); // TODO: required?
      if param.supports_partial {
        if param.rounded_bits_mask <= 0 {
          self.decode_top_line(param)?;
        } else {
          param.rounded_bits = 1;
          if (param.rounded_bits_mask & !1) != 0 {
            while param.rounded_bits_mask >> param.rounded_bits != 0 {
              param.rounded_bits += 1;
            }
          }
          self.decode_top_line_rounded(param)?;
        }
      } else {
        self.decode_top_line_no_ref_prev_line(param)?;
      }
    } else if !param.supports_partial {
      // Swap line buffers so previous decoded (1) is now above (0)
      param.line_buf.swap(0, 1);
      self.decode_nontop_line_no_ref_prev_line(param)?;
    } else if param.rounded_bits_mask <= 0 {
      // Swap line buffers so previous decoded (1) is now above (0)
      param.line_buf.swap(0, 1);
      self.decode_nontop_line(param)?;
    } else {
      // Swap line buffers so previous decoded (1) is now above (0)
      param.line_buf.swap(0, 1);
      self.decode_nontop_line_rounded(param)?;
    }
    param.cur_line += 1;
    Ok(())
  }
}

/// Constrain a given value into min/max
pub(super) fn constrain(value: i32, min: i32, max: i32) -> i32 {
  std::cmp::min(std::cmp::max(value, min), max)
  /*
  let res = if value < min {
    min
  } else if value > max {
    max
  } else {
    value
  };
  debug_assert!(res <= u16::MAX as i32);
  res
   */
}

/// The error code contains a sign bit at bit 0.
/// Example: 10010 1 -> negative value, 10010 0 -> positive value
/// This routine converts an unsigned bit_code to the correct
/// signed integer value.
/// For this, the sign bit is inverted and XOR with
/// the shifted integer value.
pub(super) fn error_code_signed(bit_code: u32) -> i32 {
  -((bit_code & 1) as i32) ^ (bit_code >> 1) as i32
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

impl CodecParams {
  /// Get the (row, column) offset of a tile in the CFA image
  fn tile_offset(&self, tile_id: usize) -> (usize, usize) {
    let tile_row_idx = tile_id / self.tile_cols; // round down
    let tile_col_idx = tile_id % self.tile_cols; // round down
    (tile_row_idx * self.tile_width, tile_col_idx * self.tile_width)
  }

  /// Convert one decoded line of all four planes of a tile and write
  /// the CFA row `row_shift` (0 or 1) of this line into `cfa_row`.
  ///
  /// A plane is a single monochrome image for one of the four CFA colors
  /// R, G1, G2 and B (plane 0 to 3). Plane 0 and 1 form the first row of the
  /// 2x2 CFA pattern, plane 2 and 3 the second row.
  ///
  /// Results from decode_line() are signed 32 bit integers.
  /// By using a median and max value, these are converted
  /// to unsigned 16 bit integers.
  fn convert_cfa_line(&self, lines: [&[i32]; 4], row_shift: usize, cfa_row: &mut [u16]) -> Result<()> {
    let [l0, l1, l2, l3] = lines;
    let pixels = izip!(cfa_row.as_chunks_mut::<2>().0, l0, l1, l2, l3);

    match self.enc_type {
      0 => {
        let median: i32 = 1 << (self.median_bits - 1);
        let max_val: i32 = (1 << self.median_bits) - 1;

        for (cfa, v0, v1, v2, v3) in pixels {
          let (left, right) = if row_shift == 0 { (v0, v1) } else { (v2, v3) };
          cfa[0] = constrain(median + left, 0, max_val) as u16;
          cfa[1] = constrain(median + right, 0, max_val) as u16;
        }
      }
      3 => {
        let median: i32 = 1 << (self.median_bits - 1) << 10;
        let max_val: i32 = (1 << self.median_bits) - 1;

        for (cfa, v0, v1, v2, v3) in pixels {
          let mut gr: i32 = median + (v0 << 10) - 168 * v1 - 585 * v3;
          if gr < 0 {
            gr = -(((gr.abs() + 512) >> 9) & !1);
          } else {
            gr = ((gr.abs() + 512) >> 9) & !1;
          }
          if row_shift == 0 {
            cfa[0] = constrain((median + (v0 << 10) + 1510 * v3 + 512) >> 10, 0, max_val) as u16;
            cfa[1] = constrain((v2 + gr + 1) >> 1, 0, max_val) as u16;
          } else {
            cfa[0] = constrain((gr - v2 + 1) >> 1, 0, max_val) as u16;
            cfa[1] = constrain((median + (v0 << 10) + 1927 * v1 + 512) >> 10, 0, max_val) as u16;
          }
        }
      }
      enc_type => {
        return Err(CrxError::General(format!("Unsupported encoding type {}", enc_type)));
      }
    }
    Ok(())
  }
}
