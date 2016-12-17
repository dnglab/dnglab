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
    let camera = try!(self.rawloader.check_supported(&self.tiff));
    let data = self.tiff.find_ifds_with_tag(Tag::StripOffsets);
    let raw = data[0];
    let width = fetch_tag!(raw, Tag::PanaWidth).get_u32(0) as usize;
    let height = fetch_tag!(raw, Tag::PanaLength).get_u32(0) as usize;
    let offset = fetch_tag!(raw, Tag::StripOffsets).get_u32(0) as usize;
    let src = &self.buffer[offset..];

    let image = if src.len() >= width*height*2 {
      decode_12le_unpacked_left_aligned(src, width, height)
    } else if src.len() >= width*height*3/2 {
      decode_12le_wcontrol(src, width, height)
    } else {
      return Err("Don't know how to decode compressed".to_string());
    };

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
}
