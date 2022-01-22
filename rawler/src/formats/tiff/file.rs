// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::IFD;
use serde::{Deserialize, Serialize};

/// Reader for TIFF files
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TiffFile {
  /// Chain of all IFDs in TIFF
  pub chain: Vec<IFD>,
  /// Base offset, starting from file or buffer (good for embedded TIFF in other structures)
  pub base: u32,
  /// Offset correction value
  pub corr: i32,
}

impl TiffFile {
  pub fn new(base: u32, corr: i32) -> Self {
    Self { base, corr, chain: Vec::new() }
  }

  pub fn push_ifd(&mut self, ifd: IFD) {
    self.chain.push(ifd);
  }
}
