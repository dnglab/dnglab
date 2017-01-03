use decoders::*;
use decoders::tiff::*;
use decoders::basics::*;
use std::f32::NAN;

const NIKON_TREE: [[u8;32];6] = [
  // 12-bit lossy
  [0,1,5,1,1,1,1,1,1,2,0,0,0,0,0,0,5,4,3,6,2,7,1,0,8,9,11,10,12,0,0,0],
  // 12-bit lossy after split
  [0,1,5,1,1,1,1,1,1,2,0,0,0,0,0,0,0x39,0x5a,0x38,0x27,0x16,5,4,3,2,1,0,11,12,12,0,0],
  // 12-bit lossless
  [0,1,4,2,3,1,2,0,0,0,0,0,0,0,0,0,5,4,6,3,7,2,8,1,9,0,10,11,12,0,0,0],
  // 14-bit lossy
  [0,1,4,3,1,1,1,1,1,2,0,0,0,0,0,0,5,6,4,7,8,3,9,2,1,0,10,11,12,13,14,0],
  // 14-bit lossy after split
  [0,1,5,1,1,1,1,1,1,1,2,0,0,0,0,0,8,0x5c,0x4b,0x3a,0x29,7,6,5,4,3,2,1,0,13,14,0],
  // 14-bit lossless
  [0,1,4,2,2,3,1,2,0,0,0,0,0,0,0,0,7,6,8,5,9,4,10,3,11,12,2,0,1,13,14,0],
];

#[derive(Debug, Clone)]
pub struct NefDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: TiffIFD<'a>,
}

impl<'a> NefDecoder<'a> {
  pub fn new(buf: &'a [u8], tiff: TiffIFD<'a>, rawloader: &'a RawLoader) -> NefDecoder<'a> {
    NefDecoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for NefDecoder<'a> {
  fn image(&self) -> Result<Image,String> {
    let camera = try!(self.rawloader.check_supported(&self.tiff));
    let raw = fetch_ifd!(&self.tiff, Tag::CFAPattern);
    let mut width = fetch_tag!(raw, Tag::ImageWidth).get_usize(0);
    let height = fetch_tag!(raw, Tag::ImageLength).get_usize(0);
    let offset = fetch_tag!(raw, Tag::StripOffsets).get_usize(0);
    let src = &self.buffer[offset..];

    let image = if camera.model == "NIKON D100" {
      width = 3040;
      decode_12be_wcontrol(src, width, height)
    } else {
      match fetch_tag!(raw, Tag::Compression).get_usize(0) {
        34713 => try!(self.decode_compressed(width, height)),
        x => return Err(format!("Don't know how to handle compression {}", x).to_string()),
      }
    };

    ok_image(camera, width, height, try!(self.get_wb()), image)
  }
}

impl<'a> NefDecoder<'a> {
  fn get_wb(&self) -> Result<[f32;4], String> {
    if let Some(levels) = self.tiff.find_entry(Tag::NefWB1) {
      let mut version: u32 = 0;
      for i in 0..4 {
        version = (version << 4) + (levels.get_data()[i]-b'0') as u32;
      }
      match version {
        0x100 =>  Ok([levels.get_force_u16(36) as f32, levels.get_force_u16(38) as f32,
                      levels.get_force_u16(37) as f32, NAN]),
        0x103 =>  Ok([levels.get_force_u16(10) as f32, levels.get_force_u16(11) as f32,
                      levels.get_force_u16(12) as f32, NAN]),
        x => Err(format!("NEF: Don't know about WB version 0x{:x}", x).to_string()),
      }
    } else {
      Err("NEF: Don't know how to fetch WB".to_string())
    }
  }

  fn decode_compressed(&self, width: usize, height: usize) -> Result<Vec<u16>, String> {
    let mut out = vec![0 as u16; width * height];
    Ok(out)
  }
}
