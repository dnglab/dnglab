// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

// Original Crx decoder crx.cpp was written by Alexey Danilchenko for libraw.
// Rewritten in Rust by Daniel Vogelbacher, based on logic found in
// crx.cpp and documentation done by Laurent Clévy (https://github.com/lclevy/canon_cr3).

use super::{BandParam, CodecParams, CrxError, Result};

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

impl CodecParams {
  /// Get symbol run count for run-length decoding
  /// See T.87 Section A.7.1.2 Run-length coding
  pub(super) fn symbol_run_count(&self, param: &mut BandParam, remaining: u32) -> Result<u32> {
    assert!(remaining > 1);
    let mut run_cnt: u32 = 1;
    // See T.87 A.7.1.2 Code segment A.15
    // Bitstream 111110... means 5 lookups into J to decode final RUNcnt
    while run_cnt != remaining && param.rice.bitstream_get_bits(1)? == 1 {
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
        run_cnt += param.rice.bitstream_get_bits(J[param.s_param as usize])?;
      }
      param.s_param = param.s_param.saturating_sub(1); // prevent underflow
      if run_cnt > remaining {
        //println!("run_cnt: {}, remaining: {}", run_cnt, remaining);
        return Err(CrxError::General(format!("Crx decoder error while decoding line")));
      }
    }
    Ok(run_cnt)
  }
}
