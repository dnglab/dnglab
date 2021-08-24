use std::f32;

use crate::bits::Endian;
use crate::decoders::*;
use crate::formats::tiff::*;
use crate::bits::*;
use crate::packed::*;

pub fn is_mrw(buf: &[u8]) -> bool {
  BEu32(buf,0) == 0x004D524D
}

#[derive(Debug, Clone)]
pub struct MrwDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  data_offset: usize,
  raw_width: usize,
  raw_height: usize,
  packed: bool,
  wb_vals: [u16;4],
  tiff: TiffIFD<'a>,
}

impl<'a> MrwDecoder<'a> {
  pub fn new(buf: &'a [u8], rawloader: &'a RawLoader) -> MrwDecoder<'a> {
    let data_offset: usize = (BEu32(buf, 4) + 8) as usize;
    let mut raw_height: usize = 0;
    let mut raw_width: usize = 0;
    let mut packed = false;
    let mut wb_vals: [u16;4] = [0;4];
    let mut tiffpos: usize = 0;

    let mut currpos: usize = 8;
    // At most we read 20 bytes from currpos so check we don't step outside that
    while currpos+20 < data_offset {
      let tag: u32 = BEu32(buf,currpos);
      let len: u32 = BEu32(buf,currpos+4);
      
      match tag {
        0x505244 => { // PRD
          raw_height = BEu16(buf,currpos+16) as usize;
          raw_width = BEu16(buf,currpos+18) as usize;
          packed = buf[currpos+24] == 12;
        }
        0x574247 => { // WBG
          for i in 0..4 {
            wb_vals[i] = BEu16(buf, currpos+12+i*2);
          }
        }
        0x545457 => { // TTW
          // Base value for offsets needs to be at the beginning of the 
          // TIFF block, not the file
          tiffpos = currpos+8;
        }
        _ => {}
      }
      currpos += (len+8) as usize;
    }

    MrwDecoder { 
      buffer: buf,
      data_offset: data_offset,
      raw_width: raw_width,
      raw_height: raw_height,
      packed: packed,
      wb_vals: wb_vals,
      tiff: TiffIFD::new(&buf[tiffpos..], 8, 0, 0, 0, Endian::Big, &vec![]).unwrap(),
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for MrwDecoder<'a> {
  fn raw_image(&self, _params: RawDecodeParams, dummy: bool) -> Result<RawImage,String> {
    let camera = self.rawloader.check_supported(&self.tiff)?;
    let src = &self.buffer[self.data_offset..];

    let buffer = if self.packed {
      decode_12be(src, self.raw_width, self.raw_height, dummy)
    }
    else {
      decode_12be_unpacked(src, self.raw_width, self.raw_height, dummy)
    };

    let wb_coeffs = if camera.find_hint("swapped_wb") {
      [self.wb_vals[2] as f32,
       self.wb_vals[0] as f32,
       self.wb_vals[1] as f32,
       f32::NAN]
    } else {
      [self.wb_vals[0] as f32,
       self.wb_vals[1] as f32,
       self.wb_vals[3] as f32,
       f32::NAN]
    };

    ok_image(camera, self.raw_width, self.raw_height, wb_coeffs, buffer)
  }
}
