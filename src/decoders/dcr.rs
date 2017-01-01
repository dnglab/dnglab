use decoders::*;
use decoders::tiff::*;
use decoders::basics::*;
use std::f32::NAN;
use itertools::Itertools;
use std::cmp;

#[derive(Debug, Clone)]
pub struct DcrDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: TiffIFD<'a>,
}

impl<'a> DcrDecoder<'a> {
  pub fn new(buf: &'a [u8], tiff: TiffIFD<'a>, rawloader: &'a RawLoader) -> DcrDecoder<'a> {
    DcrDecoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for DcrDecoder<'a> {
  fn image(&self) -> Result<Image,String> {
    let camera = try!(self.rawloader.check_supported(&self.tiff));
    let raw = fetch_ifd!(&self.tiff, Tag::CFAPattern);
    let width = fetch_tag!(raw, Tag::ImageWidth).get_usize(0);
    let height = fetch_tag!(raw, Tag::ImageLength).get_usize(0);
    let offset = fetch_tag!(raw, Tag::StripOffsets).get_usize(0);
    let src = &self.buffer[offset .. self.buffer.len()];

    let linearization = fetch_tag!(self.tiff, Tag::DcrLinearization);
    let curve = {
      let mut points = vec![0 as u16; 1024];
      for i in 0..1024 {
        points[i] = linearization.get_u32(i) as u16;
      }
      LookupTable::new(&points)
    };

    let image = DcrDecoder::decode_kodak65000(src, &curve, width, height);

    ok_image(camera, width, height, try!(self.get_wb()), image)
  }
}

impl<'a> DcrDecoder<'a> {
  fn get_wb(&self) -> Result<[f32;4], String> {
    let levels = fetch_tag!(self.tiff, Tag::DcrWB).get_data();
    Ok([2048.0 / BEu16(levels,40) as f32,
        2048.0 / BEu16(levels,42) as f32,
        2048.0 / BEu16(levels,44) as f32,
        NAN])
  }

  fn decode_kodak65000(buf: &[u8], curve: &LookupTable, width: usize, height: usize) -> Vec<u16> {
    let mut out: Vec<u16> = vec![0; width*height];
    let mut input = ByteStream::new(buf, LITTLE_ENDIAN);

    let mut random: u32 = 0;
    for row in 0..height {
      for col in (0..width).step(256) {
        let mut pred: [u32;2] = [0;2];
        let buf = DcrDecoder::decode_segment(&mut input, cmp::min(256, width-col));
        for (i,val) in buf.iter().enumerate() {
          pred[i & 1] += *val as u32;
          out[row*width+col+i] = curve.dither(pred[i & 1] as u16, &mut random);
        }
      }
    }

    out
  }

  fn decode_segment(input: &mut ByteStream, size: usize) -> Vec<u16> {
    let mut out: Vec<u16> = vec![0; size];

    let mut lens: [usize;256] = [0;256];
    for i in (0..size).step(2) {
      lens[i] = (input.peek_u8() & 15) as usize;
      lens[i+1] = (input.get_u8() >> 4) as usize;
    }

    let mut bitbuf: u64 = 0;
    let mut bits: usize = 0;
    if (size & 7) == 4 {
      bitbuf  = (input.get_u8() as u64) << 8 | (input.get_u8() as u64);
      bits = 16;
    }

    for i in 0..size {
      let len = lens[i];
      if bits < len {
        for j in (0..32).step(8) {
          bitbuf += (input.get_u8() as u64) << (bits+(j^8));
        }
        bits += 32;
      }
      let mut diff = (bitbuf & (0xffff >> (16-len))) as u32;
      bitbuf >>= len;
      bits -= len;
      if len != 0 && (diff & (1 << (len-1))) == 0 {
        diff -= (1 << len) - 1;
      }
      out[i] = diff as u16;
    }

    out
  }
}
