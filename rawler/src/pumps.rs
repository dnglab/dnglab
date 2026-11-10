use std::fmt;
use std::slice::{Chunks, Iter};

use crate::bits::*;

/// Maximum count of bits a [`BitPump`] returns at once
const MAX_PEEK_BITS: u32 = u32::BITS;

/// Error of a [`BitPump`] read
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitPumpError {
  /// The buffer has fewer than the requested bits left
  Eof,
  /// More bits requested at once than the pump can return
  ExcessiveBits { num: u32, max: u32 },
}

impl fmt::Display for BitPumpError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Eof => write!(f, "unexpected end of bit stream"),
      Self::ExcessiveBits { num, max } => write!(f, "can't read {num} bits at once from bit stream, max is {max}"),
    }
  }
}

impl std::error::Error for BitPumpError {}

impl From<BitPumpError> for String {
  #[cold]
  fn from(err: BitPumpError) -> Self {
    err.to_string()
  }
}

/// Check that a read of `num` bits fits into the result of a pump
/// that returns at most `max` bits at once.
#[inline(always)]
pub(crate) fn check_bits(num: u32, max: u32) -> Result<(), BitPumpError> {
  if num > max { Err(excessive_bits(num, max)) } else { Ok(()) }
}

// Errors are built in cold functions, so the compiler keeps the error
// branches out of the hot decoder loops.
#[cold]
fn excessive_bits(num: u32, max: u32) -> BitPumpError {
  BitPumpError::ExcessiveBits { num, max }
}

#[cold]
fn eof() -> BitPumpError {
  BitPumpError::Eof
}

#[derive(Debug, Clone)]
pub struct BitPumpLSB<'a> {
  buffer: Chunks<'a, u8>,
  bits: u64,
  nbits: u32,
}

impl<'a> BitPumpLSB<'a> {
  pub fn new(src: &'a [u8]) -> Self {
    Self {
      buffer: src.chunks(size_of::<u32>()),
      bits: 0,
      nbits: 0,
    }
  }

  /// Refill internal bit buffer - Little-Endian
  ///
  /// For fast refill, we can simply take a whole u32 value out.
  /// For slow refill, there may be 1, 2 or 3 bytes left in buffer. We need
  /// to collect them manually.
  fn refill(&mut self) -> Option<(u32, u32)> {
    let chunk = self.buffer.next()?;
    if chunk.len() == 4 {
      // Fast refill
      let bits: u32 = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
      Some((bits, u32::BITS))
    } else {
      // Slow refill
      Some(chunk.iter().rev().fold((0, 0), |(bits, bit_cnt), x| ((bits << 8) | *x as u32, bit_cnt + 8)))
    }
  }
}

#[derive(Debug, Clone)]
pub struct BitPumpMSB<'a> {
  buffer: Chunks<'a, u8>,
  bits: u64,
  nbits: u32,
}

impl<'a> BitPumpMSB<'a> {
  pub fn new(src: &'a [u8]) -> Self {
    Self {
      buffer: src.chunks(size_of::<u32>()),
      bits: 0,
      nbits: 0,
    }
  }

  /// Refill internal bit buffer - Big-Endian
  ///
  /// For fast refill, we can simply take a whole u32 value out.
  /// For slow refill, there may be 1, 2 or 3 bytes left in buffer. We need
  /// to collect them manually.
  fn refill(&mut self) -> Option<(u32, u32)> {
    let chunk = self.buffer.next()?;
    if chunk.len() == 4 {
      // Fast refill
      let bits: u32 = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
      Some((bits, u32::BITS))
    } else {
      // Slow refill
      Some(chunk.iter().fold((0, 0), |(bits, bit_cnt), x| ((bits << 8) | *x as u32, bit_cnt + 8)))
    }
  }
}

#[derive(Debug, Clone)]
pub struct BitPumpMSB32<'a> {
  buffer: Iter<'a, [u8; 4]>,
  pos: usize,
  bits: u64,
  nbits: u32,
}

impl<'a> BitPumpMSB32<'a> {
  pub fn new(src: &'a [u8]) -> Self {
    Self {
      buffer: src.as_chunks().0.iter(),
      pos: 0,
      bits: 0,
      nbits: 0,
    }
  }

