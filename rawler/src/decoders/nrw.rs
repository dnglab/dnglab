use std::f32::NAN;

use crate::decoders::*;
use crate::decoders::tiff::*;
use crate::decoders::basics::*;

#[derive(Debug, Clone)]
pub struct NrwDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: TiffIFD<'a>,
}

impl<'a> NrwDecoder<'a> {
  pub fn new(buf: &'a [u8], tiff: TiffIFD<'a>, rawloader: &'a RawLoader) -> NrwDecoder<'a> {
    NrwDecoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for NrwDecoder<'a> {
  fn image(&self, dummy: bool) -> Result<RawImage,String> {
    let camera = self.rawloader.check_supported(&self.tiff)?;
    let data = self.tiff.find_ifds_with_tag(Tag::CFAPattern);
    let raw = data.iter().find(|&&ifd| {
      ifd.find_entry(Tag::ImageWidth).unwrap().get_u32(0) > 1000
    }).unwrap();
    let width = fetch_tag!(raw, Tag::ImageWidth).get_usize(0);
    let height = fetch_tag!(raw, Tag::ImageLength).get_usize(0);
    let offset = fetch_tag!(raw, Tag::StripOffsets).get_usize(0);
    let src = &self.buffer[offset..];

    let image = if camera.find_hint("coolpixsplit") {
      decode_12be_interlaced_unaligned(src, width, height, dummy)
    } else if camera.find_hint("msb32") {
      decode_12be_msb32(src, width, height, dummy)
    } else if camera.find_hint("unpacked") {
      decode_16be(src, width, height, dummy)
    } else {
      decode_12be(src, width, height, dummy)
    };

    let wb = self.get_wb(&camera)?;
    ok_image(camera, width, height, wb, image)
  }
}

impl<'a> NrwDecoder<'a> {
  fn get_wb(&self, cam: &Camera) -> Result<[f32;4], String> {
    if cam.find_hint("nowb") {
      Ok([NAN,NAN,NAN,NAN])
    } else if let Some(levels) = self.tiff.find_entry(Tag::NefWB0) {
      Ok([levels.get_f32(0), 1.0, levels.get_f32(1), NAN])
    } else if let Some(levels) = self.tiff.find_entry(Tag::NrwWB) {
      let data = levels.get_data();
      if data[0..3] == b"NRW"[..] {
        let offset = if data[4..8] == b"0100"[..] {
          1556
        } else {
          56
        };

        Ok([(LEu32(data, offset) << 2) as f32,
            (LEu32(data, offset+4) + LEu32(data, offset+8)) as f32,
            (LEu32(data, offset+12) << 2) as f32,
            NAN])
      } else {
        Ok([BEu16(data,1248) as f32, 256.0, BEu16(data,1250) as f32, NAN])
      }
    } else {
      Err("NRW: Don't know how to fetch WB".to_string())
    }
  }
}
