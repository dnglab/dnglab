use crate::{
  decoders::*,
  decompressors::decompress_strips_fn,
  pumps::{BitPump, BitPumpError, check_bits},
};

pub(crate) fn decode_panasonic_v4(buf: &[u8], width: usize, height: usize, split: bool, dummy: bool) -> std::result::Result<PixU16, String> {
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
        pump.get_bits(8)?;
      }

      let mut sh: i32 = 0;
      for out in out.chunks_exact_mut(14) {
        let mut pred: [i32; 2] = [0, 0];
        let mut nonz: [i32; 2] = [0, 0];

        for i in 0..14 {
          if (i % 3) == 2 {
            sh = 4 >> (3 - pump.get_bits(2)?);
          }
          if nonz[i & 1] != 0 {
            let j = pump.get_bits(8)? as i32;
            if j != 0 {
              pred[i & 1] -= 0x80 << sh;
              if pred[i & 1] < 0 || sh == 4 {
                pred[i & 1] &= !(-1 << sh);
              }
              pred[i & 1] += j << sh;
            }
          } else {
            nonz[i & 1] = pump.get_bits(8)? as i32;
            if nonz[i & 1] != 0 || i > 11 {
              pred[i & 1] = (nonz[i & 1] << 4) | (pump.get_bits(4)? as i32);
            }
          }
          out[i] = pred[i & 1] as u16;
        }
      }
      Ok(())
    }),
  )
}

pub struct BitPumpPanasonic<'a> {
  buffer: &'a [u8],
  pos: usize,
  nbits: u32,
  split: bool,
}

impl<'a> BitPumpPanasonic<'a> {
  /// The 16-bit window, shifted by up to 7 bits, holds at least 9 bits
  const MAX_PEEK_BITS: u32 = 9;

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
  fn peek_bits(&mut self, num: u32) -> std::result::Result<u32, BitPumpError> {
    check_bits(num, Self::MAX_PEEK_BITS)?;
    if num > self.nbits {
      if self.pos >= self.buffer.len() {
        return Err(BitPumpError::Eof);
      }
      self.nbits += 0x4000 * 8;
      self.pos += 0x4000;
    }
    // The 16-bit window is read a byte at a time, each byte mapped
    // through the split on its own: a window that straddles the split
    // takes its high byte from this block's first byte, not from the
    // next block, and a window at the block's end takes zero, as
    // LibRaw's buffer does.
    let byte = ((self.nbits - num) >> 3) ^ 0x3ff0;
    let shift = (self.nbits - num) & 7;
    // The last block ends with the file and can be shorter than 16 KB
    let block = &self.buffer[self.pos - 0x4000..self.pos.min(self.buffer.len())];
    let at = |i: u32| -> std::result::Result<u32, BitPumpError> {
      if i >= 0x4000 {
        return Ok(0);
      }
      let i = if self.split { (i + 0x4000 - 0x2008) % 0x4000 } else { i };
      block.get(i as usize).map(|&b| b as u32).ok_or(BitPumpError::Eof)
    };
    // The high byte is only read if the bits reach into it. Otherwise it
    // can be the first byte of the next group, behind the end of the file.
    let high = if shift + num > 8 { at(byte + 1)? } else { 0 };
    let bits = at(byte)? | (high << 8);
    Ok((bits >> shift) & (0x0ffffffffu32 >> (32 - num)))
  }

  fn consume_bits(&mut self, num: u32) {
    self.nbits -= num;
  }
}
