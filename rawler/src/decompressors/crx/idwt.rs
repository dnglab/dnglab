// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

// Original Crx decoder crx.cpp was written by Alexey Danilchenko for libraw.
// Rewritten in Rust by Daniel Vogelbacher, based on logic found in
// crx.cpp and documentation done by Laurent Clévy (https://github.com/lclevy/canon_cr3).

use super::{
  BandParam, CodecParams, Result,
  mdat::{Plane, Tile},
};

/// Even sample of the inverse 5/3 lifting: the low-pass value minus the
/// rounded mean of the two neighboring high-pass values,
/// `floor((h0 + h1 + 2) / 4)`
#[inline(always)]
fn even_sample(low: i32, h0: i32, h1: i32) -> i32 {
  low - ((h0 + h1 + 2) >> 2)
}

/// [`even_sample`] at a border without a neighboring tile, where the
/// missing high-pass neighbor is mirrored (`h0 == h1 == h`)
#[inline(always)]
fn even_sample_edge(low: i32, h: i32) -> i32 {
  low - ((h + 1) >> 1)
}

/// Odd sample of the inverse 5/3 lifting: the high-pass value plus the
/// mean of the two neighboring even samples, `floor((e0 + e1) / 2)`
#[inline(always)]
fn odd_sample(high: i32, e0: i32, e1: i32) -> i32 {
  high + ((e0 + e1) >> 1)
}

/// [`odd_sample`] at a border without a neighboring tile, where the
/// missing even neighbor is mirrored (`e0 == e1 == e`)
#[inline(always)]
fn odd_sample_edge(high: i32, e: i32) -> i32 {
  high + e
}

/// Horizontal inverse 5/3 transform of one line from the coefficients of
/// the low-pass band `low` and the high-pass band `high` into `out`. At a
/// border without a neighboring tile, the missing neighbor is mirrored.
fn idwt_53_line(tile: &Tile, out: &mut [i32], low: &[i32], high: &[i32]) {
  let width = out.len();
  if width <= 1 {
    out[0] = low[0];
    return;
  }
  // With a tile on the left, its last high-pass value comes first
  let (first, high) = if tile.tiles_left {
    (even_sample(low[0], high[0], high[1]), &high[1..])
  } else {
    (even_sample_edge(low[0], high[0]), high)
  };
  out[0] = first;
  // Pairs of an odd and an even sample before the right edge
  let pairs = (width - 2) / 2;
  let (body, tail) = out[1..].split_at_mut(2 * pairs);
  let (lows, low_tail) = low[1..].split_at(pairs);
  let mut prev = first;
  let highs = high[..pairs].iter().zip(&high[1..=pairs]);
  for ((pair, &l), (&h0, &h1)) in body.as_chunks_mut::<2>().0.iter_mut().zip(lows).zip(highs) {
    let even = even_sample(l, h0, h1);
    *pair = [odd_sample(h0, prev, even), even];
    prev = even;
  }
  // The low-pass band has a value left only for a tile on the right or an odd width
  let h = high[pairs];
  if tile.tiles_right {
    let even = even_sample(low_tail[0], h, high[pairs + 1]);
    tail[0] = odd_sample(h, prev, even);
    if width & 1 == 1 {
      tail[1] = even;
    }
  } else if width & 1 == 1 {
    let even = even_sample_edge(low_tail[0], h);
    tail[0] = odd_sample(h, prev, even);
    tail[1] = even;
  } else {
    tail[0] = odd_sample_edge(h, prev);
  }
}

/// Copy the next line of the level below `level` into the LL band buffer
/// of `level`, reusing the allocation of the buffer
fn copy_lower_line(iwt_transforms: &mut [WaveletTransform], level: usize) {
  let (lower, upper) = iwt_transforms.split_at_mut(level);
  upper[0].band0_buf.clone_from(lower[level - 1].getline());
}

/// This structure holds the inverse transformation state
/// Each level has it's own state, so for 3 levels of DWT
/// 3 instances are required.
#[derive(Debug, Clone)]
pub(crate) struct WaveletTransform {
  /// Contains the decoded data from LL band
  /// or from a previous level decode.
  band0_buf: Vec<i32>,
  /// Contains the decoded data for HL band of current level
  band1_buf: Vec<i32>,
  /// Contains the decoded data for LH band of current level
  band2_buf: Vec<i32>,
  /// Contains the decoded data for HH band of current level
  band3_buf: Vec<i32>,