  /// Refill internal bit buffer - MSB32 (bytes read in little-endian order)
  ///
  /// The stream is read in whole 32-bit words. If 1, 2 or 3 bytes are left,
  /// the high bytes of the last word are missing. These would be read first,
  /// so no bit of a partial word can be read.
  fn refill(&mut self) -> Option<u32> {
    self.buffer.next().map(|word| u32::from_le_bytes(*word))
  }

  #[inline(always)]
  pub fn get_pos(&self) -> usize {
    self.pos - ((self.nbits >> 3) as usize)
  }
}

#[derive(Debug, Copy, Clone)]
pub struct BitPumpJPEG<'a> {
  buffer: &'a [u8],
  pos: usize,
  bits: u64,
  nbits: u32,
  zero_fill_bits: u32,
  consumed_zero_fill: bool,
  finished: bool,
}

impl<'a> BitPumpJPEG<'a> {
  pub fn new(src: &'a [u8]) -> Self {
    Self {
      buffer: src,
      pos: 0,
      bits: 0,
      nbits: 0,
      zero_fill_bits: 0,
      consumed_zero_fill: false,
      finished: false,
    }
  }

  /// Diagnose the end of a scan. Callers can warn without discarding decoded pixels.
  pub fn validate_end_of_scan(&self) -> Result<(), String> {
    if self.consumed_zero_fill {
      return Err("Truncated JPEG entropy data at end of scan".to_string());
    }

    let real_bits = self.nbits.saturating_sub(self.zero_fill_bits);
    if real_bits > 7 {
      return Err("Unexpected trailing JPEG entropy data at end of scan".to_string());
    }
    if real_bits != 0 {
      let padding = (self.bits >> self.zero_fill_bits) & ((1_u64 << real_bits) - 1);
      let one_padding = (1_u64 << real_bits) - 1;
      // Older rawler versions emitted zero padding.
      if padding != one_padding && padding != 0 {
        return Err("Invalid JPEG entropy padding at end of scan".to_string());
      }
    }

    if self.pos == self.buffer.len() {
      return Ok(());
    }
    if self.buffer[self.pos] != 0xff {
      return Err(format!("Unexpected trailing JPEG entropy data at byte {}", self.pos));
    }

    let mut marker_pos = self.pos;
    while marker_pos < self.buffer.len() && self.buffer[marker_pos] == 0xff {
      marker_pos += 1;
    }
    if marker_pos == self.buffer.len() {
      return Err("Truncated JPEG marker at end of scan".to_string());
    }

    let marker = self.buffer[marker_pos];
    if marker != 0xd9 {
      return Err(format!("Unexpected JPEG marker 0x{marker:02x} at end of scan"));
    }

    Ok(())
  }

  /// Discard entropy padding and resume after an expected JPEG restart marker.
  pub fn consume_restart_marker(&mut self, expected: u8) -> Result<(), String> {
    if expected > 7 {
      return Err(format!("Invalid JPEG restart marker index: {expected}"));
    }
    if self.pos >= self.buffer.len() || self.buffer[self.pos] != 0xff {
      return Err(format!("Expected JPEG restart marker RST{expected} at byte {}", self.pos));
    }
    if self.consumed_zero_fill {
      return Err(format!("Truncated JPEG entropy data before RST{expected}"));
    }
    // Discard buffered padding regardless of its bit pattern, as reference
    // decoders do. Consuming synthetic bits is still a truncated interval.

    // JPEG permits extra 0xff fill bytes before a marker.
    while self.pos < self.buffer.len() && self.buffer[self.pos] == 0xff {
      self.pos += 1;
    }
    if self.pos >= self.buffer.len() {
      return Err(format!("Truncated JPEG restart marker RST{expected}"));
    }

    let marker = self.buffer[self.pos];
    let expected_marker = 0xd0 + expected;
    if marker != expected_marker {
      return Err(format!(
        "Unexpected JPEG marker 0x{marker:02x}, expected RST{expected} (0x{expected_marker:02x})"
      ));
    }

    self.pos += 1;
    self.bits = 0;
    self.nbits = 0;
    self.zero_fill_bits = 0;
    self.consumed_zero_fill = false;
    self.finished = false;
    Ok(())
  }
}

