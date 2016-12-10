use decoders::*;
use decoders::tiff::*;
use decoders::basics::*;
use std::f32::NAN;

#[derive(Debug, Clone)]
pub struct ErfDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: TiffIFD<'a>,
}

impl<'a> ErfDecoder<'a> {
  pub fn new(buf: &'a [u8], tiff: TiffIFD<'a>, rawloader: &'a RawLoader) -> ErfDecoder<'a> {
    ErfDecoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for ErfDecoder<'a> {
  fn identify(&self) -> Result<&Camera, String> {
    let make = fetch_tag!(self.tiff, Tag::Make, "ERF: Couldn't find Make").get_str();
    let model = fetch_tag!(self.tiff, Tag::Model, "ERF: Couldn't find Model").get_str();
    self.rawloader.check_supported(make, model)
  }

  fn image(&self) -> Result<Image,String> {
    let camera = try!(self.identify());
    let data = self.tiff.find_ifds_with_tag(Tag::StripOffsets);
    let raw = data[1];
    let width = fetch_tag!(raw, Tag::ImageWidth, "ERF: Couldn't find width").get_u32(0);
    let height = fetch_tag!(raw, Tag::ImageLength, "ERF: Couldn't find height").get_u32(0);
    let offset = fetch_tag!(raw, Tag::StripOffsets, "ERF: Couldn't find offset").get_u32(0) as usize;
    let src = &self.buffer[offset .. self.buffer.len()];

    let image = decode_12be_wcontrol(src, width as usize, height as usize);
    ok_image(camera, width, height, try!(self.get_wb()), image)
  }
}

impl<'a> ErfDecoder<'a> {
  fn get_wb(&self) -> Result<[f32;4], String> {
    let levels = fetch_tag!(self.tiff, Tag::EpsonWB, "ERF: No levels");
    if levels.count() != 256 {
      Err("ERF: Levels count is off".to_string())
    } else {
      let r = BEu16(levels.get_data(), 48) as f32;
      let b = BEu16(levels.get_data(), 50) as f32;
      Ok([r * 508.0 * 1.078 / 65536.0, 1.0, b * 382.0 * 1.173 / 65536.0, NAN])
    }
  }
}
