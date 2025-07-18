// SPDX-License-Identifier: LGPL-2.1
// Copyright 2025 Daniel Vogelbacher <daniel@chaospixel.com>
//
// Floating-Point trait was ported from rawspeed:
// https://github.com/darktable-org/rawspeed/blob/6815b8ca1469234768edc9ddce8b7efb419381bf/src/librawspeed/common/FloatingPoint.h
// Copyright (C) 2017 Vasily Khoruzhick
// Copyright (C) 2020 Roman Lebedev

use std::iter::repeat;

use byteorder::{BigEndian, ByteOrder, LittleEndian};
use serde::{Deserialize, Serialize};

#[inline(always)]
pub fn clampbits(val: i32, bits: u32) -> u16 {
  let max = (1 << bits) - 1;
  if val < 0 {
    0
  } else if val > max {
    max as u16
  } else {
    val as u16
  }
}

pub fn clamp(val: i32, min: i32, max: i32) -> i32 {
  let mut res = val;
  if res < min {
    res = min;
  }
  if res > max {
    res = max;
  }
  res
}

/// Calculate the required bits to encode as many states.
pub fn log2ceil(mut states: usize) -> usize {
  let mut bits = 0;
  if states > 0 {
    states -= 1;
    loop {
      states >>= 1;
      bits += 1;
      if states == 0 {
        break;
      }
    }
  }
  bits
}

#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
pub enum Endian {
  Big,
  Little,
}

impl Default for Endian {
  fn default() -> Self {
    Self::Little
  }
}

impl Endian {
  #[inline]
  pub fn big(&self) -> bool {
    matches!(*self, Self::Big)
  }
  #[inline]
  pub fn little(&self) -> bool {
    matches!(*self, Self::Little)
  }

  #[inline]
  pub fn read_u8(&self, buf: &[u8], offset: usize) -> u8 {
    buf[offset]
  }

  #[inline]
  pub fn read_i8(&self, buf: &[u8], offset: usize) -> i8 {
    buf[offset] as i8
  }

  #[inline]
  pub fn read_u16(&self, buf: &[u8], offset: usize) -> u16 {
    match *self {
      Self::Big => BigEndian::read_u16(&buf[offset..]),
      Self::Little => LittleEndian::read_u16(&buf[offset..]),
    }
  }

  #[inline]
  pub fn read_i16(&self, buf: &[u8], offset: usize) -> i16 {
    match *self {
      Self::Big => BigEndian::read_i16(&buf[offset..]),
      Self::Little => LittleEndian::read_i16(&buf[offset..]),
    }
  }

  #[inline]
  pub fn read_u32(&self, buf: &[u8], offset: usize) -> u32 {
    match *self {
      Self::Big => BigEndian::read_u32(&buf[offset..]),
      Self::Little => LittleEndian::read_u32(&buf[offset..]),
    }
  }

  #[inline]
  pub fn read_i32(&self, buf: &[u8], offset: usize) -> i32 {
    match *self {
      Self::Big => BigEndian::read_i32(&buf[offset..]),
      Self::Little => LittleEndian::read_i32(&buf[offset..]),
    }
  }

  #[inline]
  pub fn write_u16(&self, buf: &mut [u8], n: u16) {
    match *self {
      Self::Big => BigEndian::write_u16(buf, n),
      Self::Little => LittleEndian::write_u16(buf, n),
    }
  }
}

#[allow(non_snake_case)]
#[inline]
pub fn BEi32(buf: &[u8], pos: usize) -> i32 {
  BigEndian::read_i32(&buf[pos..pos + 4])
}

#[allow(non_snake_case)]
#[inline]
pub fn LEi32(buf: &[u8], pos: usize) -> i32 {
  LittleEndian::read_i32(&buf[pos..pos + 4])
}

#[allow(non_snake_case)]
#[inline]
pub fn BEu32(buf: &[u8], pos: usize) -> u32 {
  BigEndian::read_u32(&buf[pos..pos + 4])
}

#[allow(non_snake_case)]
#[inline]
pub fn LEu32(buf: &[u8], pos: usize) -> u32 {
  LittleEndian::read_u32(&buf[pos..pos + 4])
}

#[allow(non_snake_case)]
#[inline]
pub fn LEf32(buf: &[u8], pos: usize) -> f32 {
  LittleEndian::read_f32(&buf[pos..pos + 4])
}

#[allow(non_snake_case)]
#[inline]
pub fn LEf24(buf: &[u8], pos: usize) -> f32 {
  let fp24: u32 = u32::from_le_bytes([buf[pos + 0], buf[pos + 1], buf[pos + 2], 0]);
  f32::from_bits(extend_binary_floating_point::<Binary24, Binary32>(fp24))
}

