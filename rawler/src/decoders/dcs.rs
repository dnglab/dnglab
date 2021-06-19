use std::f32::NAN;

use crate::RawImage;
use crate::bits::LookupTable;
use crate::decoders::*;
use crate::formats::tiff::*;
use crate::packed::decode_8bit_wtable;
use crate::tags::TiffRootTag;

#[derive(Debug, Clone)]
pub struct DcsDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: TiffIFD<'a>,
}

impl<'a> DcsDecoder<'a> {
  pub fn new(buf: &'a [u8], tiff: TiffIFD<'a>, rawloader: &'a RawLoader) -> DcsDecoder<'a> {
    DcsDecoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for DcsDecoder<'a> {
  fn raw_image(&self, dummy: bool) -> Result<RawImage,String> {
    let camera = self.rawloader.check_supported(&self.tiff)?;
    let data = self.tiff.find_ifds_with_tag(TiffRootTag::StripOffsets);
    let raw = data.iter().find(|&&ifd| {
      ifd.find_entry(TiffRootTag::ImageWidth).unwrap().get_u32(0) > 1000
    }).unwrap();
    let width = fetch_tag!(raw, TiffRootTag::ImageWidth).get_usize(0);
    let height = fetch_tag!(raw, TiffRootTag::ImageLength).get_usize(0);
    let offset = fetch_tag!(raw, TiffRootTag::StripOffsets).get_usize(0);
    let src = &self.buffer[offset..];
    let linearization = fetch_tag!(self.tiff, TiffRootTag::GrayResponse);
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
