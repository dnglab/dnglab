use std::f32::NAN;

use crate::RawImage;
use crate::alloc_image;
use crate::decoders::*;
use crate::formats::tiff_legacy::*;
use crate::bits::*;
use crate::packed::decode_12be;
use crate::tags::LegacyTiffRootTag;

#[derive(Debug, Clone)]
pub struct KdcDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: LegacyTiffIFD<'a>,
}

impl<'a> KdcDecoder<'a> {
  pub fn new(buf: &'a [u8], tiff: LegacyTiffIFD<'a>, rawloader: &'a RawLoader) -> KdcDecoder<'a> {
    KdcDecoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for KdcDecoder<'a> {
  fn raw_image(&self, _params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let camera = self.rawloader.check_supported(&self.tiff)?;

    if camera.model == "Kodak DC120 ZOOM Digital Camera" {
      let width = 848;
      let height = 976;
      let raw = self.tiff.find_ifds_with_tag(LegacyTiffRootTag::CFAPattern)[0];
      let off = fetch_tag!(raw, LegacyTiffRootTag::StripOffsets).get_usize(0);
      let src = &self.buffer[off..];
      let image = match fetch_tag!(raw, LegacyTiffRootTag::Compression).get_usize(0) {
        1 => Self::decode_dc120(src, width, height, dummy),
        c => return Err(RawlerError::Unsupported(format!("KDC: DC120: Don't know how to handle compression type {}", c).to_string()))
      };

      return ok_image(camera, width, height, [NAN, NAN, NAN, NAN], image)
    }

    let width = fetch_tag!(self.tiff, LegacyTiffRootTag::KdcWidth).get_usize(0)+80;
    let height = fetch_tag!(self.tiff, LegacyTiffRootTag::KdcLength).get_usize(0)+70;
    let offset = fetch_tag!(self.tiff, LegacyTiffRootTag::KdcOffset);
    if offset.count() < 13 {
      panic!("KDC Decoder: Couldn't find the KDC offset");
    }
    let mut off = offset.get_usize(4) + offset.get_usize(12);

    // Offset hardcoding gotten from dcraw
    if camera.find_hint("easyshare_offset_hack") {
      off = if off < 0x15000 {0x15000} else {0x17000};
    }

    let src = &self.buffer[off..];
    let image = decode_12be(src, width, height, dummy);

    ok_image(camera, width, height, self.get_wb()?, image)
  }
}

impl<'a> KdcDecoder<'a> {
  fn get_wb(&self) -> Result<[f32;4]> {
    match self.tiff.find_entry(LegacyTiffRootTag::KdcWB) {
      Some(levels) => {
        if levels.count() != 3 {
          Err(RawlerError::General("KDC: Levels count is off".to_string()))
        } else {
          Ok([levels.get_f32(0), levels.get_f32(1), levels.get_f32(2), NAN])
        }
      },
      None => {
        let levels = fetch_tag!(self.tiff, LegacyTiffRootTag::KodakWB);
        if levels.count() != 734 && levels.count() != 1502 {
          Err(RawlerError::General("KDC: Levels count is off".to_string()))
        } else {
          let r = BEu16(levels.get_data(), 148) as f32;
          let b = BEu16(levels.get_data(), 150) as f32;
          Ok([r / 256.0, 1.0, b / 256.0, NAN])
        }
      },
    }
  }

  pub(crate) fn decode_dc120(src: &[u8], width: usize, height: usize, dummy: bool) -> Vec<u16> {
    let mut out = alloc_image!(width, height, dummy);

    let mul: [usize;4] = [162, 192, 187,  92];
    let add: [usize;4] = [  0, 636, 424, 212];
    for row in 0..height {
      let shift = row * mul[row & 3] + add[row & 3];
      for col in 0..width {
        out[row*width+col] = src[row*width + ((col + shift) % 848)] as u16;
      }
    }

    out
  }
}