  /// Line buffers of the vertical inverse transform: `[0]` is the
  /// low-pass line, `[1]` and `[2]` are the previous and the current
  /// high-pass line, all after the horizontal inverse transform.
  /// `[3..8]` is a ring of 5 reconstructed lines, see `flt_tap_h`.
  line_buf: [Vec<i32>; 8],
  /// Current line position
  cur_line: usize,
  /// Count of lines in the ring `line_buf[3..8]` that are reconstructed but
  /// not yet returned by [`Self::getline`]. While it is above 0, the level
  /// doesn't decode or transform new lines.
  cur_h: usize,
  /// Ring position of the vertical filter tap H0: `line_buf[flt_tap_h + 3]`
  /// is the last reconstructed even line, which the next lifting step needs
  /// for the odd line after it. H1 and H2 follow at `(flt_tap_h + 1) % 5 + 3`
  /// and `(flt_tap_h + 2) % 5 + 3`.
  flt_tap_h: usize,
  /// Height of the final image for the current level
  height: usize,
  /// Width of the final image for the current level
  width: usize,
}

impl WaveletTransform {
  pub(crate) fn new(height: usize, width: usize) -> Self {
    // Line buffers for inverse transformation
    let line_buf = [
      vec![0; width],
      vec![0; width],
      vec![0; width],
      vec![0; width],
      vec![0; width],
      vec![0; width],
      vec![0; width],
      vec![0; width],
    ];
    Self {
      // We use empty vectors, they will be replaced
      // with the result of a line decode.
      band0_buf: Vec::new(),
      band1_buf: Vec::new(),
      band2_buf: Vec::new(),
      band3_buf: Vec::new(),
      line_buf,
      cur_line: 0,
      cur_h: 0,
      flt_tap_h: 0,
      height,
      width,
    }
  }

  pub(super) fn getline(&mut self) -> &Vec<i32> {
    let result = &self.line_buf[(self.flt_tap_h as i32 - self.cur_h as i32 + 5) as usize % 5 + 3];
    debug_assert!(self.cur_h > 0);
    self.cur_h -= 1;
    result
  }

  /// The line buffers `index`, each sliced to the width of the level.
  /// They are distinct mutable slices of the same length, so a loop over
  /// the width needs no bounds checks and its writes can't change the
  /// lines it reads.
  fn lines<const N: usize>(&mut self, index: [usize; N]) -> [&mut [i32]; N] {
    let width = self.width;
    self
      .line_buf
      .get_disjoint_mut(index)
      .expect("line buffer indexes are distinct")
      .map(|line| &mut line[..width])
  }
}

impl CodecParams {
  pub(super) fn idwt_53_filter_decode(
    &self,
    tile: &Tile,
    plane: &Plane,
    params: &mut Vec<BandParam>,
    iwt_transforms: &mut Vec<WaveletTransform>,
    level: usize,
  ) -> Result<()> {
    if iwt_transforms[level].cur_h > 0 {
      return Ok(());
    }
    let cur_band = 3 * level;
    let q_step_level = tile.q_step.as_ref().map(|f| &f[level]);

    if iwt_transforms[level].height - 3 <= iwt_transforms[level].cur_line && !tile.tiles_bottom {
      if iwt_transforms[level].height & 1 == 1 {
        if level > 0 {
          self.idwt_53_filter_decode(tile, plane, params, iwt_transforms, level - 1)?;
        } else {
          let sband = &plane.subbands[cur_band];
          iwt_transforms[level].band0_buf = self.decode_line_with_iquantization(sband, &mut params[cur_band], q_step_level)?;
        }
        let sband = &plane.subbands[cur_band + 1];
        iwt_transforms[level].band1_buf = self.decode_line_with_iquantization(sband, &mut params[cur_band + 1], q_step_level)?;
      }
    } else {
      if level > 0 {
        self.idwt_53_filter_decode(tile, plane, params, iwt_transforms, level - 1)?;
      } else {
        // LL band
        let sband = &plane.subbands[cur_band];
        iwt_transforms[level].band0_buf = self.decode_line_with_iquantization(sband, &mut params[cur_band], q_step_level)?;
      }

      // HL, LH and HH band
      iwt_transforms[level].band1_buf = self.decode_line_with_iquantization(&plane.subbands[cur_band + 1], &mut params[cur_band + 1], q_step_level)?;
      iwt_transforms[level].band2_buf = self.decode_line_with_iquantization(&plane.subbands[cur_band + 2], &mut params[cur_band + 2], q_step_level)?;
      iwt_transforms[level].band3_buf = self.decode_line_with_iquantization(&plane.subbands[cur_band + 3], &mut params[cur_band + 3], q_step_level)?;
    }

    Ok(())
  }

