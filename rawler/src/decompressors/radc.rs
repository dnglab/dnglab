// SPDX-License-Identifier: LGPL-2.1
// Copyright 2024 Daniel Vogelbacher <daniel@chaospixel.com>
// Originally written in C in dcraw.c by Dave Coffin
//
// Kodak Run Adaptive Differential Coding (RADC)

use rayon::iter::IndexedParallelIterator;
use rayon::iter::ParallelIterator;

use crate::Result;
use crate::alloc_image_ok;
use crate::bits::LookupTable;
use crate::buffer::PaddedBuf;
use crate::pixarray::PixU16;
use crate::pumps::BitPump;
use crate::pumps::BitPumpMSB;

#[rustfmt::skip]
const HUFF_INIT: [(u8, i8); 130] = [
    (1,1), (2,3), (3,4), (4,2), (5,7), (6,5), (7,6), (7,8),
    (1,0), (2,1), (3,3), (4,4), (5,2), (6,7), (7,6), (8,5), (8,8),
    (2,1), (2,3), (3,0), (3,2), (3,4), (4,6), (5,5), (6,7), (6,8),
    (2,0), (2,1), (2,3), (3,2), (4,4), (5,6), (6,7), (7,5), (7,8),
    (2,1), (2,4), (3,0), (3,2), (3,3), (4,7), (5,5), (6,6), (6,8),
    (2,3), (3,1), (3,2), (3,4), (3,5), (3,6), (4,7), (5,0), (5,8),
    (2,3), (2,6), (3,0), (3,1), (4,4), (4,5), (4,7), (5,2), (5,8),
    (2,4), (2,7), (3,3), (3,6), (4,1), (4,2), (4,5), (5,0), (5,8),
    (2,6), (3,1), (3,3), (3,5), (3,7), (3,8), (4,0), (5,2), (5,4),
    (2,0), (2,1), (3,2), (3,3), (4,4), (4,5), (5,6), (5,7), (4,8),
    (1,0), (2,2), (2,-2),
    (1,-3), (1,3),
    (2,-17), (2,-5), (2,5), (2,17),
    (2,-7), (2,2), (2,9), (2,18),
    (2,-18), (2,-9), (2,-2), (2,7),
    (2,-28), (2,28), (3,-49), (3,-9), (3,9), (4,49), (5,-79), (5,79),
    (2,-1), (2,13), (2,26), (3,39), (4,-16), (5,55), (6,-37), (6,76),
    (2,-26), (2,-13), (2,1), (3,-39), (4,16), (5,-55), (6,-76), (6,37)
];

#[derive(Default, Clone, Copy, PartialEq, PartialOrd)]
struct HuffSymbol {
  bitcnt: u8,
  value: u8,
}

struct HuffDecoder {
  cache: [[HuffSymbol; 256]; 19],
}

impl HuffDecoder {
  /// Create new HuffmanDecoder
  ///
  /// cbpp is Compressed Bits Per Pixel
  fn new(cbpp: u8) -> Self {
    let mut cache = [[HuffSymbol::default(); 256]; 19];
    let mut a = 0;
    for x in HUFF_INIT {
      for _ in 0..(256 >> x.0) {
        // max bit value in cache
        cache.as_flattened_mut()[a].bitcnt = x.0;
        cache.as_flattened_mut()[a].value = x.1 as u8;
        a += 1;
      }
    }
    for c in 0..256 {
      cache[18][c].bitcnt = 8 - cbpp;
      cache[18][c].value = ((c as u8) >> cbpp << cbpp) | 1 << (cbpp - 1);
    }
    Self { cache }
  }

  #[inline(always)]
  fn huff_decode(&self, pump: &mut dyn BitPump, tree: usize) -> i8 {
    let code = pump.peek_bits(8) as usize;
    let sym = self.cache[tree][code];
    pump.consume_bits(sym.bitcnt as u32);
    sym.value as i8
  }
}

