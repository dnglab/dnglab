use std::f32::NAN;

use crate::alloc_image_ok;
use crate::decoders::*;
use crate::formats::tiff_legacy::*;
use crate::decompressors::ljpeg::*;
use crate::packed::*;

#[derive(Debug, Clone)]
pub struct TfrDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: LegacyTiffIFD<'a>,
}

impl<'a> TfrDecoder<'a> {
  pub fn new(buf: &'a [u8], tiff: LegacyTiffIFD<'a>, rawloader: &'a RawLoader) -> TfrDecoder<'a> {
    TfrDecoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for TfrDecoder<'a> {
  fn raw_image(&self, _params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let camera = self.rawloader.check_supported_old(&self.tiff)?;
    let raw = fetch_ifd!(&self.tiff, LegacyTiffRootTag::WhiteLevel);
    let width = fetch_tag!(raw, LegacyTiffRootTag::ImageWidth).get_usize(0);
    let height = fetch_tag!(raw, LegacyTiffRootTag::ImageLength).get_usize(0);
    let offset = fetch_tag!(raw, LegacyTiffRootTag::StripOffsets).get_usize(0);
    let src = &self.buffer[offset..];

    let image = if camera.find_hint("uncompressed") {
      decode_16le(src, width, height, dummy)
    } else {
      self.decode_compressed(src, width, height, dummy)?
    };

    ok_image(camera, width, height, self.get_wb()?, image)
  }
}

impl<'a> TfrDecoder<'a> {
  fn get_wb(&self) -> Result<[f32;4]> {
    let levels = fetch_tag!(self.tiff, LegacyTiffRootTag::AsShotNeutral);
    Ok([1.0/levels.get_f32(0),1.0/levels.get_f32(1),1.0/levels.get_f32(2),NAN])
  }

  fn decode_compressed(&self, src: &[u8], width: usize, height: usize, dummy: bool) -> Result<Vec<u16>> {
    let mut out = alloc_image_ok!(width, height, dummy);
    let decompressor = LjpegDecompressor::new_full(src, true, false)?;
    decompressor.decode(&mut out, 0, width, width, height, dummy)?;
    Ok(out)
  }
}
