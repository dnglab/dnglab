// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

pub mod superpixel;

/// Bayer matrix pattern
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BayerPattern {
  RGGB,
  BGGR,
  GBRG,
  GRBG,
  //ERBG,
  //RGEB,
}
