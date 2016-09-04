extern crate itertools;
use self::itertools::Itertools;
extern crate crossbeam;
use std;

#[derive(Debug, Copy, Clone)]
pub struct Endian {
  big: bool,
}

impl Endian {
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
}


pub static BIG_ENDIAN: Endian = Endian{big: true};
pub static LITTLE_ENDIAN: Endian = Endian{big: false};

#[allow(non_snake_case)] pub fn BEu32(buf: &[u8], pos: usize) -> u32 {
  (buf[pos] as u32) << 24 |
  (buf[pos+1] as u32) << 16 |
  (buf[pos+2] as u32) << 8 |
  (buf[pos+3] as u32)
}

#[allow(non_snake_case)] pub fn LEu32(buf: &[u8], pos: usize) -> u32 {
  (buf[pos] as u32) |
  (buf[pos+1] as u32) << 8 |
  (buf[pos+2] as u32) << 16 |
  (buf[pos+3] as u32) << 24
}

#[allow(non_snake_case)] pub fn BEu16(buf: &[u8], pos: usize) -> u16 {
  (buf[pos] as u16) << 8 | (buf[pos+1] as u16)
}

#[allow(non_snake_case)] pub fn LEu16(buf: &[u8], pos: usize) -> u16 {
  (buf[pos] as u16) | (buf[pos+1] as u16) << 8
}

pub fn decode_12be(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
  decode_threaded(width, height, &(|out: &mut [u16], start, width, height| {
    let inb = &buf[((start*width*12/8) as usize)..];
    let mut pos: usize = 0;

    for row in 0..height {
      for col in (0..width).step(2) {
        let g1: u16 = inb[pos] as u16;
        let g2: u16 = inb[pos+1] as u16;
        let g3: u16 = inb[pos+2] as u16;
        pos += 3;

        out[width*row+col]   = (g1 << 4) | (g2 >> 4);
        out[width*row+col+1] = ((g2 & 0x0f) << 8) | g3;
      }
    }
  }))
}

pub fn decode_12le(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
  decode_threaded(width, height, &(|out: &mut [u16], start, width, height| {
    let inb = &buf[((start*width*12/8) as usize)..];
    let mut pos: usize = 0;

    for row in 0..height {
      for col in (0..width).step(2) {
        let g1: u16 = inb[pos] as u16;
        let g2: u16 = inb[pos+1] as u16;
        let g3: u16 = inb[pos+2] as u16;
        pos += 3;

        out[width*row+col]   = ((g2 & 0x0f) << 8) | g1;
        out[width*row+col+1] = (g3 << 4) | (g2 >> 4);
      }
    }
  }))
}

pub fn decode_12be_unpacked(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
  decode_threaded(width, height, &(|out: &mut [u16], start, width, height| {
    let inb = &buf[((start*width*2) as usize)..];
    let mut pos: usize = 0;

    for row in 0..height {
      for col in 0..width {
        let g1: u16 = inb[pos] as u16;
        let g2: u16 = inb[pos+1] as u16;
        pos += 2;

        out[width*row+col] = ((g1 & 0x0f) << 8) | g2;
      }
    }
  }))
}

pub fn decode_16le(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
  decode_threaded(width, height, &(|out: &mut [u16], start, width, height| {
    let inb = &buf[((start*width*2) as usize)..];
    let mut pos: usize = 0;

    for row in 0..height {
      for col in 0..width {
        let g1: u16 = inb[pos] as u16;
        let g2: u16 = inb[pos+1] as u16;
        pos += 2;

        out[width*row+col] = (g2 << 8) | g1;
      }
    }
  }))
}

pub fn decode_threaded<F>(width: usize, height: usize, closure: &F) -> Vec<u16>
  where F : Fn(&mut [u16], usize, usize, usize)+std::marker::Sync {
  let mut out: Vec<u16> = vec![0; (width*height) as usize];

  // Default to 4 threads as that should be close to the ideal speedup on most
  // machines. In the future we need a way to have this be a parameter or even
  // to set it dynamically based on how loaded the machine is.
  let threads = 4;

  // If we're only using one thread do it all sequentially and be done with it
  if threads < 2 || height < threads {
    closure(&mut out, 0, width, height);
    return out
  }

  let mut split = height/threads;
  if split*threads < height { // Make sure the last split is always the smallest
    split += 1;
  }

  crossbeam::scope(|scope| {
    let mut handles = Vec::new();
    for (i,out_part) in (&mut out[..]).chunks_mut(split*width).enumerate() {
      let start = split*(i);
      let tall = out_part.len()/width;
      let handle = scope.spawn(move || {
        closure(out_part, start, width, tall);
      });
      handles.push(handle);
    }

    for h in handles { h.join() };
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
      let base = center - ((upper - lower + 2) / 4);
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

pub struct BitPump<'a> {
  buffer: &'a [u8],
  offset: usize,
}

impl<'a> BitPump<'a> {
  pub fn new(src: &'a [u8]) -> BitPump {
    BitPump {
      buffer: src,
      offset: 0,
    }
  }

  pub fn peek_bits(&self, nbits: u32) -> u32 {
    (LEu32(self.buffer, self.offset>>3) >> (self.offset&7)) & ((1<<nbits) - 1)
  }

  pub fn get_bits(&mut self, nbits: u32) -> u32 {
    let val = self.peek_bits(nbits);
    self.offset += nbits as usize;
    val
  }
}