  pub(super) fn idwt_53_horizontal(&self, tile: &Tile, la: usize, lb: usize, wvlt: &mut WaveletTransform) {
    idwt_53_line(tile, &mut wvlt.line_buf[la], &wvlt.band0_buf, &wvlt.band1_buf);
    idwt_53_line(tile, &mut wvlt.line_buf[lb], &wvlt.band2_buf, &wvlt.band3_buf);
  }

  pub(super) fn idwt_53_filter_init(
    &self,
    tile: &Tile,
    plane: &Plane,
    params: &mut Vec<BandParam>,
    iwt_transforms: &mut Vec<WaveletTransform>,
    level: usize,
  ) -> Result<()> {
    assert!(level > 0);
    if level == 0 {
      // This code is not called from pathes where level is 0. But we keep this check.
      return Ok(());
    }

    let mut cur_band = 0;
    for cur_level in 0..level {
      let q_step_level = tile.q_step.as_ref().map(|f| &f[cur_level]);

      if cur_level > 0 {
        copy_lower_line(iwt_transforms, cur_level);
      } else {
        iwt_transforms[cur_level].band0_buf = self.decode_line_with_iquantization(&plane.subbands[cur_band], &mut params[cur_band], q_step_level)?;
      }

      let wvlt = &mut iwt_transforms[cur_level];

      let h0 = wvlt.flt_tap_h + 3;
      if wvlt.height > 1 {
        wvlt.band1_buf = self.decode_line_with_iquantization(&plane.subbands[cur_band + 1], &mut params[cur_band + 1], q_step_level)?;
        wvlt.band2_buf = self.decode_line_with_iquantization(&plane.subbands[cur_band + 2], &mut params[cur_band + 2], q_step_level)?;
        wvlt.band3_buf = self.decode_line_with_iquantization(&plane.subbands[cur_band + 3], &mut params[cur_band + 3], q_step_level)?;

        let l0 = 0;
        let l1 = 1;
        let l2 = 2;

        if tile.tiles_top {
          self.idwt_53_horizontal(tile, l0, 1, wvlt);
          wvlt.band3_buf = self.decode_line_with_iquantization(&plane.subbands[cur_band + 3], &mut params[cur_band + 3], q_step_level)?;
          wvlt.band2_buf = self.decode_line_with_iquantization(&plane.subbands[cur_band + 2], &mut params[cur_band + 2], q_step_level)?;

          // process L band
          idwt_53_line(tile, &mut wvlt.line_buf[l2], &wvlt.band2_buf, &wvlt.band3_buf);

          // process H band
          let [l0, l1, l2, h0] = wvlt.lines([l0, l1, l2, h0]);
          for i in 0..h0.len() {
            h0[i] = even_sample(l0[i], l1[i], l2[i]);
          }
        } else {
          self.idwt_53_horizontal(tile, l0, 2, wvlt);
          let [l0, l2, h0] = wvlt.lines([l0, l2, h0]);
          for i in 0..h0.len() {
            h0[i] = even_sample_edge(l0[i], l2[i]);
          }
          self.idwt_53_filter_decode(tile, plane, params, iwt_transforms, cur_level)?;
          self.idwt_53_filter_transform(tile, plane, params, iwt_transforms, cur_level)?;
        }
      } else {
        // This is unused in real world

        wvlt.band1_buf = self.decode_line_with_iquantization(&plane.subbands[cur_band + 1], &mut params[cur_band + 1], q_step_level)?;

        // process H band
        idwt_53_line(tile, &mut wvlt.line_buf[h0], &wvlt.band0_buf, &wvlt.band1_buf);
        wvlt.cur_line += 1;
        wvlt.cur_h += 1;
        wvlt.flt_tap_h = (wvlt.flt_tap_h + 1) % 5;
      }
      cur_band += 3;
    }

    Ok(())
  }

