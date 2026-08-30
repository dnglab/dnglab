// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026

//! Bit-level reader for JPEG XS precinct payloads.
//!
//! The coded sub-packets need three read sizes out of one stream: single bits
//! for significance flags, sign bits and unary codes; nibbles for raw GCLIs
//! and coefficient bit planes, which the encoder keeps 4-bit aligned; and
//! whole bytes for headers and sub-packet boundaries. Everything is MSB
//! first, and every read is bounds-checked.

use crate::RawlerError;
use crate::Result;

#[derive(Debug, Clone)]
pub struct BitReader<'a> {
  buf: &'a [u8],
  /// Byte offset of the next unread byte (the one `bits` indexes into).
  pos: usize,
  /// Bits already consumed from `buf[pos]`, 0..=7.
  bits: u8,
}

impl<'a> BitReader<'a> {
  pub fn at(buf: &'a [u8], pos: usize) -> Self {
    Self { buf, pos, bits: 0 }
  }

  fn eof<T>(&self, what: &str) -> Result<T> {
    Err(RawlerError::DecoderFailed(format!(
      "JPEG XS: bitstream ended while reading {} at byte {} of {}",
      what,
      self.pos,
      self.buf.len()
    )))
  }

  /// Byte offset of the reading position, rounded down.
  pub fn byte_pos(&self) -> usize {
    self.pos
  }

  pub fn bit(&mut self) -> Result<u8> {
    if self.pos >= self.buf.len() {
      return self.eof("a bit");
    }
    let v = (self.buf[self.pos] >> (7 - self.bits)) & 1;
    self.bits += 1;
    if self.bits == 8 {
      self.bits = 0;
      self.pos += 1;
    }
    Ok(v)
  }

  /// Read one nibble. The reader must sit on a 4-bit boundary, which the
  /// coefficient data sub-packet guarantees by construction.
  pub fn nibble(&mut self) -> Result<u8> {
    if self.pos >= self.buf.len() {
      return self.eof("a nibble");
    }
    match self.bits {
      0 => {
        self.bits = 4;
        Ok(self.buf[self.pos] >> 4)
      }
      4 => {
        self.bits = 0;
        let v = self.buf[self.pos] & 0x0f;
        self.pos += 1;
        Ok(v)
      }
      _ => Err(RawlerError::DecoderFailed(format!(
        "JPEG XS: nibble read at bit offset {} (not 4-bit aligned)",
        self.bits
      ))),
    }
  }

  /// Read a unary code: the number of 1-bits before the terminating 0-bit.
  ///
  /// The reference decoder holds the code in a 32-bit register, so a run of
  /// 32 ones is corrupt rather than a value.
  pub fn unary(&mut self) -> Result<u32> {
    let mut n = 0u32;
    while self.bit()? == 1 {
      n += 1;
      if n >= 32 {
        return Err(RawlerError::DecoderFailed("JPEG XS: unary code exceeds 31, corrupt stream".into()));
      }
    }
    Ok(n)
  }

  /// The reader must be byte-aligned.
  pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
    if self.bits != 0 {
      return Err(RawlerError::DecoderFailed(format!(
        "JPEG XS: byte read at bit offset {} (not byte-aligned)",
        self.bits
      )));
    }
    if self.pos + n > self.buf.len() {
      return self.eof("bytes");
    }
    let v = &self.buf[self.pos..self.pos + n];
    self.pos += n;
    Ok(v)
  }

  /// Advance to the next byte boundary. A no-op when already aligned.
  pub fn align(&mut self) {
    if self.bits != 0 {
      self.bits = 0;
      self.pos += 1;
    }
  }

  /// The reader must be byte-aligned.
  pub fn skip_bytes(&mut self, n: usize) -> Result<()> {
    debug_assert!(self.bits == 0);
    if self.pos + n > self.buf.len() {
      return self.eof("skipped padding");
    }
    self.pos += n;
    Ok(())
  }

  /// Skip forward over bits at any alignment.
  pub fn skip_bits(&mut self, n: usize) -> Result<()> {
    let target = self
      .pos
      .checked_mul(8)
      .and_then(|b| b.checked_add(self.bits as usize + n))
      .ok_or_else(|| RawlerError::DecoderFailed("JPEG XS: bit position overflow".into()))?;
    if target > self.buf.len() * 8 {
      return self.eof("skipped bits");
    }
    self.pos = target / 8;
    self.bits = (target % 8) as u8;
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn reads_bits_msb_first() {
    let mut r = BitReader::at(&[0b1010_0011, 0b1100_0000], 0);
    let bits: Vec<u8> = (0..10).map(|_| r.bit().unwrap()).collect();
    assert_eq!(bits, [1, 0, 1, 0, 0, 0, 1, 1, 1, 1]);
    // Ten bits in, the reader sits mid-byte; aligning rounds up.
    assert_eq!(r.byte_pos(), 1);
    r.align();
    assert_eq!(r.byte_pos(), 2);
    assert!(r.bit().is_err());
  }

  #[test]
  fn reads_nibbles_on_half_byte_boundaries() {
    let mut r = BitReader::at(&[0xab, 0xcd], 0);
    assert_eq!(r.nibble().unwrap(), 0xa);
    assert_eq!(r.nibble().unwrap(), 0xb);
    assert_eq!(r.nibble().unwrap(), 0xc);
    // A nibble read off the 4-bit grid is refused.
    let mut r = BitReader::at(&[0xff], 0);
    r.bit().unwrap();
    assert!(r.nibble().is_err());
  }

  #[test]
  fn decodes_unary_codes() {
    // 110 10 0 1110 -> 2, 1, 0, 3.
    let mut r = BitReader::at(&[0b1101_0011, 0b1000_0000], 0);
    assert_eq!(r.unary().unwrap(), 2);
    assert_eq!(r.unary().unwrap(), 1);
    assert_eq!(r.unary().unwrap(), 0);
    assert_eq!(r.unary().unwrap(), 3);
    // A run of 32 ones is corrupt, not a value.
    let mut r = BitReader::at(&[0xff; 5], 0);
    assert!(r.unary().is_err());
  }

  #[test]
  fn skips_bits_and_bytes() {
    let mut r = BitReader::at(&[0x00, 0x00, 0b0100_0000], 0);
    r.skip_bits(17).unwrap();
    assert_eq!(r.bit().unwrap(), 1);
    let mut r = BitReader::at(&[0; 4], 1);
    r.skip_bytes(3).unwrap();
    assert!(r.skip_bytes(1).is_err());
  }
}
