// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

// Original Crx decoder crx.cpp was written by Alexey Danilchenko for libraw.
// Rewritten in Rust by Daniel Vogelbacher, based on logic found in
// crx.cpp and documentation done by Laurent Clévy (https://github.com/lclevy/canon_cr3).

use super::{
  mdat::{Plane, Tile},
  BandParam, CodecParams, Result,
};

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
  /// 8 temporary buffers for inverse transformation (5/3?)
  band0_pos: usize,
  band1_pos: usize,
  band2_pos: usize,
  band3_pos: usize,

  line_buf: [Vec<i32>; 8],
  /// Current line position
  cur_line: usize,
  /// TODO ???
  cur_h: usize,
  /// TODO ???
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
      band0_pos: 0,
      band1_pos: 0,
      band2_pos: 0,
      band3_pos: 0,
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

  pub(super) fn band0(&mut self, offset: usize) -> i32 {
    self.band0_buf[self.band0_pos + offset]
  }

  pub(super) fn band1(&mut self, offset: usize) -> i32 {
    self.band1_buf[self.band1_pos + offset]
  }

  pub(super) fn band2(&mut self, offset: usize) -> i32 {
    self.band2_buf[self.band2_pos + offset]
  }

  pub(super) fn band3(&mut self, offset: usize) -> i32 {
    self.band3_buf[self.band3_pos + offset]
  }

  pub(super) fn advance_bufs(&mut self, count: usize) {
    self.band0_pos += count;
    self.band1_pos += count;
    self.band2_pos += count;
    self.band3_pos += count;
  }

  pub(super) fn reset_bufs(&mut self) {
    self.band0_pos = 0;
    self.band1_pos = 0;
    self.band2_pos = 0;
    self.band3_pos = 0;
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
    //let mut b0pos = 0;
    //let mut b1pos = 0;
    //let mut b2pos = 0;
    //let mut b3pos = 0;
    let mut lapos = 0;
    let mut lbpos = 0;
    wvlt.reset_bufs();

    if wvlt.width <= 1 {
      wvlt.line_buf[la][0] = wvlt.band0(0);
      wvlt.line_buf[lb][0] = wvlt.band2(0);
    } else {
      if tile.tiles_left {
        // Untested
        wvlt.line_buf[la][0] = wvlt.band0(0) - ((wvlt.band1(0) + wvlt.band1(1) + 2) >> 2);
        wvlt.line_buf[lb][0] = wvlt.band2(0) - ((wvlt.band3(0) + wvlt.band3(1) + 2) >> 2);
        wvlt.band1_pos += 1;
        wvlt.band3_pos += 1;
      } else {
        wvlt.line_buf[la][0] = wvlt.band0(0) - ((wvlt.band1(0) + 1) >> 1);
        wvlt.line_buf[lb][0] = wvlt.band2(0) - ((wvlt.band3(0) + 1) >> 1);
      }
      wvlt.band0_pos += 1;
      wvlt.band2_pos += 1;

      //println!("config: tile: {}, {}, band1 width: {}", tile.id, wvlt.width - 3, wvlt.band1_buf.len());
      for _i in (0..(wvlt.width - 3)).step_by(2) {
        //println!("val: {}, band1_pos: {}, band1_size: {}", _i, wvlt.band1_pos, wvlt.band1_buf.len());

        let delta = wvlt.band0(0) - ((wvlt.band1(0) + wvlt.band1(1) + 2) >> 2);
        wvlt.line_buf[la][lapos + 1] = wvlt.band1(0) + ((delta + wvlt.line_buf[la][lapos]) >> 1);
        wvlt.line_buf[la][lapos + 2] = delta;

        let delta = wvlt.band2(0) - ((wvlt.band3(0) + wvlt.band3(1) + 2) >> 2);
        wvlt.line_buf[lb][lbpos + 1] = wvlt.band3(0) + ((delta + wvlt.line_buf[lb][lbpos]) >> 1);
        wvlt.line_buf[lb][lbpos + 2] = delta;

        wvlt.advance_bufs(1);

        lapos += 2;
        lbpos += 2;
      }
      if tile.tiles_right {
        // Untested
        let delta_a = wvlt.band0(0) - ((wvlt.band1(0) + wvlt.band1(1) + 2) >> 2);
        wvlt.line_buf[la][lapos + 1] = wvlt.band1(0) + ((delta_a + wvlt.line_buf[la][lapos + 0]) >> 1);

        let delta_b = wvlt.band2(0) - ((wvlt.band3(0) + wvlt.band3(1) + 2) >> 2);
        wvlt.line_buf[lb][lbpos + 1] = wvlt.band3(0) + ((delta_b + wvlt.line_buf[lb][lbpos + 0]) >> 1);

        if wvlt.width & 1 == 1 {
          wvlt.line_buf[la][lapos + 2] = delta_a;
          wvlt.line_buf[lb][lbpos + 2] = delta_b;
        }
      } else if wvlt.width & 1 == 1 {
        wvlt.line_buf[la][lapos + 1] = wvlt.band1(0) + ((wvlt.line_buf[la][lapos] + wvlt.band0(0) - ((wvlt.band1(0) + 1) >> 1)) >> 1);
        wvlt.line_buf[la][lapos + 2] = wvlt.band0(0) - ((wvlt.band1(0) + 1) >> 1);

        wvlt.line_buf[lb][lbpos + 1] = wvlt.band3(0) + ((wvlt.line_buf[lb][lbpos] + wvlt.band2(0) - ((wvlt.band3(0) + 1) >> 1)) >> 1);
        wvlt.line_buf[lb][lbpos + 2] = wvlt.band2(0) - ((wvlt.band3(0) + 1) >> 1);
      } else {
        wvlt.line_buf[la][lapos + 1] = wvlt.line_buf[la][lapos + 0] + wvlt.band1(0);
        wvlt.line_buf[lb][lbpos + 1] = wvlt.line_buf[lb][lbpos + 0] + wvlt.band3(0);
      }
    }
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
        iwt_transforms[cur_level].band0_buf = iwt_transforms[cur_level - 1].getline().clone();
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
        let mut l2_pos = 0;

        if tile.tiles_top {
          self.idwt_53_horizontal(tile, l0, 1, wvlt);
          wvlt.band3_buf = self.decode_line_with_iquantization(&plane.subbands[cur_band + 3], &mut params[cur_band + 3], q_step_level)?;
          wvlt.band2_buf = self.decode_line_with_iquantization(&plane.subbands[cur_band + 2], &mut params[cur_band + 2], q_step_level)?;

          // process L band
          if wvlt.width <= 1 {
            wvlt.line_buf[l2][0] = wvlt.band2(0);
          } else {
            if tile.tiles_left {
              wvlt.line_buf[l2][0] = wvlt.band2(0) - ((wvlt.band3(0) + wvlt.band3(1) + 2) >> 2);
              wvlt.band3_pos += 1;
            } else {
              wvlt.line_buf[l2][0] = wvlt.band2(0) - ((wvlt.band3(0) + 1) >> 1);
            }
            wvlt.band2_pos += 1;

            for _i in (0..wvlt.width - 3).step_by(2) {
              let delta = wvlt.band2(0) - ((wvlt.band3(0) + wvlt.band3(1) + 2) >> 2);
              wvlt.line_buf[l2][1] = wvlt.band3(0) + ((wvlt.line_buf[l2][l2_pos] + delta) >> 1);
              wvlt.line_buf[l2][2] = delta;
              wvlt.band2_pos += 1;
              wvlt.band3_pos += 1;
              l2_pos += 2;
            }

            if tile.tiles_right {
              let delta = wvlt.band2(0) - ((wvlt.band3(0) + wvlt.band3(1) + 2) >> 2);
              wvlt.line_buf[l2][l2_pos + 1] = wvlt.band3(0) + ((wvlt.line_buf[l2][l2_pos + 0] + delta) >> 1);
              if wvlt.width & 1 == 1 {
                wvlt.line_buf[l2][l2_pos + 1] = delta;
              }
            } else if wvlt.width & 1 == 1 {
              let delta = wvlt.band2(0) - ((wvlt.band3(0) + 1) >> 1);
              wvlt.line_buf[l2][l2_pos + 1] = wvlt.band3(0) + ((wvlt.line_buf[l2][l2_pos + 0] + delta) >> 1);
              wvlt.line_buf[l2][l2_pos + 2] = delta;
            } else {
              wvlt.line_buf[l2][l2_pos + 1] = wvlt.band3(0) + wvlt.line_buf[l2][l2_pos + 0];
            }
          }

          // process H band
          for i in 0..wvlt.width {
            wvlt.line_buf[h0][i] = wvlt.line_buf[l0][i] - ((wvlt.line_buf[l1][i] + wvlt.line_buf[l2][i] + 2) >> 2);
          }
        } else {
          self.idwt_53_horizontal(tile, l0, 2, wvlt);
          for i in 0..wvlt.width {
            wvlt.line_buf[h0][i] = wvlt.line_buf[l0][i] - ((wvlt.line_buf[l2][i] + 1) >> 1);
          }
          self.idwt_53_filter_decode(tile, plane, params, iwt_transforms, cur_level)?;
          self.idwt_53_filter_transform(tile, plane, params, iwt_transforms, cur_level)?;
        }
      } else {
        // This is unused in real world

        wvlt.band1_buf = self.decode_line_with_iquantization(&plane.subbands[cur_band + 1], &mut params[cur_band + 1], q_step_level)?;
        let mut h0_pos = 0;

        // process H band
        if wvlt.width <= 1 {
          wvlt.line_buf[h0][0] = wvlt.band0(0);
        } else {
          if tile.tiles_left {
            wvlt.line_buf[h0][h0_pos] = wvlt.band0(0) - ((wvlt.band1(0) + wvlt.band1(1) + 2) >> 2);
            wvlt.band1_pos += 1;
          } else {
            wvlt.line_buf[h0][h0_pos] = wvlt.band0(0) - ((wvlt.band1(0) + 1) >> 1);
          }
          wvlt.band0_pos += 1;

          for _i in (0..(wvlt.width - 3)).step_by(2) {
            let delta = wvlt.band0(0) - ((wvlt.band1(0) + wvlt.band1(1) + 2) >> 2);
            wvlt.line_buf[h0][h0_pos + 1] = wvlt.band1(0) + ((wvlt.line_buf[h0][h0_pos + 0] + delta) >> 1);
            wvlt.line_buf[h0][h0_pos + 2] = delta;
            wvlt.band0_pos += 1;
            wvlt.band1_pos += 1;
            h0_pos += 2;
          }

          if tile.tiles_right {
            // untested
            let delta = wvlt.band0(0) - ((wvlt.band1(0) + wvlt.band1(1) + 2) >> 2);
            wvlt.line_buf[h0][h0_pos + 1] = wvlt.band1(0) + ((wvlt.line_buf[h0][h0_pos + 0] + delta) >> 1);
            wvlt.line_buf[h0][h0_pos + 2] = delta;
          } else if wvlt.width & 1 == 1 {
            let delta = wvlt.band0(0) - ((wvlt.band1(0) + 1) >> 1);
            wvlt.line_buf[h0][h0_pos + 1] = wvlt.band1(0) + ((wvlt.line_buf[h0][h0_pos + 0] + delta) >> 1);
            wvlt.line_buf[h0][h0_pos + 2] = delta;
          } else {
            wvlt.line_buf[h0][h0_pos + 1] = wvlt.band1(0) + wvlt.line_buf[h0][h0_pos + 0];
          }
        }
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
            iwt_transforms[level].band0_buf = iwt_transforms[level - 1].getline().clone();
          }
          let wvlt = &mut iwt_transforms[level];
          wvlt.reset_bufs();
          let h0 = wvlt.flt_tap_h + 3;
          let h1 = (wvlt.flt_tap_h + 1) % 5 + 3;
          let h2 = (wvlt.flt_tap_h + 2) % 5 + 3;
          let l0 = 0;
          let l1 = 1;
          let mut l0_pos = 0;
          //let mut l1_pos = 0;

          // process L bands
          if wvlt.width <= 1 {
            wvlt.line_buf[l0][0] = wvlt.band0(0);
          } else {
            if tile.tiles_left {
              // untested
              wvlt.line_buf[l0][l0_pos] = wvlt.band0(0) - ((wvlt.band1(0) + wvlt.band1(1) + 2) >> 2);
              wvlt.band1_pos += 1;
            } else {
              wvlt.line_buf[l0][l0_pos] = wvlt.band0(0) - ((wvlt.band1(0) + 1) >> 1);
            }
            wvlt.band0_pos += 1;
            for _i in (0..(wvlt.width - 3)).step_by(2) {
              let delta = wvlt.band0(0) - ((wvlt.band1(0) + wvlt.band1(1) + 2) >> 2);
              wvlt.line_buf[l0][l0_pos + 1] = wvlt.band1(0) + ((wvlt.line_buf[l0][l0_pos + 0] + delta) >> 1);
              wvlt.line_buf[l0][l0_pos + 2] = delta;
              wvlt.band0_pos += 1;
              wvlt.band1_pos += 1;
              l0_pos += 2;
            }
            if tile.tiles_right {
              // untested
              let delta = wvlt.band0(0) - ((wvlt.band1(0) + wvlt.band1(1) + 2) >> 2);
              wvlt.line_buf[l0][l0_pos + 1] = wvlt.band1(0) + ((wvlt.line_buf[l0][l0_pos + 0] + delta) >> 1);
              if wvlt.width & 1 == 1 {
                wvlt.line_buf[l0][l0_pos + 2] = delta;
              }
            } else if wvlt.width & 1 == 1 {
              let delta = wvlt.band0(0) - ((wvlt.band1(0) + 1) >> 1);
              wvlt.line_buf[l0][l0_pos + 1] = wvlt.band1(0) + ((wvlt.line_buf[l0][l0_pos + 0] + delta) >> 1);
              wvlt.line_buf[l0][l0_pos + 2] = delta;
            } else {
              wvlt.line_buf[l0][l0_pos + 1] = wvlt.band1(0) + wvlt.line_buf[l0][l0_pos + 0];
            }
          }

          // process H bands
          //wvlt.reset_bufs();

          wvlt.line_buf.swap(1, 2);

          for i in 0..wvlt.width {
            let delta = wvlt.line_buf[l0][i] - ((wvlt.line_buf[l1][i] + 1) >> 1);
            wvlt.line_buf[h1][i] = wvlt.line_buf[l1][i] + ((delta + wvlt.line_buf[h0][i]) >> 1);
            wvlt.line_buf[h2][i] = delta;
          }
          wvlt.cur_h += 3;
          wvlt.cur_line += 3;
          wvlt.flt_tap_h = (wvlt.flt_tap_h + 3) % 5;
        } else {
          let wvlt = &mut iwt_transforms[level];
          let l2 = 2;
          let h0 = wvlt.flt_tap_h + 3;
          let h1 = (wvlt.flt_tap_h + 1) % 5 + 3;

          for i in 0..wvlt.width {
            wvlt.line_buf[h1][i] = wvlt.line_buf[h0][i] + wvlt.line_buf[l2][i];
          }

          // The original libraw CRX decoder copies the pointer from line_buf[2] to [1].
          // But it doesn't makes sense, so we swap the buffers as we do on other locations.
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
        iwt_transforms[level].band0_buf = iwt_transforms[level - 1].getline().clone();
      }
      let wvlt = &mut iwt_transforms[level];

      wvlt.reset_bufs();

      let l0 = 0;
      let l1 = 1;
      //let l2 = 2;
      let mut l0_pos = 0;
      let mut l1_pos = 0;
      //let mut l2_pos = 0;
      let h0 = wvlt.flt_tap_h + 3;
      let h1 = (wvlt.flt_tap_h + 1) % 5 + 3;
      let h2 = (wvlt.flt_tap_h + 2) % 5 + 3;

      // process L bands
      if wvlt.width <= 1 {
        wvlt.line_buf[l0][0] = wvlt.band0(0);
        wvlt.line_buf[l1][0] = wvlt.band2(0);
      } else {
        // untested
        if tile.tiles_left {
          wvlt.line_buf[l0][0] = wvlt.band0(0) - ((wvlt.band1(0) + wvlt.band1(1) + 2) >> 2);
          wvlt.line_buf[l1][0] = wvlt.band2(0) - ((wvlt.band3(0) + wvlt.band3(1) + 2) >> 2);
          wvlt.band1_pos += 1;
          wvlt.band3_pos += 1;
        } else {
          wvlt.line_buf[l0][0] = wvlt.band0(0) - ((wvlt.band1(0) + 1) >> 1);
          wvlt.line_buf[l1][0] = wvlt.band2(0) - ((wvlt.band3(0) + 1) >> 1);
        }
        wvlt.band0_pos += 1;
        wvlt.band2_pos += 1;
        for _i in (0..(wvlt.width - 3)).step_by(2) {
          let delta = wvlt.band0(0) - ((wvlt.band1(0) + wvlt.band1(1) + 2) >> 2);
          wvlt.line_buf[l0][l0_pos + 1] = wvlt.band1(0) + ((delta + wvlt.line_buf[l0][l0_pos + 0]) >> 1);
          wvlt.line_buf[l0][l0_pos + 2] = delta;
          let delta = wvlt.band2(0) - ((wvlt.band3(0) + wvlt.band3(1) + 2) >> 2);
          wvlt.line_buf[l1][l1_pos + 1] = wvlt.band3(0) + ((delta + wvlt.line_buf[l1][l1_pos + 0]) >> 1);
          wvlt.line_buf[l1][l1_pos + 2] = delta;
          wvlt.advance_bufs(1);
          l0_pos += 2;
          l1_pos += 2;
        }
        if tile.tiles_right {
          // untested
          let delta_a = wvlt.band0(0) - ((wvlt.band1(0) + wvlt.band1(1) + 2) >> 2);
          wvlt.line_buf[l0][l0_pos + 1] = wvlt.band1(0) + ((delta_a + wvlt.line_buf[l0][l0_pos + 0]) >> 1);

          let delta_b = wvlt.band2(0) - ((wvlt.band3(0) + wvlt.band3(1) + 2) >> 2);
          wvlt.line_buf[l1][l1_pos + 1] = wvlt.band3(0) + ((delta_b + wvlt.line_buf[l1][l1_pos + 0]) >> 1);

          if wvlt.width & 1 == 1 {
            wvlt.line_buf[l0][l0_pos + 2] = delta_a;
            wvlt.line_buf[l1][l1_pos + 2] = delta_b;
          }
        } else if wvlt.width & 1 == 1 {
          let delta = wvlt.band0(0) - ((wvlt.band1(0) + 1) >> 1);
          wvlt.line_buf[l0][l0_pos + 1] = wvlt.band1(0) + ((delta + wvlt.line_buf[l0][l0_pos + 0]) >> 1);
          wvlt.line_buf[l0][l0_pos + 2] = delta;

          let delta = wvlt.band2(0) - ((wvlt.band3(0) + 1) >> 1);
          wvlt.line_buf[l1][l1_pos + 1] = wvlt.band3(0) + ((delta + wvlt.line_buf[l1][l1_pos + 0]) >> 1);
          wvlt.line_buf[l1][l1_pos + 2] = delta;
        } else {
          wvlt.line_buf[l0][l0_pos + 1] = wvlt.line_buf[l0][l0_pos] + wvlt.band1(0);
          wvlt.line_buf[l1][l1_pos + 1] = wvlt.line_buf[l1][l1_pos] + wvlt.band3(0);
        }
      }

      // process H bands
      let wvlt = &mut iwt_transforms[level];

      wvlt.line_buf.swap(1, 2);

      let l0 = 0;
      let l1 = 1;
      let l2 = 2;
      for i in 0..wvlt.width {
        let delta = wvlt.line_buf[l0][i] - ((wvlt.line_buf[l2][i] + wvlt.line_buf[l1][i] + 2) >> 2);
        wvlt.line_buf[h1][i] = wvlt.line_buf[l1][i] + ((delta + wvlt.line_buf[h0][i]) >> 1);
        wvlt.line_buf[h2][i] = delta;
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
