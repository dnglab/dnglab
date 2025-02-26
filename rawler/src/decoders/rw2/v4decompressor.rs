use crate::{bits::LEu16, decoders::*, pumps::BitPump};

pub(crate) fn decode_panasonic_v4(buf: &[u8], width: usize, height: usize, split: bool, dummy: bool) -> PixU16 {
  decode_threaded_multiline(
    width,
    height,
    5,
    dummy,
    &(|out: &mut [u16], row| {
      let skip = ((width * row * 9) + (width / 14 * 2 * row)) / 8;
      let blocks = skip / 0x4000;
      let src = &buf[blocks * 0x4000..];
      let mut pump = BitPumpPanasonic::new(src, split);
      for _ in 0..(skip % 0x4000) {
        pump.get_bits(8);
      }

      let mut sh: i32 = 0;
      for out in out.chunks_exact_mut(14) {
        let mut pred: [i32; 2] = [0, 0];
        let mut nonz: [i32; 2] = [0, 0];

        for i in 0..14 {
          if (i % 3) == 2 {
            sh = 4 >> (3 - pump.get_bits(2));
          }
          if nonz[i & 1] != 0 {
            let j = pump.get_bits(8) as i32;
            if j != 0 {
              pred[i & 1] -= 0x80 << sh;
              if pred[i & 1] < 0 || sh == 4 {
                pred[i & 1] &= !(-1 << sh);
              }
              pred[i & 1] += j << sh;
            }
          } else {
            nonz[i & 1] = pump.get_bits(8) as i32;
            if nonz[i & 1] != 0 || i > 11 {
              pred[i & 1] = nonz[i & 1] << 4 | (pump.get_bits(4) as i32);
            }
          }
          out[i] = pred[i & 1] as u16;
        }
      }
      Ok(())
    }),
  )
  .expect("Failed to decode") // Decoder should never fail
}

pub struct BitPumpPanasonic<'a> {
  buffer: &'a [u8],
  pos: usize,
  nbits: u32,
  split: bool,
}

impl<'a> BitPumpPanasonic<'a> {
  pub fn new(src: &'a [u8], split: bool) -> BitPumpPanasonic<'a> {
    BitPumpPanasonic {
      buffer: src,
      pos: 0,
      nbits: 0,
      split,
    }
  }
}

impl<'a> BitPump for BitPumpPanasonic<'a> {
  fn peek_bits(&mut self, num: u32) -> u32 {
    if num > self.nbits {
      self.nbits += 0x4000 * 8;
      self.pos += 0x4000;
    }
    let mut byte = (self.nbits - num) >> 3 ^ 0x3ff0;
    if self.split {
      byte = (byte + 0x4000 - 0x2008) % 0x4000;
    }
    let bits = LEu16(self.buffer, byte as usize + self.pos - 0x4000) as u32;
    (bits >> ((self.nbits - num) & 7)) & (0x0ffffffffu32 >> (32 - num))
  }

  fn consume_bits(&mut self, num: u32) {
    self.nbits -= num;
  }
}
