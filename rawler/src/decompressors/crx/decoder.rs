// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

// Original Crx decoder crx.cpp was written by Alexey Danilchenko for libraw.
// Rewritten in Rust by Daniel Vogelbacher, based on logic found in
// crx.cpp and documentation done by Laurent Clévy (https://github.com/lclevy/canon_cr3).

use bitstream_io::BitReader;
use log::debug;
use rayon::prelude::*;
use std::{io::Cursor, time::Instant};

use crate::decompressors::crx::{idwt::WaveletTransform, mdat::parse_header, rice::RiceDecoder};

use super::{
  mdat::{Plane, Tile},
  BandParam, CodecParams, CrxError, Result,
};

/// Maximum value for K during Adaptive Golomb-Rice for K prediction
pub(super) const PREDICT_K_MAX: u32 = 15;
pub(super) const PREDICT_K_ESCAPE: u32 = 41;
pub(super) const PREDICT_K_ESCBITS: u32 = 21;

impl CodecParams {
  /// Decode MDAT section into a single CFA image
  ///
  /// Decoding processes all planes in all tiles and assembles the
  /// decoded planes into proper tile output position and CFA pattern.
  pub fn decode(mut self, mdat: &[u8]) -> Result<Vec<u16>> {
    // Build nested Tiles/Planes/Bands
    let mut tiles = parse_header(self.get_header(mdat))?;
    self.process_tiles(&mut tiles);
    for tile in tiles.iter_mut() {
      tile.generate_qstep_table(&self, self.get_data(mdat))?;
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
      let plane_buf = buf.unwrap();
      //debug!("BUF: tile: {} plane: {}", tile_id, plane_id);
      assert_eq!(plane_buf.len(), (tiles[tile_id].plane_height * tiles[tile_id].plane_width) as usize);
      self.integrate_cfa(&tiles, &mut cfa, tile_id, plane_id, &plane_buf)?;
    }
    debug!("CFA build: {} s", instant.elapsed().as_secs_f32());
    Ok(cfa)
  }

  /// Decode a single plane
  ///
  /// A plane is a monochrome image, a CFA image raw has
  /// normally 4 planes for R G1 G2 B or some other CFA pattern.
  pub fn decode_plane(&self, tile: &Tile, plane: &Plane, mdat: &[u8]) -> Result<Vec<u16>> {
    if self.levels > 0 {
      self.decode_plane_lossy(tile, plane, mdat)
    } else {
      self.decode_plane_lossess(tile, plane, mdat)
    }
  }

  /// Decode a single plane
  ///
  /// A plane is a monochrome image, a CFA image raw has
  /// normally 4 planes for R G1 G2 B or some other CFA pattern.
  pub fn decode_plane_lossess(&self, tile: &Tile, plane: &Plane, mdat: &[u8]) -> Result<Vec<u16>> {
    // Some checks for correct input
    assert!(tile.plane_height > 0);
    assert!(tile.plane_width > 0);

    // Reference to data section in MDAT
    // All calculated offsets are relative to the data section.
    let data = self.get_data(mdat);

    // Plane decoder returns a vector of exactly the size
    // of a plane (w*h).
    let mut plane_buf = vec![0; (tile.plane_height * tile.plane_width) as usize];

    let plane_mdat_offset =
      tile.data_offset + tile.qp_data.as_ref().map(|qp| qp.mdat_qp_data_size + qp.mdat_extra_size as u32).unwrap_or(0) as usize + plane.data_offset;

    let i = 0;

    let band = &plane.subbands.get(i).ok_or(CrxError::General(format!("Subband {} not found", i)))?;
    let band_mdat_offset = plane_mdat_offset + band.data_offset;
    //println!("band mdat offset: {}", band_mdat_offset);
    let band_buf = &data[band_mdat_offset..band_mdat_offset + band.data_size];

    // Line length is subband + one additional pixel at start end end
    let line_len = 1 + band.width + 1;

    let bitpump = BitReader::endian(Cursor::new(band_buf), bitstream_io::BigEndian);

    let line_buf = [vec![0; line_len], vec![0; line_len]];
    let line_k = Vec::new(); // lossless decoding needs no K buffer

    let mut param = BandParam {
      subband_width: band.width,
      subband_height: band.height,
      rounded_bits_mask: if plane.support_partial && i == 0 { plane.rounded_bits_mask } else { 0 },
      rounded_bits: 0,
      cur_line: 0,
      line_buf,
      line_k,
      line_pos: 0,
      line_len,
      s_param: 0,
      q_param: band.q_param,
      supports_partial: if plane.support_partial && i == 0 { true } else { false }, // TODO: only for subbandnum == 0
      rice: RiceDecoder::new(bitpump),
    };

    //debug!("Param: {:?}", param);

    //println!("band height: {}", band.height);
    for i in 0..band.height {
      self.decode_line(&mut param)?;
      assert_eq!(param.decoded_buf().len(), param.subband_width as usize);
      self.convert_plane_line(param.decoded_buf(), &mut plane_buf[(i * band.width)..]);
    }

    assert_eq!(plane_buf.len(), (tile.plane_height * tile.plane_width) as usize);

    Ok(plane_buf)
  }

