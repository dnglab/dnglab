use std::slice::Chunks;

use crate::bits::*;

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
  fn refill(&mut self) -> (u32, u32) {
    if let Some(chunk) = self.buffer.next() {
      if chunk.len() == 4 {
        // Fast refill
        let bits: u32 = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        (bits, u32::BITS)
      } else {
        // Slow refill
        chunk
          .into_iter()
          .rev()
          .fold((0, 0), |(bits, bit_cnt), x| ((bits << 8) | *x as u32, bit_cnt + 8))
      }
    } else {
      panic!("Can't refill bitpump, buffer exhausted");
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
  fn refill(&mut self) -> (u32, u32) {
    if let Some(chunk) = self.buffer.next() {
      if chunk.len() == 4 {
        // Fast refill
        let bits: u32 = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        (bits, u32::BITS)
      } else {
        // Slow refill
        chunk.into_iter().fold((0, 0), |(bits, bit_cnt), x| ((bits << 8) | *x as u32, bit_cnt + 8))
      }
    } else {
      panic!("Can't refill bitpump, buffer exhausted");
    }
  }
}

#[derive(Debug, Clone)]
pub struct BitPumpMSB32<'a> {
  buffer: Chunks<'a, u8>,
  pos: usize,
  bits: u64,
  nbits: u32,
}

impl<'a> BitPumpMSB32<'a> {
  pub fn new(src: &'a [u8]) -> Self {
    Self {
      buffer: src.chunks(size_of::<u32>()),
      pos: 0,
      bits: 0,
      nbits: 0,
    }
  }

