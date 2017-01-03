use decoders::*;
use decoders::tiff::*;
use decoders::basics::*;
use std::f32::NAN;

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
      return Err("NEF is still unfinished".to_string())
    };

    ok_image(camera, width, height, try!(self.get_wb()), image)
  }
}

impl<'a> NefDecoder<'a> {
  fn get_wb(&self) -> Result<[f32;4], String> {
    if let Some(levels) = self.tiff.find_entry(Tag::NefWB1) {
      Ok([levels.get_force_u16(36) as f32, levels.get_force_u16(38) as f32,
          levels.get_force_u16(37) as f32, NAN])
    } else {
      Err("Don't know how to fetch WB".to_string())
    }
  }
}
