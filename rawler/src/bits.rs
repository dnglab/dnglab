// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use byteorder::{BigEndian, ByteOrder, LittleEndian};

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

#[derive(Debug, Copy, Clone)]
pub enum Endian {
  Big,
  Little,
}

impl Endian {
  #[inline]
  pub fn big(&self) -> bool {
    if let Self::Big = *self {
      true
    } else {
      false
    }
  }
  #[inline]
  pub fn little(&self) -> bool {
    if let Self::Little = *self {
      true
    } else {
      false
    }
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



#[allow(non_snake_case)] #[inline] pub fn BEi32(buf: &[u8], pos: usize) -> i32 {
  BigEndian::read_i32(&buf[pos..pos+4])
}

#[allow(non_snake_case)] #[inline] pub fn LEi32(buf: &[u8], pos: usize) -> i32 {
  LittleEndian::read_i32(&buf[pos..pos+4])
}

#[allow(non_snake_case)] #[inline] pub fn BEu32(buf: &[u8], pos: usize) -> u32 {
  BigEndian::read_u32(&buf[pos..pos+4])
}

#[allow(non_snake_case)] #[inline] pub fn LEu32(buf: &[u8], pos: usize) -> u32 {
  LittleEndian::read_u32(&buf[pos..pos+4])
}

#[allow(non_snake_case)] #[inline] pub fn LEf32(buf: &[u8], pos: usize) -> f32 {
  LittleEndian::read_f32(&buf[pos..pos+4])
}

#[allow(non_snake_case)] #[inline] pub fn BEu16(buf: &[u8], pos: usize) -> u16 {
  BigEndian::read_u16(&buf[pos..pos+2])
}

#[allow(non_snake_case)] #[inline] pub fn LEu16(buf: &[u8], pos: usize) -> u16 {
  LittleEndian::read_u16(&buf[pos..pos+2])
}


#[derive(Debug, Clone)]
pub struct LookupTable {
  table: Vec<(u16, u16, u16)>,
}

impl LookupTable {
  pub fn new(table: &[u16]) -> LookupTable {
    let mut tbl = vec![(0,0,0); table.len()];
    for i in 0..table.len() {
      let center = table[i];
      let lower = if i > 0 {table[i-1]} else {center};
      let upper = if i < (table.len()-1) {table[i+1]} else {center};
      let base = if center == 0 {0} else {center - ((upper - lower + 2) / 4)};
      let delta = upper - lower;
      tbl[i] = (center, base, delta);
    }
    LookupTable {
      table: tbl,
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