/// Decompress a RADC buffer
///
/// cbpp is Compressed Bits Per Pixel
pub fn decompress(src: &PaddedBuf, width: usize, height: usize, cbpp: u8, dummy: bool) -> Result<PixU16> {
  log::debug!("RADC decompress with cbpp: {}, width: {}, height: {}", cbpp, width, height);
  let mut out = alloc_image_ok!(width, height, dummy);

  let mut last: [i16; 3] = [16, 16, 16];
  let mut mul: [i16; 3];
  let mut buf: [[[i16; 386]; 3]; 3] = [[[2048; 386]; 3]; 3];

  let tbl = {
    const PT: [(usize, f32); 6] = [(0, 0.0), (1280, 1344.0), (2320, 3616.0), (3328, 8000.0), (4095, 16383.0), (65535, 16383.0)];
    let mut curve = vec![0; 65536];
    for i in 1..PT.len() {
      for c in PT[i - 1].0..=PT[i].0 {
        curve[c] = ((c - PT[i - 1].0) as f32 / (PT[i].0 - PT[i - 1].0) as f32 * (PT[i].1 - PT[i - 1].1) + PT[i - 1].1 + 0.5) as u16;
      }
    }
    LookupTable::new_with_bits(&curve, 16)
  };

  let dec = HuffDecoder::new(cbpp);
  let mut pump = BitPumpMSB::new(src);

  for row in (0..height).step_by(4) {
    mul = [pump.get_bits(6) as i16, pump.get_bits(6) as i16, pump.get_bits(6) as i16];

    for c in 0..3 {
      let predictor = |buf: &[[[i16; 386]; 3]; 3], x: usize, y: usize| -> i16 {
        (if c > 0 {
          (buf[c][y - 1][x] as i32 + buf[c][y][x + 1] as i32) / 2
        } else {
          (buf[c][y - 1][x + 1] as i32 + 2 * buf[c][y - 1][x] as i32 + buf[c][y][x + 1] as i32) / 4
        }) as i16
      };

      let mut val: i32 = ((0x1000000 / (last[c] as i32) + 0x7ff) >> 12) * mul[c] as i32;
      let s = if val > 65564 { 10 } else { 12 };
      let x: i32 = (1 << (s - 1)) - 1;
      val <<= 12 - s;
      buf[c].as_flattened_mut().iter_mut().for_each(|i| *i = ((*i as i32 * val + x) >> s) as i16);
      last[c] = mul[c];

      let max = if c == 0 { 1 } else { 0 };
      for r in 0..=max {
        buf[c][1][width / 2] = mul[c] << 7;
        buf[c][2][width / 2] = mul[c] << 7;

        let mut tree = 1;
        let mut col = width / 2;
        while col > 0 {
          tree = dec.huff_decode(&mut pump, tree) as usize;
          if tree != 0 {
            col -= 2;
            if tree == 8 {
              for y in 1..3 {
                for x in (col..=(col + 1)).rev() {
                  buf[c][y][x] = (dec.huff_decode(&mut pump, 18) as u8) as i16 * mul[c];
                  assert!(buf[c][y][x] >= 0);
                }
              }
            } else {
              for y in 1..3 {
                for x in (col..=(col + 1)).rev() {
                  buf[c][y][x] = dec.huff_decode(&mut pump, tree + 10) as i16 * 16 + predictor(&buf, x, y);
                }
              }
            }
          } else {
            loop {
              let nreps = if col > 2 { dec.huff_decode(&mut pump, 9) + 1 } else { 1 };

              for rep in 0..8 {
                if rep < nreps && col > 0 {
                  col -= 2;
                  for y in 1..3 {
                    for x in (col..=(col + 1)).rev() {
                      buf[c][y][x] = predictor(&buf, x, y);
                    }
                  }
                  if rep & 1 > 0 {
                    let step = dec.huff_decode(&mut pump, 10) << 4;
                    for y in 1..3 {
                      for x in (col..=(col + 1)).rev() {
                        buf[c][y][x] += step as i16;
                      }
                    }
                  }
                }
              }
              if nreps != 9 {
                break;
              }
            }
          }
        }
        for y in 0..2 {
          for x in 0..(width / 2) {
            let val = ((buf[c][y + 1][x] as i32) << 4) / mul[c] as i32;
            let val = if val < 0 { 0 } else { val };
            if c > 0 {
              *out.at_mut(row + y * 2 + c - 1, x * 2 + 2 - c) = val as u16;
            } else {
              *out.at_mut(row + r * 2 + y, x * 2 + y) = val as u16;
            }
          }
        }

        // Copy buffer from buf[c][2] to buf[c][0]
        // Borrow checker needs this hack...
        let (dst, src) = buf[c].split_at_mut(2);
        if c == 0 {
          dst[0][1..].copy_from_slice(&src[2 - 2][..386 - 1]);
        } else {
          dst[0][..].copy_from_slice(&src[2 - 2][..]);
        }
      }
    }

    for y in row..row + 4 {
      for x in 0..width {
        if ((x + y) & 1) > 0 {
          let r = if x > 0 { x - 1 } else { x + 1 };
          let s = if x + 1 < width { x + 1 } else { x - 1 };
          let val = (*out.at(y, x) as i32 - 2048) * 2 + ((*out.at(y, r) as i32 + *out.at(y, s) as i32) / 2);
          if val < 0 {
            *out.at_mut(y, x) = 0;
          } else {
            *out.at_mut(y, x) = val as u16;
          }
        }
      }
    }
  }

  out.par_pixel_rows_mut().enumerate().for_each(|(_row, line)| {
    let mut random = ((line[0] as u32) << 16) | line[1] as u32;
    for x in line {
      *x = tbl.dither(*x, &mut random);
    }
  });

  Ok(out)
}
