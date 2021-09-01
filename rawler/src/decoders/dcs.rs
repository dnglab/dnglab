use std::f32::NAN;

use crate::RawImage;
use crate::bits::LookupTable;
use crate::decoders::*;
use crate::formats::tiff_legacy::*;
use crate::packed::decode_8bit_wtable;
use crate::tags::LegacyTiffRootTag;

#[derive(Debug, Clone)]
pub struct DcsDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: LegacyTiffIFD<'a>,
}

impl<'a> DcsDecoder<'a> {
  pub fn new(buf: &'a [u8], tiff: LegacyTiffIFD<'a>, rawloader: &'a RawLoader) -> DcsDecoder<'a> {
    DcsDecoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for DcsDecoder<'a> {
  fn raw_image(&self, _params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let camera = self.rawloader.check_supported(&self.tiff)?;
    let data = self.tiff.find_ifds_with_tag(LegacyTiffRootTag::StripOffsets);
    let raw = data.iter().find(|&&ifd| {
      ifd.find_entry(LegacyTiffRootTag::ImageWidth).unwrap().get_u32(0) > 1000
    }).unwrap();
    let width = fetch_tag!(raw, LegacyTiffRootTag::ImageWidth).get_usize(0);
    let height = fetch_tag!(raw, LegacyTiffRootTag::ImageLength).get_usize(0);
    let offset = fetch_tag!(raw, LegacyTiffRootTag::StripOffsets).get_usize(0);
    let src = &self.buffer[offset..];
    let linearization = fetch_tag!(self.tiff, LegacyTiffRootTag::GrayResponse);
    let table = {
      let mut t: [u16;256] = [0;256];
      for i in 0..256 {
        t[i] = linearization.get_u32(i) as u16;
      }
      LookupTable::new(&t)
    };

    let image = decode_8bit_wtable(src, &table, width, height, dummy);
    ok_image(camera, width, height, [NAN,NAN,NAN,NAN], image)
  }
}
