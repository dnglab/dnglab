use crate::bits::*;

#[derive(Debug, Copy, Clone)]
pub struct BitPumpLSB<'a> {
  buffer: &'a [u8],
  pos: usize,
  bits: u64,
  nbits: u32,
}

impl<'a> BitPumpLSB<'a> {
  pub fn new(src: &'a [u8]) -> BitPumpLSB {
    BitPumpLSB {
      buffer: src,
      pos: 0,
      bits: 0,
      nbits: 0,
    }
  }
}

#[derive(Debug, Copy, Clone)]
pub struct BitPumpMSB<'a> {
  buffer: &'a [u8],
  pos: usize,
  bits: u64,
  nbits: u32,
}

impl<'a> BitPumpMSB<'a> {
  pub fn new(src: &'a [u8]) -> BitPumpMSB {
    BitPumpMSB {
      buffer: src,
      pos: 0,
      bits: 0,
      nbits: 0,
    }
  }
}

#[derive(Debug, Copy, Clone)]
pub struct BitPumpMSB32<'a> {
  buffer: &'a [u8],
  pos: usize,
  bits: u64,
  nbits: u32,
}

impl<'a> BitPumpMSB32<'a> {
  pub fn new(src: &'a [u8]) -> BitPumpMSB32 {
    BitPumpMSB32 {
      buffer: src,
      pos: 0,
      bits: 0,
      nbits: 0,
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
  finished: bool,
}

impl<'a> BitPumpJPEG<'a> {
  pub fn new(src: &'a [u8]) -> BitPumpJPEG {
    BitPumpJPEG {
      buffer: src,
      pos: 0,
      bits: 0,
      nbits: 0,
      finished: false,
    }
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
      let inbits: u64 = LEu32(self.buffer, self.pos) as u64;
      self.bits = ((inbits << 32) | (self.bits << (32 - self.nbits))) >> (32 - self.nbits);
      self.pos += 4;
      self.nbits += 32;
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
      let inbits: u64 = BEu32(self.buffer, self.pos) as u64;
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

impl<'a> BitPump for BitPumpMSB32<'a> {
  #[inline(always)]
  fn peek_bits(&mut self, num: u32) -> u32 {
    if num > self.nbits {
      let inbits: u64 = LEu32(self.buffer, self.pos) as u64;
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
          let byte = {
            if self.pos >= self.buffer.len() {
              self.finished = true;
              0
            } else {
              let nextbyte = self.buffer[self.pos];
              if nextbyte != 0xff {
                nextbyte
              } else if self.buffer[self.pos + 1] == 0x00 {
                self.pos += 1; // Skip the extra byte used to mark 255
                nextbyte
              } else {
                self.finished = true;
                0
              }
            }
          };
          self.bits = (self.bits << 8) | (byte as u64);
          self.pos += 1;
          self.nbits += 8;
          read_bytes += 1;
        }
      }
    }
    if num > self.nbits && self.finished {
      // Stuff with zeroes to not fail to read
      self.bits <<= 32;
      self.nbits += 32;
    }

    (self.bits >> (self.nbits - num)) as u32
  }

  #[inline(always)]
  fn consume_bits(&mut self, num: u32) {
    debug_assert!(num <= self.nbits);
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
  pub fn new(src: &'a [u8], endian: Endian) -> ByteStream {
    ByteStream { buffer: src, pos: 0, endian }
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
