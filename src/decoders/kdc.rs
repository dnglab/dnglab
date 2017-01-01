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
  fn image(&self) -> Result<Image,String> {
    let camera = try!(self.rawloader.check_supported(&self.tiff));
    let width = fetch_tag!(self.tiff, Tag::KdcWidth).get_usize(0)+80;
    let height = fetch_tag!(self.tiff, Tag::KdcLength).get_usize(0)+70;
    let offset = fetch_tag!(self.tiff, Tag::KdcOffset);
    if offset.count() < 13 {
      panic!("KDC Decoder: Couldn't find the KDC offset");
    }
    let mut off = offset.get_usize(4) + offset.get_usize(12);

    // Offset hardcoding gotten from dcraw
    if camera.find_hint("easyshare_offset_hack") {
      off = if off < 0x15000 {0x15000} else {0x17000};
    }

    let src = &self.buffer[off..];
    let image = decode_12be(src, width, height);
    ok_image(camera, width, height, try!(self.get_wb()), image)
  }
}

impl<'a> KdcDecoder<'a> {
  fn get_wb(&self) -> Result<[f32;4], String> {
    match self.tiff.find_entry(Tag::KdcWB) {
      Some(levels) => {
        if levels.count() != 3 {
          Err("KDC: Levels count is off".to_string())
        } else {
          Ok([levels.get_f32(0), levels.get_f32(1), levels.get_f32(2), NAN])
        }
      },
      None => {
        let levels = fetch_tag!(self.tiff, Tag::KodakWB);
        if levels.count() != 734 && levels.count() != 1502 {
          Err("KDC: Levels count is off".to_string())
        } else {
          let r = BEu16(levels.get_data(), 148) as f32;
          let b = BEu16(levels.get_data(), 150) as f32;
          Ok([r / 256.0, 1.0, b / 256.0, NAN])
        }
      },
    }
  }
}
