use crate::{decoders::*, decompressors::decompress_strips_fn, pumps::BitPump};

pub(crate) fn decode_panasonic_v4(buf: &[u8], width: usize, height: usize, split: bool, dummy: bool) -> PixU16 {
  decompress_strips_fn(
    width,
    height,
    5,
    dummy,
    &(|out: &mut [u16], _strip, row| {
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
              pred[i & 1] = (nonz[i & 1] << 4) | (pump.get_bits(4) as i32);
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

  /// Byte `idx` of the current block in load order, zero past its end.
  fn block_byte(&self, idx: usize) -> u32 {
    if idx >= 0x4000 {
      return 0;
    }
    let idx = if self.split { (idx + 0x4000 - 0x2008) % 0x4000 } else { idx };
    self.buffer.get(self.pos - 0x4000 + idx).copied().unwrap_or(0) as u32
  }
}

impl<'a> BitPump for BitPumpPanasonic<'a> {
  fn peek_bits(&mut self, num: u32) -> u32 {
    if num > self.nbits {
      self.nbits += 0x4000 * 8;
      self.pos += 0x4000;
    }
    // The two bytes are read from the 0x4000 byte block in its load order, as LibRaw does:
    // the byte after the last one is zero, never the first byte of the next block, and with
    // `split` both bytes go through the remapping, not only the first one.
    let byte = (((self.nbits - num) >> 3) ^ 0x3ff0) as usize;
    let bits = self.block_byte(byte) | (self.block_byte(byte + 1) << 8);
    (bits >> ((self.nbits - num) & 7)) & (0x0ffffffffu32 >> (32 - num))
  }

  fn consume_bits(&mut self, num: u32) {
    self.nbits -= num;
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// The second byte of a window comes from the same block in load order, or is zero
  /// past its end, as in LibRaw's `pana_data()`; it is never read from the next block.
  #[test]
  fn window_stays_in_block_where_load_order_wraps() {
    let mut buf = vec![0u8; 0x8000];
    buf[0x3fff] = 0xab;
    buf[0x0000] = 0x12;
    buf[0x4000] = 0xef; // first byte of the next block
    // 8 bits read 4 bits into load order byte `byte`, so they need the byte after it.
    let peek = |byte: u32, split| {
      BitPumpPanasonic {
        buffer: &buf,
        pos: 0x4000,
        nbits: (byte ^ 0x3ff0) * 8 + 12,
        split,
      }
      .peek_bits(8)
    };
    // Split: load order byte 0x2007 is file byte 0x3fff, and 0x2008 is file byte 0.
    assert_eq!(peek(0x2007, true), 0x2a);
    // Unsplit: load order byte 0x3fff is the last one, followed by zero.
    assert_eq!(peek(0x3fff, false), 0x0a);
  }
}