pub trait BitPump {
  /// Return the next `num` bits without consuming them.
  ///
  /// Fails if the buffer has fewer than `num` bits left or if `num`
  /// is larger than the pump can return at once.
  fn peek_bits(&mut self, num: u32) -> Result<u32, BitPumpError>;

  /// Consume `num` bits. Only valid after a successful
  /// [`Self::peek_bits`] of at least `num` bits.
  fn consume_bits(&mut self, num: u32);

  #[inline(always)]
  fn get_bits(&mut self, num: u32) -> Result<u32, BitPumpError> {
    if num == 0 {
      return Ok(0);
    }

    let val = self.peek_bits(num)?;
    self.consume_bits(num);

    Ok(val)
  }

  #[inline(always)]
  fn peek_ibits(&mut self, num: u32) -> Result<i32, BitPumpError> {
    Ok(self.peek_bits(num)? as i32)
  }

  #[inline(always)]
  fn get_ibits(&mut self, num: u32) -> Result<i32, BitPumpError> {
    Ok(self.get_bits(num)? as i32)
  }

  // Sign extend ibits
  #[inline(always)]
  fn get_ibits_sextended(&mut self, num: u32) -> Result<i32, BitPumpError> {
    let val = self.get_ibits(num)?;
    Ok(val.wrapping_shl(32 - num).wrapping_shr(32 - num))
  }

  /// Count the leading zeroes block-wise in 31 bits
  /// per block and returns the count.
  /// All zero bits are consumed.
  #[inline(always)]
  fn consume_zerobits(&mut self) -> Result<u32, BitPumpError> {
    // Take one bit less because leading_zeros() is undefined
    // when all bits in register are zero.
    const BITS_PER_LOOP: u32 = u32::BITS - 1;
    let mut count = 0;
    // Count-and-skip all the leading `0`s.
    loop {
      let batch: u32 = (self.peek_bits(BITS_PER_LOOP)? << 1) | 0x1;
      let n = batch.leading_zeros();
      self.consume_bits(n);
      count += n;
      if n != BITS_PER_LOOP {
        break;
      }
    }
    Ok(count)
  }
}

impl<'a> BitPump for BitPumpLSB<'a> {
  #[inline(always)]
  fn peek_bits(&mut self, num: u32) -> Result<u32, BitPumpError> {
    check_bits(num, MAX_PEEK_BITS)?;
    if num > self.nbits {
      let (inbits, bit_cnt) = self.refill().ok_or_else(eof)?;
      self.bits = (((inbits as u64) << 32) | (self.bits << (32 - self.nbits))) >> (32 - self.nbits);
      self.nbits += bit_cnt;
      if num > self.nbits {
        return Err(eof());
      }
    }
    Ok((self.bits & (0x0ffffffffu64 >> (32 - num))) as u32)
  }

  #[inline(always)]
  fn consume_bits(&mut self, num: u32) {
    self.nbits -= num;
    self.bits >>= num;
  }
}

impl<'a> BitPump for BitPumpMSB<'a> {
  #[inline(always)]
  fn peek_bits(&mut self, num: u32) -> Result<u32, BitPumpError> {
    check_bits(num, MAX_PEEK_BITS)?;
    if num > self.nbits {
      let (inbits, bit_cnt) = self.refill().ok_or_else(eof)?;
      self.bits = (self.bits << bit_cnt) | inbits as u64;
      self.nbits += bit_cnt;
      if num > self.nbits {
        return Err(eof());
      }
    }
    Ok((self.bits >> (self.nbits - num)) as u32)
  }

  #[inline(always)]
  fn consume_bits(&mut self, num: u32) {
    self.nbits -= num;
    self.bits &= (1 << self.nbits) - 1;
  }
}

impl<'a> BitPump for BitPumpMSB32<'a> {
  #[inline(always)]
  fn peek_bits(&mut self, num: u32) -> Result<u32, BitPumpError> {
    check_bits(num, MAX_PEEK_BITS)?;
    if num > self.nbits {
      let inbits = self.refill().ok_or_else(eof)?;
      self.bits = (self.bits << 32) | inbits as u64;
      self.nbits += u32::BITS;
      self.pos += size_of::<u32>();
    }
    Ok((self.bits >> (self.nbits - num)) as u32)
  }

