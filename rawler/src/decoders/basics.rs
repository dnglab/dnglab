use byteorder::{BigEndian, LittleEndian, ByteOrder};
use rayon::prelude::*;

pub use crate::decoders::packed::*;
pub use crate::decoders::pumps::*;

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
pub struct Endian {
  big: bool,
}

impl Endian {
  pub fn ri32(&self, buf: &[u8], pos: usize) -> i32 {
    if self.big {
      BEi32(buf,pos)
    } else {
      LEi32(buf,pos)
    }
  }

  pub fn ru32(&self, buf: &[u8], pos: usize) -> u32 {
    if self.big {
      BEu32(buf,pos)
    } else {
      LEu32(buf,pos)
    }
  }

  pub fn ru16(&self, buf: &[u8], pos: usize) -> u16 {
    if self.big {
      BEu16(buf,pos)
    } else {
      LEu16(buf,pos)
    }
  }

  pub fn little(&self) -> bool { !self.big }
}

pub static BIG_ENDIAN: Endian = Endian{big: true};
pub static LITTLE_ENDIAN: Endian = Endian{big: false};

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

pub fn decode_threaded<F>(width: usize, height: usize, dummy: bool, closure: &F) -> Vec<u16>
  where F : Fn(&mut [u16], usize)+Sync {

  let mut out: Vec<u16> = alloc_image!(width, height, dummy);
    out.par_chunks_mut(width).enumerate().for_each(|(row, line)| {
    closure(line, row);
  });
  out
}

pub fn decode_threaded_multiline<F>(width: usize, height: usize, lines: usize, dummy: bool, closure: &F) -> Vec<u16>
  where F : Fn(&mut [u16], usize)+Sync {

  let mut out: Vec<u16> = alloc_image!(width, height, dummy);
  out.par_chunks_mut(width*lines).enumerate().for_each(|(row, line)| {
    closure(line, row*lines);
  });
  out
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

// For rust <= 1.31 we just alias chunks_exact() and chunks_exact_mut() to the non-exact versions
// so we can use exact everywhere without spreading special cases across the code
#[cfg(needs_chunks_exact)]
mod chunks_exact {
  use std::slice;

  // Add a chunks_exact for &[u8] and Vec<u16>
  pub trait ChunksExact<T> {
    fn chunks_exact(&self, n: usize) -> slice::Chunks<T>;
  }
  impl<'a, T> ChunksExact<T> for &'a [T] {
    fn chunks_exact(&self, n: usize) -> slice::Chunks<T> { self.chunks(n) }
  }
  impl<T> ChunksExact<T> for Vec<T> {
    fn chunks_exact(&self, n: usize) -> slice::Chunks<T> { self.chunks(n) }
  }

  // Add a chunks_exact_mut for &mut[u16] mostly
  pub trait ChunksExactMut<'a, T> {
    fn chunks_exact_mut(self, n: usize) -> slice::ChunksMut<'a, T>;
  }
  impl<'a, T> ChunksExactMut<'a, T> for &'a mut [T] {
    fn chunks_exact_mut(self, n: usize) -> slice::ChunksMut<'a, T> { self.chunks_mut(n) }
  }
}
#[cfg(needs_chunks_exact)] pub use self::chunks_exact::*;
