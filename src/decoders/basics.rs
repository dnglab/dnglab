extern crate itertools;
use self::itertools::Itertools;

#[allow(non_snake_case)] pub fn BEu32(buf: &[u8], pos: usize) -> u32 {
  (buf[pos] as u32) << 24 |
  (buf[pos+1] as u32) << 16 |
  (buf[pos+2] as u32) << 8 |
  (buf[pos+3] as u32)
}

//#[allow(non_snake_case)] pub fn LEu32(buf: &[u8], pos: usize) -> u32 {
//  (buf[pos] as u32) |
//  (buf[pos+1] as u32) << 8 |
//  (buf[pos+2] as u32) << 16 |
//  (buf[pos+3] as u32) << 24
//}

#[allow(non_snake_case)] pub fn BEu16(buf: &[u8], pos: usize) -> u16 {
  (buf[pos] as u16) << 8 | (buf[pos+1] as u16)
}

//#[allow(non_snake_case)] pub fn LEu16(buf: &[u8], pos: usize) -> u16 {
//  (buf[pos] as u16) | (buf[pos+1] as u16) << 8
//}

pub fn decode_12be(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
  let mut buffer: Vec<u16> = vec![0; width*height];
  let mut pos: usize = 0;

  for row in 0..height {
    for col in (0..width).step(2) {
      let g1: u16 = buf[pos] as u16;
      let g2: u16 = buf[pos+1] as u16;
      let g3: u16 = buf[pos+2] as u16;
      pos += 3;

      buffer[width*row+col]   = (g1 << 4) | (g2 >> 4);
      buffer[width*row+col+1] = ((g2 & 0x0f) << 8) | g3;
    }
  }

  buffer
}

pub fn decode_12be_unpacked(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
  let mut buffer: Vec<u16> = vec![0; width*height];
  let mut pos: usize = 0;

  for row in 0..height {
    for col in 0..width {
      let g1: u16 = buf[pos] as u16;
      let g2: u16 = buf[pos+1] as u16;
      pos += 2;

      buffer[width*row+col] = ((g1 & 0x0f) << 8) | g2;
    }
  }

  buffer
}