  #[inline(always)]
  fn consume_bits(&mut self, num: u32) {
    self.nbits -= num;
    self.bits &= (1 << self.nbits) - 1;
  }
}

impl<'a> BitPump for BitPumpJPEG<'a> {
  #[inline(always)]
  fn peek_bits(&mut self, num: u32) -> Result<u32, BitPumpError> {
    check_bits(num, MAX_PEEK_BITS)?;
    if num > self.nbits && !self.finished {
      if (self.buffer.len() >= 4)
        && self.pos < self.buffer.len() - 4
        && self.buffer[self.pos + 0] != 0xff
        && self.buffer[self.pos + 1] != 0xff
        && self.buffer[self.pos + 2] != 0xff
        && self.buffer[self.pos + 3] != 0xff
      {
        let inbits: u64 = BEu32(self.buffer, self.pos) as u64;
        self.bits = (self.bits << 32) | inbits;
        self.pos += 4;
        self.nbits += 32;
      } else {
        // Read 32 bits the hard way
        let mut read_bytes = 0;
        while read_bytes < 4 && !self.finished {
          if self.pos >= self.buffer.len() {
            self.finished = true;
            break;
          }

          let nextbyte = self.buffer[self.pos];
          let byte = if nextbyte != 0xff {
            nextbyte
          } else if self.pos + 1 < self.buffer.len() && self.buffer[self.pos + 1] == 0x00 {
            self.pos += 1; // Skip the extra byte used to mark 255
            nextbyte
          } else {
            // Leave the marker untouched so a restart-aware decoder can
            // validate and consume it after the current MCU.
            self.finished = true;
            break;
          };
          self.bits = (self.bits << 8) | (byte as u64);
          self.pos += 1;
          self.nbits += 8;
          read_bytes += 1;
        }
      }
    }
    if num > self.nbits && self.finished {
      // Huffman lookup may read ahead of the marker. Fill with zero bits
      // instead of failing, but remember these synthetic bits so the
      // restart-aware decoder can reject a segment that actually consumes them.
      self.bits <<= 32;
      self.nbits += 32;
      self.zero_fill_bits += 32;
    }

    Ok((self.bits >> (self.nbits - num)) as u32)
  }

  #[inline(always)]
  fn consume_bits(&mut self, num: u32) {
    debug_assert!(num <= self.nbits);
    let real_bits = self.nbits.saturating_sub(self.zero_fill_bits);
    let consumed_zero_fill = num.saturating_sub(real_bits);
    if consumed_zero_fill != 0 {
      self.consumed_zero_fill = true;
      self.zero_fill_bits -= consumed_zero_fill;
    }
    self.nbits -= num;
    self.bits &= (1 << self.nbits) - 1;
  }
}

#[derive(Debug, Copy, Clone)]
pub struct ByteStream<'a> {
  buffer: &'a [u8],
  pos: usize,
  endian: Endian,
}

impl<'a> ByteStream<'a> {
  pub fn new(src: &'a [u8], endian: Endian) -> Self {
    Self { buffer: src, pos: 0, endian }
  }

  #[inline(always)]
  pub fn remaining_bytes(&self) -> usize {
    self.buffer.len() - self.pos
  }

  #[inline(always)]
  pub fn get_pos(&self) -> usize {
    self.pos
  }

  #[inline(always)]
  pub fn peek_u8(&self) -> u8 {
    self.buffer[self.pos]
  }
  #[inline(always)]
  pub fn get_u8(&mut self) -> u8 {
    let val = self.peek_u8();
    self.pos += 1;
    val
  }

  #[inline(always)]
  pub fn peek_i8(&self) -> i8 {
    self.buffer[self.pos] as i8
  }
  #[inline(always)]
  pub fn get_i8(&mut self) -> i8 {
    let val = self.peek_i8();
    self.pos += 1;
    val
  }