#[allow(non_snake_case)]
#[inline]
pub fn BEf24(buf: &[u8], pos: usize) -> f32 {
  let fp24: u32 = u32::from_be_bytes([0, buf[pos + 0], buf[pos + 1], buf[pos + 2]]);
  f32::from_bits(extend_binary_floating_point::<Binary24, Binary32>(fp24))
}

#[allow(non_snake_case)]
#[inline]
pub fn LEf16(buf: &[u8], pos: usize) -> f32 {
  let fp16: u16 = u16::from_le_bytes([buf[pos + 0], buf[pos + 1]]);
  f32::from_bits(extend_binary_floating_point::<Binary16, Binary32>(fp16 as u32))
}

#[allow(non_snake_case)]
#[inline]
pub fn BEf16(buf: &[u8], pos: usize) -> f32 {
  let fp16: u16 = u16::from_be_bytes([buf[pos + 0], buf[pos + 1]]);
  f32::from_bits(extend_binary_floating_point::<Binary16, Binary32>(fp16 as u32))
}

#[allow(non_snake_case)]
#[inline]
pub fn BEf32(buf: &[u8], pos: usize) -> f32 {
  BigEndian::read_f32(&buf[pos..pos + 4])
}

#[allow(non_snake_case)]
#[inline]
pub fn BEu16(buf: &[u8], pos: usize) -> u16 {
  BigEndian::read_u16(&buf[pos..pos + 2])
}

#[allow(non_snake_case)]
#[inline]
pub fn LEu16(buf: &[u8], pos: usize) -> u16 {
  LittleEndian::read_u16(&buf[pos..pos + 2])
}

#[derive(Debug, Clone)]
pub struct LookupTable {
  table: Vec<(u16, u16, u16)>,
}

impl LookupTable {
  pub fn new(table: &[u16]) -> LookupTable {
    let mut tbl = vec![(0, 0, 0); table.len()];
    for i in 0..table.len() {
      let center = table[i];
      let lower = if i > 0 { table[i - 1] } else { center };
      let upper = if i < (table.len() - 1) { table[i + 1] } else { center };
      let base = if center == 0 { 0 } else { center - ((upper - lower + 2) / 4) };
      let delta = upper - lower;
      tbl[i] = (center, base, delta);
    }
    LookupTable { table: tbl }
  }

  pub fn new_with_bits(table: &[u16], bits: u32) -> LookupTable {
    assert!(!table.is_empty());
    if table.len() >= 1 << bits {
      Self::new(table)
    } else {
      let mut expanded = Vec::with_capacity(1 << bits);
      expanded.extend_from_slice(table);
      expanded.extend(repeat(table.last().expect("Need one element")).take((1 << bits) - table.len()));
      Self::new(&expanded)
    }
  }

  //  pub fn lookup(&self, value: u16) -> u16 {
  //    let (val, _, _) = self.table[value as usize];
  //    val
  //  }

  #[inline(always)]
  pub fn dither(&self, value: u16, rand: &mut u32) -> u16 {
    let (_, sbase, sdelta) = self.table[value as usize];
    let base = sbase as u32;
    let delta = sdelta as u32;
    let pixel = base + ((delta * (*rand & 2047) + 1024) >> 12);
    *rand = 15700 * (*rand & 65535) + (*rand >> 16);
    pixel as u16
  }
}

/// A trait defining compile-time parameters for a floating-point representation.
///
/// This trait provides associated constants that describe the bit layout of a floating-point type,
/// including the total storage width, the number of bits for the fraction (mantissa), and the exponent.
/// It also provides derived constants for the sign bit, precision, exponent bias, and bit positions.
///
/// # Associated Constants
/// - `STORAGE_WIDTH`: Total number of bits used to store the floating-point value.
/// - `FRACTION_WIDTH`: Number of bits used for the fraction (mantissa).
/// - `EXPONENT_WIDTH`: Number of bits used for the exponent.
/// - `STORAGE_BYTES`: Number of bytes required (rounded up) for storage.
/// - `SIGN_BITS`: Number of bits used for the sign (always 1).
/// - `PRECISION`: Number of significant bits in the mantissa (fraction width + 1 for the implicit bit).
/// - `EXPONENT_MAX`: Maximum value of the exponent (before bias).
/// - `BIAS`: Bias value applied to the exponent.
/// - `FRACTION_POS`: Bit position where the fraction starts (always 0).
/// - `EXPONENT_POS`: Bit position where the exponent starts.
/// - `SIGN_BIT_POS`: Bit position of the sign bit (highest bit).
pub(crate) trait FloatingPointParameters {
  const STORAGE_WIDTH: usize;
  const FRACTION_WIDTH: usize;
  const EXPONENT_WIDTH: usize;

  const STORAGE_BYTES: usize = Self::STORAGE_WIDTH.div_ceil(u8::BITS as usize);

