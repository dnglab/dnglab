use decoders::*;
use decoders::tiff::*;
use decoders::basics::*;
use std::f32::NAN;

#[derive(Debug, Clone)]
pub struct IiqDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: TiffIFD<'a>,
}

impl<'a> IiqDecoder<'a> {
  pub fn new(buf: &'a [u8], tiff: TiffIFD<'a>, rawloader: &'a RawLoader) -> IiqDecoder<'a> {
    IiqDecoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for IiqDecoder<'a> {
  fn image(&self) -> Result<Image,String> {
    let camera = try!(self.rawloader.check_supported(&self.tiff));

    let off = LEu32(self.buffer, 16) as usize + 8;
    let entries = LEu32(self.buffer, off);
    let mut pos = 8;

    let mut wb_offset: usize = 0;
    let mut width: usize = 0;
    let mut height: usize = 0;
    let mut data_offset: usize = 0;
    let mut strip_offset: usize = 0;
    let mut black: u16 = 0;
    for _ in 0..entries {
      let tag = LEu32(self.buffer, off+pos);
      let data = LEu32(self.buffer, off+pos+12) as usize;
      pos += 16;
      match tag {
        0x107 => wb_offset = data+8,
        0x108 => width = data,
        0x109 => height = data,
        0x10f => data_offset = data+8,
        0x21c => strip_offset = data+8,
        0x21d => black = (data>>2) as u16,
        _ => {},
      }
    }

    if width <= 0 || height <= 0 {
      return Err("IIQ: couldn't find width and height".to_string())
    }

    let image = self.decode_compressed(data_offset, strip_offset, width, height);

    ok_image_with_blacklevels(camera, width, height, try!(self.get_wb(wb_offset)), [black, black, black, black], image)
  }
}

impl<'a> IiqDecoder<'a> {
  fn get_wb(&self, wb_offset: usize) -> Result<[f32;4], String> {
    Ok([LEf32(self.buffer, wb_offset),
        LEf32(self.buffer, wb_offset+4),
        LEf32(self.buffer, wb_offset+8), NAN])
  }

  fn decode_compressed(&self, data_offset: usize, strip_offset: usize, width: usize, height: usize) -> Vec<u16>{
    let mut out = vec![0 as u16; width*height];

    let lens: [u32; 10] = [8,7,6,9,11,10,5,12,14,13];

    for row in 0..height {
      let offset = data_offset + LEu32(self.buffer, strip_offset+row*4) as usize;
      let mut pump = BitPumpMSB32::new(&self.buffer[offset..]);
      let mut pred = [0 as u32; 2];
      let mut len = [0 as u32; 2];
      for col in 0..width {
        if col >= (width & 0xfffffff8) {
          len[0] = 14;
          len[1] = 14;
        } else if col&7 == 0 {
          for i in 0..2 {
            let mut j: usize = 0;
            while j < 5 && pump.get_bits(1) == 0 {j += 1}
            if j > 0 {
              len[i] = lens[(j-1)*2 + pump.get_bits(1) as usize];
            }
          }
        }
        let i = len[col & 1];
        pred[col & 1] = if i == 14 {
          pump.get_bits(16)
        } else {
          pred[col & 1] + pump.get_bits(i) + 1 - (1 << (i-1))
        };
        out[row*width+col] = pred[col & 1] as u16;
      }
    }

    out
  }
}
