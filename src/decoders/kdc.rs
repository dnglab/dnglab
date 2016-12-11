use decoders::*;
use decoders::tiff::*;
use decoders::basics::*;
use std::f32::NAN;

#[derive(Debug, Clone)]
pub struct KdcDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: TiffIFD<'a>,
}

impl<'a> KdcDecoder<'a> {
  pub fn new(buf: &'a [u8], tiff: TiffIFD<'a>, rawloader: &'a RawLoader) -> KdcDecoder<'a> {
    KdcDecoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for KdcDecoder<'a> {
  fn identify(&self) -> Result<&Camera, String> {
    let make = fetch_tag!(self.tiff, Tag::Make, "KDC: Couldn't find Make").get_str();
    let model = fetch_tag!(self.tiff, Tag::Model, "KDC: Couldn't find Model").get_str();
    self.rawloader.check_supported(make, model)
  }

  fn image(&self) -> Result<Image,String> {
    let camera = try!(self.identify());
    let width = fetch_tag!(self.tiff, Tag::KdcWidth, "KDC: Couldn't find width").get_u32(0)+80;
    let height = fetch_tag!(self.tiff, Tag::KdcLength, "KDC: Couldn't find height").get_u32(0)+70;
    let offset = fetch_tag!(self.tiff, Tag::KdcOffset, "KDC: Couldn't find offset");
    if offset.count() < 13 {
      panic!("KDC Decoder: Couldn't find the KDC offset");
    }
    let mut off = (offset.get_u32(4) + offset.get_u32(12)) as usize;

    // Offset hardcoding gotten from dcraw
    if camera.find_hint("easyshare_offset_hack") {
      off = if off < 0x15000 {0x15000} else {0x17000};
    }

    let src = &self.buffer[off..];
    let image = decode_12be(src, width as usize, height as usize);
    ok_image(camera, width, height, try!(self.get_wb()), image)
  }
}

impl<'a> KdcDecoder<'a> {
  fn get_wb(&self) -> Result<[f32;4], String> {
    let levels = fetch_tag!(self.tiff, Tag::KodakWB, "KDC: No levels");
    if levels.count() != 734 && levels.count() != 1502 {
      Err("KDC: Levels count is off".to_string())
    } else {
      let r = BEu16(levels.get_data(), 148) as f32;
      let b = BEu16(levels.get_data(), 150) as f32;
      Ok([r / 256.0, 1.0, b / 256.0, NAN])
    }
  }
}