  const SIGN_BITS: usize = 1;
  const PRECISION: usize = Self::FRACTION_WIDTH + 1;
  const EXPONENT_MAX: usize = (1 << (Self::EXPONENT_WIDTH - 1)) - 1;
  const BIAS: i32 = Self::EXPONENT_MAX as i32;
  const FRACTION_POS: usize = 0; // FractionPos is always 0.
  const EXPONENT_POS: usize = Self::FRACTION_WIDTH;
  const SIGN_BIT_POS: usize = Self::STORAGE_WIDTH - 1;
}

/// A generic struct representing a binary number with customizable storage width, fraction width, and exponent width.
///
/// # Type Parameters
/// - `STORAGE_WITH`: The total number of bits used for storage.
/// - `FRACTION_WIDTH`: The number of bits allocated for the fractional part.
/// - `EXPONENT_WIDTH`: The number of bits allocated for the exponent part.
///
/// This struct can be used to represent custom floating-point or fixed-point binary formats.
pub(crate) struct BinaryN<const STORAGE_WITH: usize, const FRACTION_WIDTH: usize, const EXPONENT_WIDTH: usize> {}

impl<const STORAGE_WITH: usize, const FRACTION_WIDTH: usize, const EXPONENT_WIDTH: usize> FloatingPointParameters
  for BinaryN<STORAGE_WITH, FRACTION_WIDTH, EXPONENT_WIDTH>
{
  const STORAGE_WIDTH: usize = STORAGE_WITH;

  const FRACTION_WIDTH: usize = FRACTION_WIDTH;

  const EXPONENT_WIDTH: usize = EXPONENT_WIDTH;

  const SIGN_BITS: usize = 1;
}

pub(crate) type Binary16 = BinaryN<16, 10, 5>;
pub(crate) type Binary24 = BinaryN<24, 16, 7>;
pub(crate) type Binary32 = BinaryN<32, 23, 8>;

pub(crate) fn extend_binary_floating_point<NARROW: FloatingPointParameters, WIDE: FloatingPointParameters>(value: u32) -> u32 {
  let sign = (value >> NARROW::SIGN_BIT_POS) & 1;
  let narrow_exponent = (value >> NARROW::EXPONENT_POS) & ((1 << NARROW::EXPONENT_WIDTH) - 1);
  let narrow_fraction = value & ((1 << NARROW::FRACTION_WIDTH) - 1);

  // Normalized or zero
  let mut wide_exponent = ((narrow_exponent as i32) - NARROW::BIAS + WIDE::BIAS) as u32;
  let mut wide_fraction = narrow_fraction << (WIDE::FRACTION_WIDTH - NARROW::FRACTION_WIDTH);

  if narrow_exponent == ((1 << NARROW::EXPONENT_WIDTH) - 1) {
    // Infinity or NaN
    wide_exponent = (1 << WIDE::EXPONENT_WIDTH) - 1;
    // Narrow fraction is kept/widened!
  } else if narrow_exponent == 0 {
    if narrow_fraction == 0 {
      // +-Zero
      wide_exponent = 0;
      wide_fraction = 0;
    } else {
      // Subnormal numbers
      // We can represent it as a normalized value in wider type,
      // we have to shift fraction until we get 1.new_fraction
      // and decrement exponent for each shift.
      // FIXME; what is the implicit precondition here?
      wide_exponent = (1 - NARROW::BIAS + WIDE::BIAS) as u32;
      while 0 == (wide_fraction & (1 << WIDE::FRACTION_WIDTH)) {
        wide_exponent -= 1;
        wide_fraction <<= 1;
      }
      wide_fraction &= (1 << WIDE::FRACTION_WIDTH) - 1;
    }
  }

  return (sign << WIDE::SIGN_BIT_POS) | (wide_exponent << WIDE::EXPONENT_POS) | wide_fraction;
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_convert() {
    assert_eq!(Binary16::PRECISION, 11);
    assert_eq!(Binary16::EXPONENT_MAX, 15);
    assert_eq!(Binary16::EXPONENT_POS, 10);
    assert_eq!(Binary16::SIGN_BIT_POS, 15);
    assert_eq!(Binary16::STORAGE_BYTES, 2);

    assert_eq!(Binary24::PRECISION, 17);
    assert_eq!(Binary24::EXPONENT_MAX, 63);
    assert_eq!(Binary24::EXPONENT_POS, 16);
    assert_eq!(Binary24::SIGN_BIT_POS, 23);
    assert_eq!(Binary24::STORAGE_BYTES, 3);

    assert_eq!(Binary32::PRECISION, 24);
    assert_eq!(Binary32::EXPONENT_MAX, 127);
    assert_eq!(Binary32::EXPONENT_POS, 23);
    assert_eq!(Binary32::SIGN_BIT_POS, 31);
    assert_eq!(Binary32::STORAGE_BYTES, 4);
  }
}
