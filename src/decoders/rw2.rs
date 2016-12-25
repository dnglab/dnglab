use decoders::*;
use decoders::tiff::*;
use decoders::basics::*;
use std::f32::NAN;

#[derive(Debug, Clone)]
pub struct Rw2Decoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: TiffIFD<'a>,
}

impl<'a> Rw2Decoder<'a> {
  pub fn new(buf: &'a [u8], tiff: TiffIFD<'a>, rawloader: &'a RawLoader) -> Rw2Decoder<'a> {
    Rw2Decoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for Rw2Decoder<'a> {
  fn image(&self) -> Result<Image,String> {
    let width: usize;
    let height: usize;
    let image = {
      let data = self.tiff.find_ifds_with_tag(Tag::PanaOffsets);
      if data.len() > 0 {
        let raw = data[0];
        width = fetch_tag!(raw, Tag::PanaWidth).get_u32(0) as usize;
        height = fetch_tag!(raw, Tag::PanaLength).get_u32(0) as usize;
        let offset = fetch_tag!(raw, Tag::PanaOffsets).get_u32(0) as usize;
        let src = &self.buffer[offset..];
        Rw2Decoder::decode_panasonic(src, width, height, true)
      } else {
        let raw = fetch_ifd!(&self.tiff, Tag::StripOffsets);
        width = fetch_tag!(raw, Tag::PanaWidth).get_u32(0) as usize;
        height = fetch_tag!(raw, Tag::PanaLength).get_u32(0) as usize;
        let offset = fetch_tag!(raw, Tag::StripOffsets).get_u32(0) as usize;
        let src = &self.buffer[offset..];

        if src.len() >= width*height*2 {
          decode_12le_unpacked_left_aligned(src, width, height)
        } else if src.len() >= width*height*3/2 {
          decode_12le_wcontrol(src, width, height)
        } else {
          Rw2Decoder::decode_panasonic(src, width, height, false)
        }
      }
    };

    let mode = {
      let ratio = width*100/height;
      if ratio < 125 {
        "1:1"
      } else if ratio < 145 {
        "4:3"
      } else if ratio < 165 {
        "3:2"
      } else {
        "16:9"
      }
    };
    let camera = try!(self.rawloader.check_supported_with_mode(&self.tiff, mode));

    ok_image(camera, width as u32, height as u32, try!(self.get_wb()), image)
  }
}

impl<'a> Rw2Decoder<'a> {
  fn get_wb(&self) -> Result<[f32;4], String> {
    if self.tiff.has_entry(Tag::PanaWBsR) && self.tiff.has_entry(Tag::PanaWBsB) {
      let r = fetch_tag!(self.tiff, Tag::PanaWBsR).get_u32(0) as f32;
      let b = fetch_tag!(self.tiff, Tag::PanaWBsB).get_u32(0) as f32;
      Ok([r, 256.0, b, NAN])
    } else if self.tiff.has_entry(Tag::PanaWBs2R)
           && self.tiff.has_entry(Tag::PanaWBs2G)
           && self.tiff.has_entry(Tag::PanaWBs2B) {
      let r = fetch_tag!(self.tiff, Tag::PanaWBs2R).get_u32(0) as f32;
      let g = fetch_tag!(self.tiff, Tag::PanaWBs2G).get_u32(0) as f32;
      let b = fetch_tag!(self.tiff, Tag::PanaWBs2B).get_u32(0) as f32;
      Ok([r, g, b, NAN])
    } else {
      Err("Couldn't find WB".to_string())
    }
  }

  fn decode_panasonic(buf: &[u8], width: usize, height: usize, split: bool) -> Vec<u16> {
    decode_threaded_multiline(width, height, 5, &(|out: &mut [u16], row| {
      let skip = ((width * row * 9) + (width/14 * 2 * row)) / 8;
      let blocks = skip / 0x4000;
      let src = &buf[blocks*0x4000..];
      let mut pump = BitPumpPanasonic::new(src, split);
      for _ in 0..(skip % 0x4000) {
        pump.get_bits(8);
      }

      let mut sh: i32 = 0;
      for out in out.chunks_mut(14) {
        let mut pred: [i32;2] = [0,0];
        let mut nonz: [i32;2] = [0,0];

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
    }))
  }
}

pub struct BitPumpPanasonic<'a> {
  buffer: &'a [u8],
  pos: usize,
  nbits: u32,
  split: bool,
}

impl<'a> BitPumpPanasonic<'a> {
  pub fn new(src: &'a [u8], split: bool) -> BitPumpPanasonic {
    BitPumpPanasonic {
      buffer: src,
      pos: 0,
      nbits: 0,
      split: split,
    }
  }
}

impl<'a> BitPump for BitPumpPanasonic<'a> {
  fn peek_bits(&mut self, num: u32) -> u32 {
    if num > self.nbits {
      self.nbits += 0x4000 * 8;
      self.pos += 0x4000;
    }
    let mut byte = (self.nbits-num) >> 3 ^ 0x3ff0;
    if self.split {
      byte = (byte + 0x4000 - 0x2008) % 0x4000;
    }
    let bits = LEu16(self.buffer, byte as usize + self.pos - 0x4000) as u32;
    (bits >> ((self.nbits-num) & 7)) & (0x0ffffffffu32 >> (32-num))
  }

  fn consume_bits(&mut self, num: u32) {
    self.nbits -= num;
  }
}
