// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

// Original Crx decoder crx.cpp was written by Alexey Danilchenko for libraw.
// Rewritten in Rust by Daniel Vogelbacher, based on logic found in
// crx.cpp and documentation done by Laurent Clévy (https://github.com/lclevy/canon_cr3).

use super::BitPump;
use super::Result;
use bitstream_io::BitRead;

/// Adaptive Golomb-Rice decoder
pub(super) struct RiceDecoder<'mdat> {
  /// Bitstream from MDAT
  bitpump: BitPump<'mdat>,
  k_param: u32,
}

impl<'mdat> RiceDecoder<'mdat> {
  /// Create new decoder for given bit pump
  pub(super) fn new(bitpump: BitPump<'mdat>) -> Self {
    Self { bitpump, k_param: 0 }
  }

  /// Get current K parameter
  #[inline(always)]
  pub(super) fn k(&self) -> u32 {
    self.k_param
  }

  /// Set K parameter
  #[inline(always)]
  pub(super) fn set_k(&mut self, k: u32) {
    self.k_param = k;
  }

  /// Return the positive number of 0-bits in bitstream.
  /// All 0-bits are consumed.
  #[inline(always)]
  pub(super) fn bitstream_zeros(&mut self) -> Result<u32> {
    Ok(self.bitpump.read_unary1()?)
  }

  /// Return the requested bits
  // All bits are consumed.
  // The maximum number of bits are 32
  #[inline(always)]
  pub(super) fn bitstream_get_bits(&mut self, bits: u32) -> Result<u32> {
    debug_assert!(bits <= 32);
    Ok(self.bitpump.read(bits)?)
  }

  /// Golomb-Rice decoding
  /// https://w3.ual.es/~vruiz/Docencia/Apuntes/Coding/Text/03-symbol_encoding/09-Golomb_coding/index.html
  /// escape and esc_bits are used to interrupt decoding when
  /// a value is not encoded using Golomb-Rice but directly encoded
  /// by esc_bits bits.
  fn rice_decode(&mut self, escape: u32, esc_bits: u32) -> Result<u32> {
    // q, quotient = n//m, with m = 2^k (Rice coding)
    let prefix = self.bitstream_zeros()?;
    if prefix >= escape {
      // n
      Ok(self.bitstream_get_bits(esc_bits)?)
    } else if self.k_param > 0 {
      // Golomb-Rice coding : n = q * 2^k + r, with r is next k bits. r is n - (q*2^k)
      Ok((prefix << self.k_param) | self.bitstream_get_bits(self.k_param)?)
    } else {
      // q
      Ok(prefix)
    }
  }

  /// Adaptive Golomb-Rice decoding, by adapting k value
  /// Sometimes adapting is based on the next coefficent (n) instead
  /// of current (x) coefficent. So you can disable it with `adapt_k`
  /// and update k later.
  pub(super) fn adaptive_rice_decode(&mut self, adapt_k: bool, escape: u32, esc_bits: u32, k_max: u32) -> Result<u32> {
    let val = self.rice_decode(escape, esc_bits)?;
    if adapt_k {
      self.k_param = Self::predict_k_param_max(self.k_param, val, k_max);
    }
    Ok(val)
  }

  /// Update current K parameter
  pub(super) fn update_k_param(&mut self, bit_code: u32, k_max: u32) {
    self.k_param = Self::predict_k_param_max(self.k_param, bit_code, k_max);
  }

  /// Predict K parameter with maximum constraint
  /// Golomb-Rice becomes more efficient when used with an adaptive
  /// K parameter. This is done by predicting the next K value for the
  /// next sample value.
  fn predict_k_param_max(prev_k: u32, value: u32, k_max: u32) -> u32 {
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

    if k_max > 0 {
      std::cmp::min(new_k, k_max)
    } else {
      new_k
    }
  }
}
