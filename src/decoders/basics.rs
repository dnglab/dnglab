use std;
use std::mem;
extern crate rayon;
use self::rayon::prelude::*;

extern crate byteorder;
use self::byteorder::{BigEndian, LittleEndian, ByteOrder};

pub fn clampbits(val: i32, bits: u32) -> i32 {
  let temp = val >> bits;
  if temp != 0 {
    !temp >> (32-bits)
  } else {
    val
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

pub fn decode_8bit_wtable(buf: &[u8], tbl: &LookupTable, width: usize, height: usize) -> Vec<u16> {
  decode_threaded(width, height, &(|out: &mut [u16], row| {
    let inb = &buf[(row*width)..];
    let mut random = LEu32(inb, 0);

    for (o, i) in out.chunks_mut(1).zip(inb.chunks(1)) {
      o[0] = tbl.dither(i[0] as u16, &mut random);
    }
  }))
}

pub fn decode_10le_lsb16(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
  decode_threaded(width, height, &(|out: &mut [u16], row| {
    let inb = &buf[(row*width*10/8)..];

    for (o, i) in out.chunks_mut(8).zip(inb.chunks(10)) {
      let g1:  u16 = i[0] as u16;
      let g2:  u16 = i[1] as u16;
      let g3:  u16 = i[2] as u16;
      let g4:  u16 = i[3] as u16;
      let g5:  u16 = i[4] as u16;
      let g6:  u16 = i[5] as u16;
      let g7:  u16 = i[6] as u16;
      let g8:  u16 = i[7] as u16;
      let g9:  u16 = i[8] as u16;
      let g10: u16 = i[9] as u16;

      o[0] = g2 << 2  | g1 >> 6;
      o[1] = (g1 & 0x3f) << 4 | g4 >> 4;
      o[2] = (g4 & 0x0f) << 6 | g3 >> 2;
      o[3] = (g3 & 0x03) << 8 | g6;
      o[4] = g5 << 2 | g8 >> 6;
      o[5] = (g8 & 0x3f) << 4 | g7 >> 4;
      o[6] = (g7 & 0x0f) << 6 | g10 >> 2;
      o[7] = (g10 & 0x03) << 8 | g9;
    }
  }))
}

pub fn decode_10le(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
  decode_threaded(width, height, &(|out: &mut [u16], row| {
    let inb = &buf[(row*width*10/8)..];

    for (o, i) in out.chunks_mut(4).zip(inb.chunks(5)) {
      let g1:  u16 = i[0] as u16;
      let g2:  u16 = i[1] as u16;
      let g3:  u16 = i[2] as u16;
      let g4:  u16 = i[3] as u16;
      let g5:  u16 = i[4] as u16;

      o[0] = g1 << 2  | g2 >> 6;
      o[1] = (g2 & 0x3f) << 4 | g3 >> 4;
      o[2] = (g3 & 0x0f) << 6 | g3 >> 2;
      o[3] = (g4 & 0x03) << 8 | g5;
    }
  }))
}

pub fn decode_12be(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
  decode_threaded(width, height, &(|out: &mut [u16], row| {
    let inb = &buf[(row*width*12/8)..];

    for (o, i) in out.chunks_mut(2).zip(inb.chunks(3)) {
      let g1: u16 = i[0] as u16;
      let g2: u16 = i[1] as u16;
      let g3: u16 = i[2] as u16;

      o[0] = (g1 << 4) | (g2 >> 4);
      o[1] = ((g2 & 0x0f) << 8) | g3;
    }
  }))
}

pub fn decode_12be_msb16(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
  let mut out: Vec<u16> = vec![0; width*height];

  for (o, i) in out.chunks_mut(4).zip(buf.chunks(6)) {
    let g1:  u16 = i[ 0] as u16;
    let g2:  u16 = i[ 1] as u16;
    let g3:  u16 = i[ 2] as u16;
    let g4:  u16 = i[ 3] as u16;
    let g5:  u16 = i[ 4] as u16;
    let g6:  u16 = i[ 5] as u16;

    o[0] = (g2 << 4) | (g1 >> 4);
    o[1] = ((g1 & 0x0f) << 8) | g4;
    o[2] = (g3 << 4) | (g6 >> 4);
    o[3] = ((g6 & 0x0f) << 8) | g5;
  }

  out
}

pub fn decode_12be_msb32(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
  let mut out: Vec<u16> = vec![0; width*height];

  for (o, i) in out.chunks_mut(8).zip(buf.chunks(12)) {
    let g1:  u16 = i[ 0] as u16;
    let g2:  u16 = i[ 1] as u16;
    let g3:  u16 = i[ 2] as u16;
    let g4:  u16 = i[ 3] as u16;
    let g5:  u16 = i[ 4] as u16;
    let g6:  u16 = i[ 5] as u16;
    let g7:  u16 = i[ 6] as u16;
    let g8:  u16 = i[ 7] as u16;
    let g9:  u16 = i[ 8] as u16;
    let g10: u16 = i[ 9] as u16;
    let g11: u16 = i[10] as u16;
    let g12: u16 = i[11] as u16;

    o[0] = (g4 << 4) | (g3 >> 4);
    o[1] = ((g3 & 0x0f) << 8) | g2;
    o[2] = (g1 << 4) | (g8 >> 4);
    o[3] = ((g8 & 0x0f) << 8) | g7;
    o[4] = (g6 << 4) | (g5 >> 4);
    o[5] = ((g5 & 0x0f) << 8) | g12;
    o[6] = (g11 << 4) | (g10 >> 4);
    o[7] = ((g10 & 0x0f) << 8) | g9;
  }

  out
}

pub fn decode_12le_wcontrol(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
  // Calulate expected bytes per line.
  let perline = width * 12 / 8 + ((width+2) / 10);

  decode_threaded(width, height, &(|out: &mut [u16], row| {
    let inb = &buf[(row*perline)..];

    for (oc, ic) in out.chunks_mut(10).zip(inb.chunks(16)) {
      for (o, i) in oc.chunks_mut(2).zip(ic.chunks(3)) {
        let g1: u16 = i[0] as u16;
        let g2: u16 = i[1] as u16;
        let g3: u16 = i[2] as u16;

        o[0] = ((g2 & 0x0f) << 8) | g1;
        o[1] = (g3 << 4) | (g2 >> 4);
      }
    }
  }))
}

pub fn decode_12be_wcontrol(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
  // Calulate expected bytes per line.
  let perline = width * 12 / 8 + ((width+2) / 10);

  decode_threaded(width, height, &(|out: &mut [u16], row| {
    let inb = &buf[(row*perline)..];

    for (oc, ic) in out.chunks_mut(10).zip(inb.chunks(16)) {
      for (o, i) in oc.chunks_mut(2).zip(ic.chunks(3)) {
        let g1: u16 = i[0] as u16;
        let g2: u16 = i[1] as u16;
        let g3: u16 = i[2] as u16;

        o[0] = (g1 << 4) | (g2 >> 4);
        o[1] = ((g2 & 0x0f) << 8) | g3;
      }
    }
  }))
}


pub fn decode_12be_interlaced(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
  let half = (height+1) >> 1;
  // Second field is 2048 byte aligned
  let second_field_offset = ((half*width*3/2 >> 11) + 1) << 11;
  let second_field = &buf[second_field_offset..];

  decode_threaded(width, height, &(|out: &mut [u16], row| {
    let off = row/2*width*12/8;
    let inb = if (row % 2) == 0 { &buf[off..] } else { &second_field[off..] };

    for (o, i) in out.chunks_mut(2).zip(inb.chunks(3)) {
      let g1: u16 = i[0] as u16;
      let g2: u16 = i[1] as u16;
      let g3: u16 = i[2] as u16;

      o[0] = (g1 << 4) | (g2 >> 4);
      o[1] = ((g2 & 0x0f) << 8) | g3;
    }
  }))
}

pub fn decode_12be_interlaced_unaligned(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
  let half = (height+1) >> 1;
  let second_field = &buf[half*width*12/8..];

  decode_threaded(width, height, &(|out: &mut [u16], row| {
    let off = row/2*width*12/8;
    let inb = if (row % 2) == 0 { &buf[off..] } else { &second_field[off..] };

    for (o, i) in out.chunks_mut(2).zip(inb.chunks(3)) {
      let g1: u16 = i[0] as u16;
      let g2: u16 = i[1] as u16;
      let g3: u16 = i[2] as u16;

      o[0] = (g1 << 4) | (g2 >> 4);
      o[1] = ((g2 & 0x0f) << 8) | g3;
    }
  }))
}

pub fn decode_12le(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
  decode_threaded(width, height, &(|out: &mut [u16], row| {
    let inb = &buf[(row*width*12/8)..];

    for (o, i) in out.chunks_mut(2).zip(inb.chunks(3)) {
      let g1: u16 = i[0] as u16;
      let g2: u16 = i[1] as u16;
      let g3: u16 = i[2] as u16;

      o[0] = ((g2 & 0x0f) << 8) | g1;
      o[1] = (g3 << 4) | (g2 >> 4);
    }
  }))
}

pub fn decode_12le_unpacked(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
  decode_threaded(width, height, &(|out: &mut [u16], row| {
    let inb = &buf[(row*width*2)..];

    for (i, bytes) in (0..width).zip(inb.chunks(2)) {
      out[i] = LEu16(bytes, 0) & 0x0fff;
    }
  }))
}

pub fn decode_12be_unpacked(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
  decode_threaded(width, height, &(|out: &mut [u16], row| {
    let inb = &buf[(row*width*2)..];

    for (i, bytes) in (0..width).zip(inb.chunks(2)) {
      out[i] = BEu16(bytes, 0) & 0x0fff;
    }
  }))
}

pub fn decode_12be_unpacked_left_aligned(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
  decode_threaded(width, height, &(|out: &mut [u16], row| {
    let inb = &buf[(row*width*2)..];

    for (i, bytes) in (0..width).zip(inb.chunks(2)) {
      out[i] = BEu16(bytes, 0) >> 4;
    }
  }))
}

pub fn decode_12le_unpacked_left_aligned(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
  decode_threaded(width, height, &(|out: &mut [u16], row| {
    let inb = &buf[(row*width*2)..];

    for (i, bytes) in (0..width).zip(inb.chunks(2)) {
      out[i] = LEu16(bytes, 0) >> 4;
    }
  }))
}

pub fn decode_14le_unpacked(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
  decode_threaded(width, height, &(|out: &mut [u16], row| {
    let inb = &buf[(row*width*2)..];

    for (i, bytes) in (0..width).zip(inb.chunks(2)) {
      out[i] = LEu16(bytes, 0) & 0x3fff;
    }
  }))
}

pub fn decode_14be_unpacked(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
  decode_threaded(width, height, &(|out: &mut [u16], row| {
    let inb = &buf[(row*width*2)..];

    for (i, bytes) in (0..width).zip(inb.chunks(2)) {
      out[i] = BEu16(bytes, 0) & 0x3fff;
    }
  }))
}

pub fn decode_16le(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
  decode_threaded(width, height, &(|out: &mut [u16], row| {
    let inb = &buf[(row*width*2)..];

    for (i, bytes) in (0..width).zip(inb.chunks(2)) {
      out[i] = LEu16(bytes, 0);
    }
  }))
}

pub fn decode_16le_skiplines(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
  decode_threaded(width, height, &(|out: &mut [u16], row| {
    let inb = &buf[(row*width*4)..];

    for (i, bytes) in (0..width).zip(inb.chunks(2)) {
      out[i] = LEu16(bytes, 0);
    }
  }))
}

pub fn decode_16be(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
  decode_threaded(width, height, &(|out: &mut [u16], row| {
    let inb = &buf[(row*width*2)..];

    for (i, bytes) in (0..width).zip(inb.chunks(2)) {
      out[i] = BEu16(bytes, 0);
    }
  }))
}

pub fn decode_threaded<F>(width: usize, height: usize, closure: &F) -> Vec<u16>
  where F : Fn(&mut [u16], usize)+std::marker::Sync {

  let mut out: Vec<u16> = vec![0; width*height];
  out.par_chunks_mut(width).enumerate().for_each(|(row, line)| {
    closure(line, row);
  });
  out
}

pub fn decode_threaded_multiline<F>(width: usize, height: usize, lines: usize, closure: &F) -> Vec<u16>
  where F : Fn(&mut [u16], usize)+std::marker::Sync {

  let mut out: Vec<u16> = vec![0; width*height];
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

  pub fn dither(&self, value: u16, rand: &mut u32) -> u16 {
    let (_, sbase, sdelta) = self.table[value as usize];
    let base = sbase as u32;
    let delta = sdelta as u32;
    let pixel = base + ((delta * (*rand & 2047) + 1024) >> 12);
    *rand = 15700 * (*rand & 65535) + (*rand >> 16);
    pixel as u16
  }
}

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

  fn get_bits(&mut self, num: u32) -> u32 {
    if num == 0 {
      return 0
    }

    let val = self.peek_bits(num);
    self.consume_bits(num);

    val
  }

  fn peek_ibits(&mut self, num: u32) -> i32 {
    unsafe{mem::transmute(self.peek_bits(num))}
  }

  fn get_ibits(&mut self, num: u32) -> i32 {
    unsafe{mem::transmute(self.get_bits(num))}
  }

  // Sign extend ibits
  fn get_ibits_sextended(&mut self, num: u32) -> i32 {
    let val = self.get_ibits(num);
    val.wrapping_shl(32 - num).wrapping_shr(32 - num)
  }
}

impl<'a> BitPump for BitPumpLSB<'a> {
  fn peek_bits(&mut self, num: u32) -> u32 {
    if num > self.nbits {
      let inbits: u64 = LEu32(self.buffer, self.pos) as u64;
      self.bits = ((inbits << 32) | (self.bits << (32-self.nbits))) >> (32-self.nbits);
      self.pos += 4;
      self.nbits += 32;
    }
    (self.bits & (0x0ffffffffu64 >> (32-num))) as u32
  }

  fn consume_bits(&mut self, num: u32) {
    self.nbits -= num;
    self.bits >>= num;
  }
}

impl<'a> BitPump for BitPumpMSB<'a> {
  fn peek_bits(&mut self, num: u32) -> u32 {
    if num > self.nbits {
      let inbits: u64 = BEu32(self.buffer, self.pos) as u64;
      self.bits = (self.bits << 32) | inbits;
      self.pos += 4;
      self.nbits += 32;
    }
    (self.bits >> (self.nbits-num)) as u32
  }

  fn consume_bits(&mut self, num: u32) {
    self.nbits -= num;
    self.bits &= (1 << self.nbits) - 1;
  }
}

impl<'a> BitPump for BitPumpMSB32<'a> {
  fn peek_bits(&mut self, num: u32) -> u32 {
    if num > self.nbits {
      let inbits: u64 = LEu32(self.buffer, self.pos) as u64;
      self.bits = (self.bits << 32) | inbits;
      self.pos += 4;
      self.nbits += 32;
    }
    (self.bits >> (self.nbits-num)) as u32
  }

  fn consume_bits(&mut self, num: u32) {
    self.nbits -= num;
    self.bits &= (1 << self.nbits) - 1;
  }
}

impl<'a> BitPump for BitPumpJPEG<'a> {
  fn peek_bits(&mut self, num: u32) -> u32 {
    if num > self.nbits && !self.finished {
      if self.buffer[self.pos+0] != 0xff &&
         self.buffer[self.pos+1] != 0xff &&
         self.buffer[self.pos+2] != 0xff &&
         self.buffer[self.pos+3] != 0xff {
        let inbits: u64 = BEu32(self.buffer, self.pos) as u64;
        self.bits = (self.bits << 32) | inbits;
        self.pos += 4;
        self.nbits += 32;
      } else {
        // Read 32 bits the hard way
        let mut read_bytes = 0;
        while read_bytes < 4 && !self.finished {
          let byte = {
            let nextbyte = self.buffer[self.pos];
            if nextbyte != 0xff {
              nextbyte
            } else if self.buffer[self.pos+1] == 0x00 {
              self.pos += 1; // Skip the extra byte used to mark 255
              nextbyte
            } else {
              self.finished = true;
              0
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

    (self.bits >> (self.nbits-num)) as u32
  }

  fn consume_bits(&mut self, num: u32) {
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
    ByteStream {
      buffer: src,
      pos: 0,
      endian: endian,
    }
  }

  pub fn get_pos(&self) -> usize { self.pos }

  pub fn peek_u8(&self) -> u8 { self.buffer[self.pos] }
  pub fn get_u8(&mut self) -> u8 {
    let val = self.peek_u8();
    self.pos += 1;
    val
  }

  pub fn peek_u16(&self) -> u16 { self.endian.ru16(self.buffer, self.pos) }
  pub fn get_u16(&mut self) -> u16 {
    let val = self.peek_u16();
    self.pos += 2;
    val
  }

//  pub fn peek_u32(&self) -> u32 { self.endian.ru32(self.buffer, self.pos) }
//  pub fn get_u32(&mut self) -> u32 {
//    let val = self.peek_u32();
//    self.pos += 4;
//    val
//  }

  pub fn consume_bytes(&mut self, num: usize) {
    self.pos += num
  }

  pub fn skip_to_marker(&mut self) -> Result<usize, String> {
    let mut skip_count = 0;
    while !(self.buffer[self.pos] == 0xFF &&
            self.buffer[self.pos+1] != 0 &&
            self.buffer[self.pos+1] != 0xFF) {
      self.pos += 1;
      skip_count += 1;
      if self.pos >= self.buffer.len() {
        return Err("No marker found inside rest of buffer".to_string())
      }
    }
    self.pos += 1; // Make the next byte the marker
    Ok(skip_count+1)
  }
}