  /// Decode a single plane
  ///
  /// A plane is a monochrome image, a CFA image raw has
  /// normally 4 planes for R G1 G2 B or some other CFA pattern.
  pub fn decode_plane_lossy(&self, tile: &Tile, plane: &Plane, mdat: &[u8]) -> Result<Vec<u16>> {
    // Some checks for correct input
    assert!(tile.plane_height > 0);
    assert!(tile.plane_width > 0);

    // Reference to data section in MDAT
    // All calculated offsets are relative to the data section.
    let data = self.get_data(mdat);

    // Plane decoder returns a vector of exactly the size
    // of a plane (w*h).
    let mut plane_buf = vec![0; (tile.plane_height * tile.plane_width) as usize];

    let plane_mdat_offset =
      tile.data_offset + tile.qp_data.as_ref().map(|qp| qp.mdat_qp_data_size + qp.mdat_extra_size as u32).unwrap_or(0) as usize + plane.data_offset;

    let mut params = Vec::with_capacity(plane.subbands.len());
    for (band_id, band) in plane.subbands.iter().enumerate() {
      let band_mdat_offset = plane_mdat_offset + band.data_offset;
      //println!("band mdat offset: {}", band_mdat_offset);
      let band_buf = &data[band_mdat_offset..band_mdat_offset + band.data_size];

      // Line length is subband + one additional pixel at start end end
      let line_len = 1 + band.width + 1;

      let bitpump = BitReader::endian(Cursor::new(band_buf), bitstream_io::BigEndian);

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
        supports_partial: if plane.support_partial && band_id == 0 { true } else { false }, // TODO: only for subbandnum == 0
        rice: RiceDecoder::new(bitpump),
      };
      params.push(param);
    }

    let mut iwt_transforms = Vec::with_capacity(self.levels);
    for level in 0..self.levels {
      let band = 3 * level + 1;
      let (height, width) = if level >= self.levels - 1 {
        (tile.plane_height, tile.plane_width)
      } else {
        (plane.subbands[band + 3].height, plane.subbands[band + 4].width)
      };
      //println!("band width: {}", width);
      iwt_transforms.push(WaveletTransform::new(height, width));
    }

    self.idwt_53_filter_init(tile, plane, &mut params, &mut iwt_transforms, self.levels)?;

    for i in 0..tile.plane_height {
      self.idwt_53_filter_decode(tile, plane, &mut params, &mut iwt_transforms, self.levels - 1)?;
      self.idwt_53_filter_transform(tile, plane, &mut params, &mut iwt_transforms, self.levels - 1)?;

      let line_data = iwt_transforms[self.levels - 1].getline();

      assert_eq!(line_data.len(), tile.plane_width);
      self.convert_plane_line(line_data, &mut plane_buf[(i * tile.plane_width)..]);
    }

    assert_eq!(plane_buf.len(), (tile.plane_height * tile.plane_width) as usize);

    Ok(plane_buf)
  }

  /// Decode top line without a previous K buffer
  fn decode_top_line_no_ref_prev_line(&self, p: &mut BandParam) -> Result<()> {
    assert_eq!(p.line_pos, 1);
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
          let n_syms = self.symbol_run_count(p, remaining).unwrap();
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
    assert!(p.line_pos < p.line_buf[1].len());
    p.line_buf[1][p.line_pos] = 0;
    Ok(())
  }

  /// Decode nontop line with a previous K buffer
  fn decode_nontop_line_no_ref_prev_line(&self, p: &mut BandParam) -> Result<()> {
    //println!("Decode nontop {}", p.cur_line);
    assert_eq!(p.line_pos, 1);
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
          assert!(remaining != 1);
          let n_syms = self.symbol_run_count(p, remaining).unwrap();

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
    assert!(p.line_pos < p.line_buf[1].len());
    Ok(())
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
          bit_code = (bit_code + delta.abs() as u32) >> 1;
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
    assert!(p.line_pos < p.line_buf[1].len());
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
      p.rice.update_k_param((bit_code + 2 * code.abs() as u32) >> 1, PREDICT_K_MAX);
    } else {
      p.rice.update_k_param(bit_code, PREDICT_K_MAX);
    }

    p.line_pos += 1;
    Ok(())
  }

  /// Decode a rounded line which is not a top line
  fn decode_top_line_rounded(&self, p: &mut BandParam) -> Result<()> {
    assert_eq!(p.line_pos, 1);
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
  fn decode_nontop_line_rounded(&self, p: &mut BandParam) -> Result<()> {
    assert_eq!(p.line_pos, 1);
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
  pub(super) fn decode_line(&self, param: &mut BandParam) -> Result<()> {
    assert!(param.cur_line < param.subband_height);
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
      self.decode_nontop_line_no_ref_prev_line(param).unwrap();
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
  fn integrate_cfa(&self, tiles: &Vec<Tile>, cfa_buf: &mut [u16], tile_id: usize, plane_id: usize, plane_buf: &[u16]) -> Result<()> {
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

    for plane_row in 0..tiles[tile_id].plane_height {
      // Row index into CFA for untiled full area
      let row_idx = row_offset + (plane_row * CFA_DIM) + row_shift;

      for plane_col in 0..tiles[tile_id].plane_width {
        // Row index into CFA for untiled full area
        let col_idx = col_offset + (plane_col * CFA_DIM) + col_shift;

        // Copy from plane to CFA
        cfa_buf[(row_idx * self.image_width) + col_idx] = plane_buf[plane_row * tiles[tile_id].plane_width + plane_col];
      }
    }

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
