// SPDX-License-Identifier: LGPL-2.1

use std::io::{Error, ErrorKind, Result};

/// Big-endian bit reader for CRX bitstreams
///
/// Reading past the end of the stream is an error, as CRX streams
/// come straight from the file and may be truncated.
pub(super) struct BitPump<'a> {
  /// Bytes not yet moved into `bits`
  data: &'a [u8],
  /// Bit buffer, the next bit in the stream is the most significant bit
  bits: u64,
  /// Number of valid bits in `bits`, always below 64
  nbits: u32,
}

impl<'a> BitPump<'a> {
  pub(super) fn new(data: &'a [u8]) -> Self {
    Self { data, bits: 0, nbits: 0 }
  }

  /// Fill the bit buffer so it holds at least 56 bits,
  /// or all remaining bits at the end of the stream.
  #[inline(always)]
  fn refill(&mut self) {
    if let Some((word, _)) = self.data.split_first_chunk::<8>() {
      // Only whole bytes are accounted for. Bits below `nbits` are the
      // same stream bits as in the next refill, so OR-ing them is safe.
      self.bits |= u64::from_be_bytes(*word) >> self.nbits;
      let bytes = (63 - self.nbits) >> 3;
      self.data = &self.data[bytes as usize..];
      self.nbits += bytes * 8;
    } else {
      self.refill_tail();
    }
  }

  /// Refill from the last bytes of the stream
  #[cold]
  fn refill_tail(&mut self) {
    while let Some((byte, rest)) = self.data.split_first()
      && self.nbits <= 55
    {
      self.bits |= (*byte as u64) << (56 - self.nbits);
      self.data = rest;
      self.nbits += 8;
    }
  }

  #[inline(always)]
  fn consume(&mut self, num: u32) {
    debug_assert!(num <= self.nbits);
    self.bits <<= num;
    self.nbits -= num;
  }

  /// Read the given number of bits, up to 32
  #[inline(always)]
  pub(super) fn read_bits(&mut self, num: u32) -> Result<u32> {
    if num > self.nbits.min(u32::BITS) {
      self.refill();
      if num > self.nbits.min(u32::BITS) {
        return Err(Self::read_error(num));
      }
    }
    // Two shifts because num may be zero
    let value = (self.bits >> 1) >> (63 - num);
    self.consume(num);
    Ok(value as u32)
  }

  /// Return the number of 0-bits up to the next 1-bit.
  /// All 0-bits and the 1-bit are consumed.
  #[inline(always)]
  pub(super) fn read_zeros(&mut self) -> Result<u32> {
    let zeros = self.bits.leading_zeros();
    if zeros < self.nbits {
      self.consume(zeros + 1);
      Ok(zeros)
    } else {
      self.read_zeros_refill()
    }
  }

  /// Handle a run of 0-bits which reaches the end of the bit buffer
  #[cold]
  fn read_zeros_refill(&mut self) -> Result<u32> {
    let mut count = 0_u32;
    loop {
      count = count.saturating_add(self.nbits);
      self.consume(self.nbits);
      self.refill();
      if self.nbits == 0 {
        return Err(Error::from(ErrorKind::UnexpectedEof));
      }
      let zeros = self.bits.leading_zeros();
      if zeros < self.nbits {
        self.consume(zeros + 1);
        return Ok(count.saturating_add(zeros));
      }
    }
  }

  #[cold]
  fn read_error(num: u32) -> Error {
    if num > u32::BITS {
      Error::new(ErrorKind::InvalidInput, "excessive bits for type read")
    } else {
      Error::from(ErrorKind::UnexpectedEof)
    }
  }
}

#[cfg(test)]
mod tests {
  use super::BitPump;
  use bitstream_io::{BigEndian, BitRead, BitReader};
  use std::io::Cursor;

  /// Test data with a given chance of 1-bits, to get short and long runs of 0-bits
  fn test_data(len: usize, mut state: u64, one_bits_per_256: u64) -> Vec<u8> {
    let mut next = move || {
      state ^= state << 13;
      state ^= state >> 7;
      state ^= state << 17;
      state
    };
    (0..len)
      .map(|_| (0..8).fold(0_u8, |byte, _| (byte << 1) | ((next() & 0xff) < one_bits_per_256) as u8))
      .collect()
  }

  /// Same results as bitstream_io for any mix of reads, including at the end of the stream
  #[test]
  fn matches_bitstream_io() {
    for len in (0..32).chain([100, 4097]) {
      for (seed, density) in [
        (0x9e3779b97f4a7c15, 128),
        (0xd1b54a32d192ed03, 16),
        (0xa0761d6478bd642f, 1),
        (0x8ebc6af09c88c6e3, 0),
      ] {
        let data = test_data(len, seed, density);
        let mut reader = BitReader::endian(Cursor::new(data.as_slice()), BigEndian);
        let mut pump = BitPump::new(&data);
        let mut op = seed;
        loop {
          op = op.wrapping_mul(0x2545f4914f6cdd1d).wrapping_add(1);
          let num = ((op >> 40) % 33) as u32;
          let (expected, actual) = if op >> 63 == 0 {
            (reader.read_unary::<1>(), pump.read_zeros())
          } else {
            (reader.read_var::<u32>(num), pump.read_bits(num))
          };
          match (expected, actual) {
            (Ok(expected), Ok(actual)) => assert_eq!(expected, actual, "len {len}, density {density}"),
            (Err(expected), Err(actual)) => {
              assert_eq!(expected.kind(), actual.kind(), "len {len}, density {density}");
              break;
            }
            (expected, actual) => panic!("len {len}, density {density}: {expected:?} != {actual:?}"),
          }
        }
      }
    }
  }

  #[test]
  fn too_many_bits_is_an_error() {
    let data = [0xff; 16];
    assert!(BitPump::new(&data).read_bits(33).is_err());
    assert!(BitPump::new(&data).read_bits(57).is_err());
    // Also when the bit buffer already holds that many bits
    let mut pump = BitPump::new(&data);
    assert_eq!(pump.read_bits(1).ok(), Some(1));
    assert!(pump.read_bits(33).is_err());
    assert_eq!(BitPump::new(&data).read_bits(32).ok(), Some(u32::MAX));
    assert_eq!(BitPump::new(&data).read_bits(0).ok(), Some(0));
  }
}
