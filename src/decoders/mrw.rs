use decoders::*;
use decoders::basics::*;

pub fn is_mrw(buf: &[u8]) -> bool {
  BEu32(buf,0) == 0x004D524D
}

pub struct MrwDecoder<'a> {
  buffer: &'a [u8],
  data_offset: usize,
  raw_width: u16,
  raw_height: u16,
  packed: bool,
  wb_coeffs: [f32;4],
}

impl<'a> MrwDecoder<'a> {
  pub fn new(buf: &[u8]) -> MrwDecoder {
    let data_offset: usize = (BEu32(buf, 4) + 8) as usize;
    let mut raw_height: u16 = 0;
    let mut raw_width: u16 = 0;
    let mut packed = false;
    let mut wb_coeffs: [f32;4] = [0.0,0.0,0.0,0.0];

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
            wb_coeffs[i] = (BEu16(buf, currpos+12+i*2)) as f32;
          }
        }
        0x545457 => { // TTW
          // Base value for offsets needs to be at the beginning of the 
          // TIFF block, not the file
//          FileMap *f = new FileMap(mFile, currpos+8);
//          if (little == getHostEndianness())
//            tiff_meta = new TiffIFDBE(f, 8);
//          else
//            tiff_meta = new TiffIFD(f, 8);
//          delete f;
//          break;
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
      wb_coeffs: wb_coeffs,
    }
  }
}

impl<'a> Decoder for MrwDecoder<'a> {
  fn make(&self) -> String {
    "Minolta".to_string()
  }

  fn model(&self) -> String {
    "SomeModel".to_string()
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

    Image {
      width: self.raw_width as u32,
      height: self.raw_height as u32,
      wb_coeffs: self.wb_coeffs,
      data: buffer.into_boxed_slice(),
    }
  }
}
