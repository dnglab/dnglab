// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::{
  cmp::Ordering,
  fmt::{Debug, Display, Write},
  ops::{Add, BitAnd, BitOr, BitXor, Div, Mul, Not, Shl, ShlAssign, Shr, ShrAssign, Sub},
};

pub type BitArray8 = BitArray<u8>;
pub type BitArray16 = BitArray<u16>;
pub type BitArray32 = BitArray<u32>;
pub type BitArray64 = BitArray<u64>;
pub type BitArray128 = BitArray<u128>;

#[derive(Debug, Clone, Copy, Default)]
pub struct BitArray<T: BitStorage> {
  storage: T,
  nbits: usize,
}

impl<T: BitStorage> PartialEq for BitArray<T> {
  fn eq(&self, other: &Self) -> bool {
    self.nbits == other.nbits && self.storage == other.storage
  }
}

impl<T: BitStorage> Eq for BitArray<T> {}

impl<T: BitStorage> PartialOrd for BitArray<T> {
  fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
    Some(self.cmp(other))
  }
}

impl<T: BitStorage> Ord for BitArray<T> {
  fn cmp(&self, other: &Self) -> std::cmp::Ordering {
    if self.nbits > other.nbits {
      Ordering::Greater
    } else if self.nbits < other.nbits {
      Ordering::Less
    } else {
      self.storage.cmp(&other.storage)
    }
  }
}

impl<T: BitStorage> Display for BitArray<T> {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let mut value = self.clone();
    let mut str = String::new();
    while !value.is_empty() {
      match value.pop() {
        true => str.push('1'),
        false => str.push('0'),
      }
    }
    for c in str.chars().rev() {
      f.write_char(c)?;
    }
    Ok(())
  }
}

impl<T: BitStorage> BitArray<T> {
  pub fn new() -> Self {
    Self {
      storage: T::default(),
      nbits: 0,
    }
  }

  pub fn len(&self) -> usize {
    self.nbits
  }

  pub fn storage(&self) -> T {
    self.storage
  }

  pub fn is_full(&self) -> bool {
    self.nbits == T::bit_size()
  }

  pub fn is_empty(&self) -> bool {
    self.nbits == 0
  }

  pub fn push(&mut self, bit: bool) {
    if self.is_full() {
      panic!("BitArray is full");
    } else {
      self.nbits += 1;
      self.storage = self.storage | T::from(bit) << (T::bit_size() - self.nbits);
    }
  }

  pub fn pop(&mut self) -> bool {
    if self.is_empty() {
      panic!("BitArray is empty");
    } else {
      let mask = T::from(true) << (T::bit_size() - self.nbits);
      let bit = self.storage & mask;
      self.storage = self.storage & !mask;
      self.nbits -= 1;
      !(bit == T::from(false))
    }
  }

  pub fn get_msb(&self) -> T {
    self.storage
  }

  pub fn get_lsb(&self) -> T {
    self.storage >> (T::bit_size() - self.nbits)
  }

  pub fn from_msb(nbits: usize, value: T) -> Self {
    Self { storage: value, nbits }
  }

  pub fn from_lsb(nbits: usize, value: T) -> Self {
    Self {
      storage: value << (T::bit_size() - nbits),
      nbits,
    }
  }
}

pub trait BitStorage:
  Default
  + Debug
  + Display
  + Copy
  + Clone
  + ShlAssign
  + ShrAssign
  + Shl<usize>
  + Shl<usize, Output = Self>
  + Shr<usize>
  + Shr<usize, Output = Self>
  + Add
  + Sub
  + Div
  + Mul
  + BitXor<Self, Output = Self>
  + BitOr<Self, Output = Self>
  + BitAnd<Self, Output = Self>
  + Not<Output = Self>
  + PartialEq
  + From<bool>
  + Ord
  + PartialOrd
  + Eq
  + PartialEq
{
  fn bit_size() -> usize;
}

impl BitStorage for u8 {
  fn bit_size() -> usize {
    Self::BITS as usize
  }
}
impl BitStorage for u16 {
  fn bit_size() -> usize {
    Self::BITS as usize
  }
}
impl BitStorage for u32 {
  fn bit_size() -> usize {
    Self::BITS as usize
  }
}
impl BitStorage for u64 {
  fn bit_size() -> usize {
    Self::BITS as usize
  }
}
impl BitStorage for u128 {
  fn bit_size() -> usize {
    Self::BITS as usize
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn check_storage() -> std::result::Result<(), Box<dyn std::error::Error>> {
    crate::init_test_logger();
    assert_eq!(BitArray16::from_lsb(3, 0b110).storage(), 0b1100_0000_0000_0000);
    assert_eq!(BitArray16::from_msb(3, 0b110 << u16::BITS - 3).storage(), 0b1100_0000_0000_0000);
    Ok(())
  }

  #[test]
  fn push_check_storage() -> std::result::Result<(), Box<dyn std::error::Error>> {
    crate::init_test_logger();
    let mut bits = BitArray8::new();
    bits.push(true);
    assert_eq!(bits.len(), 1);
    assert_eq!(bits.storage(), 0b1000_0000);
    Ok(())
  }

  #[test]
  fn ppo_check_storage() -> std::result::Result<(), Box<dyn std::error::Error>> {
    crate::init_test_logger();
    let mut bits = BitArray8::new();
    bits.push(true);
    bits.push(false);
    bits.push(true);
    assert_eq!(bits.len(), 3);
    assert_eq!(bits.storage(), 0b1010_0000);
    assert_eq!(bits.pop(), true);
    assert_eq!(bits.storage(), 0b1000_0000);
    assert_eq!(bits.pop(), false);
    assert_eq!(bits.storage(), 0b1000_0000);
    assert_eq!(bits.pop(), true);
    assert_eq!(bits.storage(), 0b0000_0000);
    assert!(bits.is_empty());
    Ok(())
  }

  #[test]
  fn bitvec_compare() -> std::result::Result<(), Box<dyn std::error::Error>> {
    crate::init_test_logger();
    assert!(BitArray8::from_lsb(1, 0b1) > BitArray8::from_lsb(1, 0b0));
    assert!(BitArray8::from_lsb(2, 0b00) > BitArray8::from_lsb(1, 0b0));
    assert!(BitArray8::from_lsb(2, 0b11) < BitArray8::from_lsb(3, 0b000));
    assert!(BitArray8::from_lsb(3, 0b101) == BitArray8::from_lsb(3, 0b101));
    assert!(BitArray8::from_lsb(3, 0b101) != BitArray8::from_lsb(3, 0b111));

    assert_eq!(10u8.cmp(&20u8), Ordering::Less);
    assert_eq!(BitArray8::from_lsb(1, 0b0).cmp(&BitArray8::from_lsb(1, 0b1)), Ordering::Less);
    assert_eq!(BitArray8::from_lsb(1, 0b1).cmp(&BitArray8::from_lsb(1, 0b0)), Ordering::Greater);
    assert_eq!(BitArray8::from_lsb(1, 0b1).cmp(&BitArray8::from_lsb(1, 0b1)), Ordering::Equal);
    assert_eq!(BitArray8::from_lsb(1, 0b0).cmp(&BitArray8::from_lsb(2, 0b1)), Ordering::Less);
    assert_eq!(BitArray8::from_lsb(1, 0b1).cmp(&BitArray8::from_lsb(2, 0b0)), Ordering::Less);
    assert_eq!(BitArray8::from_lsb(1, 0b1).cmp(&BitArray8::from_lsb(2, 0b1)), Ordering::Less);
    Ok(())
  }
}
