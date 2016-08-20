use decoders::*;
use decoders::tiff::*;
use decoders::basics::*;
use std::f32;

pub fn is_mrw(buf: &[u8]) -> bool {
  BEu32(buf,0) == 0x004D524D
}

pub struct MrwDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  data_offset: usize,
  raw_width: u16,
  raw_height: u16,
  packed: bool,
  wb_vals: [u16;4],
  tiff: TiffIFD<'a>,
}

impl<'a> MrwDecoder<'a> {
  pub fn new(buf: &'a [u8], rawloader: &'a RawLoader) -> MrwDecoder<'a> {
    let data_offset: usize = (BEu32(buf, 4) + 8) as usize;
    let mut raw_height: u16 = 0;
    let mut raw_width: u16 = 0;
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
          raw_height = BEu16(buf,currpos+16);
          raw_width = BEu16(buf,currpos+18);
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
      tiff: TiffIFD::new(&buf[tiffpos .. buf.len()], 8, 0),
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for MrwDecoder<'a> {
  fn identify(&self) -> Result<&Camera, String> {
    let make = match self.tiff.find_entry(Tag::MAKE) {
      Some(val) => val,
      None => return Err("MRW: Couldn't find the maker".to_string()),
    }.get_str();
    let model = match self.tiff.find_entry(Tag::MODEL) {
      Some(val) => val,
      None => return Err("MRW: Couldn't find the model".to_string()),
    }.get_str();

    self.rawloader.check_supported(make, model)
  }

  fn image(&self) -> Image {
    let src = &self.buffer[self.data_offset .. self.buffer.len()];
    let w = self.raw_width as usize;
    let h = self.raw_height as usize;

    let buffer = if self.packed {
      decode_12be(&src, w, h)
    }
    else {
      decode_12be_unpacked(&src, w, h)
    };

    let swapped_wb = false;

    let wb_coeffs = if swapped_wb {
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

    Image {
      width: self.raw_width as u32,
      height: self.raw_height as u32,
      wb_coeffs: wb_coeffs,
      data: buffer.into_boxed_slice(),
    }
  }
}
