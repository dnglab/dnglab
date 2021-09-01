use crate::RawImage;
use crate::decoders::*;
use crate::formats::tiff_legacy::*;
use crate::packed::decode_12be;
use crate::tags::LegacyTiffRootTag;
use std::f32::NAN;

#[derive(Debug, Clone)]
pub struct MefDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: LegacyTiffIFD<'a>,
}

impl<'a> MefDecoder<'a> {
  pub fn new(buf: &'a [u8], tiff: LegacyTiffIFD<'a>, rawloader: &'a RawLoader) -> MefDecoder<'a> {
    MefDecoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for MefDecoder<'a> {
  fn raw_image(&self, _params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let camera = self.rawloader.check_supported(&self.tiff)?;
    let raw = fetch_ifd!(&self.tiff, LegacyTiffRootTag::CFAPattern);
    let width = fetch_tag!(raw, LegacyTiffRootTag::ImageWidth).get_usize(0);
    let height = fetch_tag!(raw, LegacyTiffRootTag::ImageLength).get_usize(0);
    let offset = fetch_tag!(raw, LegacyTiffRootTag::StripOffsets).get_usize(0);
    let src = &self.buffer[offset..];

    let image = decode_12be(src, width, height, dummy);
    ok_image(camera, width, height, [NAN,NAN,NAN,NAN], image)
  }
}
