use std::f32::NAN;

use crate::RawImage;
use crate::decoders::*;
use crate::formats::tiff_legacy::*;
use crate::bits::*;
use crate::packed::*;
use crate::tags::LegacyTiffRootTag;

#[derive(Debug, Clone)]
pub struct ErfDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: LegacyTiffIFD<'a>,
}

impl<'a> ErfDecoder<'a> {
  pub fn new(buf: &'a [u8], tiff: LegacyTiffIFD<'a>, rawloader: &'a RawLoader) -> ErfDecoder<'a> {
    ErfDecoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for ErfDecoder<'a> {
  fn raw_image(&self, _params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let camera = self.rawloader.check_supported(&self.tiff)?;
    let raw = fetch_ifd!(&self.tiff, LegacyTiffRootTag::CFAPattern);
    let width = fetch_tag!(raw, LegacyTiffRootTag::ImageWidth).get_usize(0);
    let height = fetch_tag!(raw, LegacyTiffRootTag::ImageLength).get_usize(0);
    let offset = fetch_tag!(raw, LegacyTiffRootTag::StripOffsets).get_usize(0);
    let src = &self.buffer[offset..];

    let image = decode_12be_wcontrol(src, width, height, dummy);
    ok_image(camera, width, height, self.get_wb()?, image)
  }
}

impl<'a> ErfDecoder<'a> {
  fn get_wb(&self) -> Result<[f32;4]> {
    let levels = fetch_tag!(self.tiff, LegacyTiffRootTag::EpsonWB);
    if levels.count() != 256 {
      Err(RawlerError::General("ERF: Levels count is off".to_string()))
    } else {
      let r = BEu16(levels.get_data(), 48) as f32;
      let b = BEu16(levels.get_data(), 50) as f32;
      Ok([r * 508.0 * 1.078 / 65536.0, 1.0, b * 382.0 * 1.173 / 65536.0, NAN])
    }
  }
}
