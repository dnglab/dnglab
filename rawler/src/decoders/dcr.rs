use std::f32::NAN;
use std::cmp;

use crate::RawImage;
use crate::alloc_image;
use crate::bits::Endian;
use crate::decoders::*;
use crate::pumps::ByteStream;
use crate::formats::tiff::*;
use crate::bits::*;
use crate::tags::TiffRootTag;

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
  fn raw_image(&self, dummy: bool) -> Result<RawImage,String> {
    let camera = self.rawloader.check_supported(&self.tiff)?;
    let raw = fetch_ifd!(&self.tiff, TiffRootTag::CFAPattern);
    let width = fetch_tag!(raw, TiffRootTag::ImageWidth).get_usize(0);
    let height = fetch_tag!(raw, TiffRootTag::ImageLength).get_usize(0);
    let offset = fetch_tag!(raw, TiffRootTag::StripOffsets).get_usize(0);
    let src = &self.buffer[offset..];

    let linearization = fetch_tag!(self.tiff, TiffRootTag::DcrLinearization);
    let curve = {
      let mut points = Vec::new();
      for i in 0..linearization.count() {
        points.push(linearization.get_u32(i as usize) as u16);
      }
      LookupTable::new(&points)
    };

    let image = DcrDecoder::decode_kodak65000(src, &curve, width, height, dummy);

    ok_image(camera, width, height, self.get_wb()?, image)
  }
}

impl<'a> DcrDecoder<'a> {
  fn get_wb(&self) -> Result<[f32;4], String> {
    let dcrwb = fetch_tag!(self.tiff, TiffRootTag::DcrWB);
    if dcrwb.count() >= 46 {
      let levels = dcrwb.get_data();
      Ok([2048.0 / BEu16(levels,40) as f32,
          2048.0 / BEu16(levels,42) as f32,
          2048.0 / BEu16(levels,44) as f32,
          NAN])
    } else {
      Ok([NAN,NAN,NAN,NAN])
    }
  }

  pub(crate) fn decode_kodak65000(buf: &[u8], curve: &LookupTable, width: usize, height: usize, dummy: bool) -> Vec<u16> {
    let mut out: Vec<u16> = alloc_image!(width, height, dummy);
    let mut input = ByteStream::new(buf, Endian::Little);

    let mut random: u32 = 0;
    for row in 0..height {
      for col in (0..width).step_by(256) {
        let mut pred: [i32;2] = [0;2];
        let buf = DcrDecoder::decode_segment(&mut input, cmp::min(256, width-col));
        for (i,val) in buf.iter().enumerate() {
          pred[i & 1] += *val;
          if pred[i & 1] < 0 {
            panic!("Found a negative pixel!");
          }
          out[row*width+col+i] = curve.dither(pred[i & 1] as u16, &mut random);
        }
      }
    }

    out
  }

  fn decode_segment(input: &mut ByteStream, size: usize) -> Vec<i32> {
    let mut out: Vec<i32> = vec![0; size];

    let mut lens: [usize;256] = [0;256];
    for i in (0..size).step_by(2) {
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
        for j in (0..32).step_by(8) {
          bitbuf += (input.get_u8() as u64) << (bits+(j^8));
        }
        bits += 32;
      }
      out[i] = (bitbuf & (0xffff >> (16-len))) as i32;
      bitbuf >>= len;
      bits -= len;
      if len != 0 && (out[i] & (1 << (len-1))) == 0 {
        out[i] -= (1 << len) - 1;
      }
    }

    out
  }
}