  pub(super) fn idwt_53_filter_transform(
    &self,
    tile: &Tile,
    plane: &Plane,
    params: &mut Vec<BandParam>,
    iwt_transforms: &mut Vec<WaveletTransform>,
    level: usize,
  ) -> Result<()> {
    if iwt_transforms[level].cur_h > 0 {
      return Ok(());
    }
    if iwt_transforms[level].cur_line >= iwt_transforms[level].height - 3 {
      if !tile.tiles_bottom {
        if iwt_transforms[level].height & 1 == 1 {
          if level > 0 {
            if iwt_transforms[level - 1].cur_h == 0 {
              self.idwt_53_filter_transform(tile, plane, params, iwt_transforms, level - 1)?;
            }
            copy_lower_line(iwt_transforms, level);
          }
          let wvlt = &mut iwt_transforms[level];
          let h0 = wvlt.flt_tap_h + 3;
          let h1 = (wvlt.flt_tap_h + 1) % 5 + 3;
          let h2 = (wvlt.flt_tap_h + 2) % 5 + 3;
          let l0 = 0;
          let l1 = 1;

          // process L bands
          idwt_53_line(tile, &mut wvlt.line_buf[l0], &wvlt.band0_buf, &wvlt.band1_buf);

          // process H bands
          wvlt.line_buf.swap(1, 2);

          let [l0, l1, h0, h1, h2] = wvlt.lines([l0, l1, h0, h1, h2]);
          for i in 0..h1.len() {
            let delta = even_sample_edge(l0[i], l1[i]);
            h1[i] = odd_sample(l1[i], h0[i], delta);
            h2[i] = delta;
          }
          wvlt.cur_h += 3;
          wvlt.cur_line += 3;
          wvlt.flt_tap_h = (wvlt.flt_tap_h + 3) % 5;
        } else {
          let wvlt = &mut iwt_transforms[level];
          let l2 = 2;
          let h0 = wvlt.flt_tap_h + 3;
          let h1 = (wvlt.flt_tap_h + 1) % 5 + 3;

          let [l2, h0, h1] = wvlt.lines([l2, h0, h1]);
          for i in 0..h1.len() {
            h1[i] = odd_sample_edge(l2[i], h0[i]);
          }

          // The original libraw CRX decoder copies the pointer from line_buf[2] to [1].
          // But it doesn't makes sense, so we swap the buffers as we do on other locations.
          // The swap has no effect: these are the last lines of the level, the buffers
          // are not used again.
          wvlt.line_buf.swap(1, 2);
          //wvlt.line_buf[1] = wvlt.line_buf[2].clone();

          wvlt.cur_h += 2;
          wvlt.cur_line += 2;
          wvlt.flt_tap_h = (wvlt.flt_tap_h + 2) % 5;
        }
      } // end if !tile.tiles_bottom
    } else {
      if level > 0 {
        if iwt_transforms[level - 1].cur_h == 0 {
          self.idwt_53_filter_transform(tile, plane, params, iwt_transforms, level - 1)?;
        }
        copy_lower_line(iwt_transforms, level);
      }
      let wvlt = &mut iwt_transforms[level];

      let l0 = 0;
      let l1 = 1;
      let h0 = wvlt.flt_tap_h + 3;
      let h1 = (wvlt.flt_tap_h + 1) % 5 + 3;
      let h2 = (wvlt.flt_tap_h + 2) % 5 + 3;

      // process L bands
      idwt_53_line(tile, &mut wvlt.line_buf[l0], &wvlt.band0_buf, &wvlt.band1_buf);
      idwt_53_line(tile, &mut wvlt.line_buf[l1], &wvlt.band2_buf, &wvlt.band3_buf);

      // process H bands
      let wvlt = &mut iwt_transforms[level];

      wvlt.line_buf.swap(1, 2);

      let l0 = 0;
      let l1 = 1;
      let l2 = 2;
      let [l0, l1, l2, h0, h1, h2] = wvlt.lines([l0, l1, l2, h0, h1, h2]);
      for i in 0..h1.len() {
        let delta = even_sample(l0[i], l2[i], l1[i]);
        h1[i] = odd_sample(l1[i], h0[i], delta);
        h2[i] = delta;
      }

      if iwt_transforms[level].cur_line >= iwt_transforms[level].height - 3 && iwt_transforms[level].height & 1 == 1 {
        iwt_transforms[level].cur_h += 3;
        iwt_transforms[level].cur_line += 3;
        iwt_transforms[level].flt_tap_h = (iwt_transforms[level].flt_tap_h + 3) % 5;
      } else {
        iwt_transforms[level].cur_h += 2;
        iwt_transforms[level].cur_line += 2;
        iwt_transforms[level].flt_tap_h = (iwt_transforms[level].flt_tap_h + 2) % 5;
      }
    }

    Ok(())
  }
}