  /// Refill internal bit buffer - MSB32 (bytes read in little-endian order)
  ///
  /// For fast refill, we can simply take a whole u32 value out.
  /// For slow refill, there may be 1, 2 or 3 bytes left in buffer. We need
  /// to collect them manually.
  fn refill(&mut self) -> (u32, u32) {
    if let Some(chunk) = self.buffer.next() {
      if chunk.len() == 4 {
        // Fast refill
        let bits: u32 = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        (bits, u32::BITS)
      } else {
        // Slow refill
        chunk
          .into_iter()
          .rev()
          .fold((0, 0), |(bits, bit_cnt), x| ((bits << 8) | *x as u32, bit_cnt + 8))
      }
    } else {
      panic!("Can't refill bitpump, buffer exhausted");
    }
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

  fn validate_entropy_padding(&self, context: &str, allow_legacy_zero_padding: bool, allow_trailing_entropy: bool) -> Result<(), String> {
    if self.consumed_zero_fill {
      return Err(format!("Truncated JPEG entropy data {context}"));
    }

    let real_bits = self.nbits.saturating_sub(self.zero_fill_bits);
    if real_bits > 7 && !allow_trailing_entropy {
      return Err(format!("Unexpected trailing JPEG entropy data {context}"));
    }
    if real_bits <= 7 && real_bits != 0 {
      let padding = (self.bits >> self.zero_fill_bits) & ((1_u64 << real_bits) - 1);
      let one_padding = (1_u64 << real_bits) - 1;
      if padding != one_padding && !(allow_legacy_zero_padding && padding == 0) {
        return Err(format!("Invalid JPEG entropy padding {context}"));
      }
    }

    Ok(())
  }

  /// Validate the padding after the final decoded MCU in the scan.
  pub fn validate_end_of_scan(&self) -> Result<(), String> {
    self.validate_end_of_scan_inner(false)
  }

  /// Validate the final scan while accepting extra entropy buffered before
  /// EOI by a small set of legacy camera streams.
  pub fn validate_end_of_scan_with_legacy_trailing_entropy(&self) -> Result<(), String> {
    self.validate_end_of_scan_inner(true)
  }

  fn validate_end_of_scan_inner(&self, allow_legacy_trailing_entropy: bool) -> Result<(), String> {
    let has_eoi = if self.pos < self.buffer.len() && self.buffer[self.pos] == 0xff {
      let mut marker_pos = self.pos;
      while marker_pos < self.buffer.len() && self.buffer[marker_pos] == 0xff {
        marker_pos += 1;
      }
      marker_pos < self.buffer.len() && self.buffer[marker_pos] == 0xd9
    } else {
      false
    };

    // Older rawler versions emitted zero padding at the end of a scan.
    // Some legacy camera files also leave extra entropy bits immediately
    // before EOI. Continue to decode those files, but keep padding strict when
    // no EOI is present and before restart markers, where extra data would make
    // the restart cadence ambiguous.
    self.validate_entropy_padding("at end of scan", true, allow_legacy_trailing_entropy && has_eoi)?;

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
    self.validate_entropy_padding(&format!("before RST{expected}"), false, false)?;

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
  fn peek_bits(&mut self, num: u32) -> u32;
  fn consume_bits(&mut self, num: u32);

  #[inline(always)]
  fn get_bits(&mut self, num: u32) -> u32 {
    if num == 0 {
      return 0;
    }

    let val = self.peek_bits(num);
    self.consume_bits(num);

    val
  }

  #[inline(always)]
  fn peek_ibits(&mut self, num: u32) -> i32 {
    self.peek_bits(num) as i32
  }

  #[inline(always)]
  fn get_ibits(&mut self, num: u32) -> i32 {
    self.get_bits(num) as i32
  }

  // Sign extend ibits
  #[inline(always)]
  fn get_ibits_sextended(&mut self, num: u32) -> i32 {
    let val = self.get_ibits(num);
    val.wrapping_shl(32 - num).wrapping_shr(32 - num)
  }

  /// Count the leading zeroes block-wise in 31 bits
  /// per block and returns the count.
  /// All zero bits are consumed.
  #[inline(always)]
  fn consume_zerobits(&mut self) -> u32 {
    // Take one bit less because leading_zeros() is undefined
    // when all bits in register are zero.
    const BITS_PER_LOOP: u32 = u32::BITS - 1;
    let mut count = 0;
    // Count-and-skip all the leading `0`s.
    loop {
      let batch: u32 = (self.peek_bits(BITS_PER_LOOP) << 1) | 0x1;
      let n = batch.leading_zeros();
      self.consume_bits(n);
      count += n;
      if n != BITS_PER_LOOP {
        break;
      }
    }
    count
  }
}

impl<'a> BitPump for BitPumpLSB<'a> {
  #[inline(always)]
  fn peek_bits(&mut self, num: u32) -> u32 {
    if num > self.nbits {
      let (inbits, bit_cnt) = self.refill();
      self.bits = (((inbits as u64) << 32) | (self.bits << (32 - self.nbits))) >> (32 - self.nbits);
      self.nbits += bit_cnt;
    }
    (self.bits & (0x0ffffffffu64 >> (32 - num))) as u32
  }

  #[inline(always)]
  fn consume_bits(&mut self, num: u32) {
    self.nbits -= num;
    self.bits >>= num;
  }
}

impl<'a> BitPump for BitPumpMSB<'a> {
  #[inline(always)]
  fn peek_bits(&mut self, num: u32) -> u32 {
    if num > self.nbits {
      let (inbits, bit_cnt) = self.refill();
      self.bits = (self.bits << bit_cnt) | inbits as u64;
      self.nbits += bit_cnt;
    }
    (self.bits >> (self.nbits - num)) as u32
  }

  #[inline(always)]
  fn consume_bits(&mut self, num: u32) {
    self.nbits -= num;
    self.bits &= (1 << self.nbits) - 1;
  }
}

impl<'a> BitPump for BitPumpMSB32<'a> {
  #[inline(always)]
  fn peek_bits(&mut self, num: u32) -> u32 {
    if num > self.nbits {
      let (inbits, bit_cnt) = self.refill();
      self.bits = (self.bits << 32) | inbits as u64;
      self.nbits += bit_cnt;
      self.pos += bit_cnt as usize / 8;
    }
    (self.bits >> (self.nbits - num)) as u32
  }

  #[inline(always)]
  fn consume_bits(&mut self, num: u32) {
    self.nbits -= num;
    self.bits &= (1 << self.nbits) - 1;
  }
}

impl<'a> BitPump for BitPumpJPEG<'a> {
  #[inline(always)]
  fn peek_bits(&mut self, num: u32) -> u32 {
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
      // Huffman lookup may read ahead of the marker. Preserve the fast,
      // infallible BitPump interface, but remember these synthetic bits so the
      // restart-aware decoder can reject a segment that actually consumes them.
      self.bits <<= 32;
      self.nbits += 32;
      self.zero_fill_bits += 32;
    }

    (self.bits >> (self.nbits - num)) as u32
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
  fn peek_bits(&mut self, num: u32) -> u32 {
    debug_assert!(num <= 32);
    if num > self.nbits {
      let mut raw: [u8; 4] = BEu32(self.buffer, self.pos).to_ne_bytes();
      raw[0] = raw[0].reverse_bits();
      raw[1] = raw[1].reverse_bits();
      raw[2] = raw[2].reverse_bits();
      raw[3] = raw[3].reverse_bits();
      let inbits: u64 = u32::from_ne_bytes(raw) as u64;
      self.bits = (self.bits << 32) | inbits;
      self.pos += 4;
      self.nbits += 32;
    }
    (self.bits >> (self.nbits - num)) as u32
  }

  #[inline(always)]
  fn consume_bits(&mut self, num: u32) {
    self.nbits -= num;
    self.bits &= (1 << self.nbits) - 1;
  }
}