  #[inline(always)]
  pub fn peek_u16(&self) -> u16 {
    self.endian.read_u16(self.buffer, self.pos)
  }
  #[inline(always)]
  pub fn get_u16(&mut self) -> u16 {
    let val = self.peek_u16();
    self.pos += 2;
    val
  }

  #[inline(always)]
  pub fn peek_i16(&self) -> i16 {
    self.endian.read_i16(self.buffer, self.pos)
  }
  #[inline(always)]
  pub fn get_i16(&mut self) -> i16 {
    let val = self.peek_i16();
    self.pos += 2;
    val
  }

  #[inline(always)]
  pub fn peek_u32(&self) -> u32 {
    self.endian.read_u32(self.buffer, self.pos)
  }

  #[inline(always)]
  pub fn get_u32(&mut self) -> u32 {
    let val = self.peek_u32();
    self.pos += 4;
    val
  }

  #[inline(always)]
  pub fn get_bytes(&mut self, n: usize) -> Vec<u8> {
    let mut val = Vec::with_capacity(n);
    val.extend_from_slice(&self.buffer[self.pos..self.pos + n]);
    self.pos += n;
    val
  }

  //  #[inline(always)]
  //  pub fn peek_u32(&self) -> u32 { self.endian.ru32(self.buffer, self.pos) }
  //  #[inline(always)]
  //  pub fn get_u32(&mut self) -> u32 {
  //    let val = self.peek_u32();
  //    self.pos += 4;
  //    val
  //  }

  #[inline(always)]
  pub fn consume_bytes(&mut self, num: usize) {
    self.pos += num
  }

  #[inline(always)]
  pub fn skip_to_marker(&mut self) -> Result<usize, String> {
    let mut skip_count = 0;
    while !(self.buffer[self.pos] == 0xFF && self.buffer[self.pos + 1] != 0 && self.buffer[self.pos + 1] != 0xFF) {
      self.pos += 1;
      skip_count += 1;
      if self.pos >= self.buffer.len() {
        return Err("No marker found inside rest of buffer".to_string());
      }
    }
    self.pos += 1; // Make the next byte the marker
    Ok(skip_count + 1)
  }
}

/// This pump is for bitstreams where values are stored in LSB bit order.
/// During refill, bits are converted from LSB to MSB so peaking
/// is done by reading in MSB mode.
///
/// Input bitstream is:     1011 0101 0010 1110...
/// Output for peek(10) is: 1010 1101 01
#[derive(Debug, Copy, Clone)]
pub struct BitPumpReverseBitsMSB<'a> {
  buffer: &'a [u8],
  pos: usize,
  bits: u64,
  nbits: u32,
}

impl<'a> BitPumpReverseBitsMSB<'a> {
  pub fn new(src: &'a [u8]) -> Self {
    Self {
      buffer: src,
      pos: 0,
      bits: 0,
      nbits: 0,
    }
  }
}

impl<'a> BitPump for BitPumpReverseBitsMSB<'a> {
  #[inline(always)]
  fn peek_bits(&mut self, num: u32) -> Result<u32, BitPumpError> {
    check_bits(num, MAX_PEEK_BITS)?;
    if num > self.nbits {
      if let Some(&[b0, b1, b2, b3]) = self.buffer.get(self.pos..self.pos + 4) {
        // Fast refill
        let raw = [b0.reverse_bits(), b1.reverse_bits(), b2.reverse_bits(), b3.reverse_bits()];
        let inbits: u64 = u32::from_be_bytes(raw) as u64;
        self.bits = (self.bits << 32) | inbits;
        self.pos += 4;
        self.nbits += 32;
      } else {
        // Slow refill, there may be 1, 2 or 3 bytes left in buffer
        for byte in &self.buffer[self.pos..] {
          self.bits = (self.bits << 8) | byte.reverse_bits() as u64;
          self.nbits += 8;
        }
        self.pos = self.buffer.len();
        if num > self.nbits {
          return Err(eof());
        }
      }
    }
    Ok((self.bits >> (self.nbits - num)) as u32)
  }

  #[inline(always)]
  fn consume_bits(&mut self, num: u32) {
    self.nbits -= num;
    self.bits &= (1 << self.nbits) - 1;
  }
}
