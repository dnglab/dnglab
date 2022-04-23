use std::f32::NAN;

use log::debug;
use serde::{Deserialize, Serialize};

use crate::analyze::FormatDump;
use crate::decompressors::ljpeg::*;
use crate::exif::Exif;
use crate::formats::tiff::reader::TiffReader;
use crate::formats::tiff::{Entry, GenericTiffReader, Rational, Value};
use crate::lens::{LensDescription, LensResolver};
use crate::packed::decode_16le;
use crate::pixarray::PixU16;
use crate::tags::{ExifTag, TiffCommonTag};
use crate::Result;
use crate::{alloc_image_ok, RawFile, RawImage, RawLoader};

use super::{ok_image_with_black_white, Camera, Decoder, RawDecodeParams, RawMetadata};

/// 3FR format encapsulation for analyzer
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TfrFormat {
  tiff: GenericTiffReader,
}

#[derive(Debug, Clone)]
pub struct TfrDecoder<'a> {
  camera: Camera,
  #[allow(unused)]
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
}

impl<'a> TfrDecoder<'a> {
  pub fn new(_file: &mut RawFile, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<TfrDecoder<'a>> {
    debug!("3FR decoder choosen");
    let camera = rawloader.check_supported(tiff.root_ifd())?;
    //let makernotes = new_makernote(file, 8).map_err(|ioerr| RawlerError::with_io_error("load 3FR makernotes", &file.path, ioerr))?;
    Ok(TfrDecoder {
      camera,
      tiff,
      rawloader,
      // makernotes,
    })
  }
}

impl<'a> Decoder for TfrDecoder<'a> {
  fn raw_image(&self, file: &mut RawFile, _params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let raw = self.tiff.find_first_ifd_with_tag(TiffCommonTag::WhiteLevel).unwrap();

    let white = match raw.get_entry(TiffCommonTag::WhiteLevel) {
      Some(tag) => tag.force_u16(0),
      None => self.camera.whitelevels[0],
    };

    let black = match raw.get_entry(TiffCommonTag::BlackLevels) {
      Some(tag) => tag.force_u16(0),
      None => self.camera.whitelevels[0],
    };

    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let offset = fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_usize(0);

    let src = file.subview_until_eof(offset as u64).unwrap();

    let image = if self.camera.find_hint("uncompressed") {
      decode_16le(&src, width, height, dummy)
    } else {
      self.decode_compressed(&src, width, height, dummy)?
    };

    //crate::devtools::dump_image_u16(&image.data, width, height, "/tmp/tfrdump.pnm");

    let cpp = 1;
    ok_image_with_black_white(self.camera.clone(), width, height, cpp, self.get_wb()?, black, white, image.into_inner())
  }

  fn format_dump(&self) -> FormatDump {
    FormatDump::Tfr(TfrFormat { tiff: self.tiff.clone() })
  }

  fn raw_metadata(&self, _file: &mut RawFile, _params: RawDecodeParams) -> Result<RawMetadata> {
    let exif = Exif::new(self.tiff.root_ifd())?;
    let mdata = RawMetadata::new_with_lens(&self.camera, exif, self.get_lens_description()?.cloned());
    Ok(mdata)
  }
}

impl<'a> TfrDecoder<'a> {
  /// Get lens description by analyzing TIFF tags and makernotes
  fn get_lens_description(&self) -> Result<Option<&'static LensDescription>> {
    if let Some(exif) = self.tiff.root_ifd().get_sub_ifds(TiffCommonTag::ExifIFDPointer).and_then(|list| list.get(0)) {
      let lens_make = exif.get_entry(ExifTag::LensMake).and_then(|entry| entry.as_string());
      let lens_model = exif.get_entry(ExifTag::LensModel).and_then(|entry| entry.as_string());
      let focal_len = match exif.get_entry(ExifTag::FocalLength) {
        Some(Entry { value: Value::Rational(x), .. }) => x.get(0).cloned(),
        Some(Entry { value: Value::Short(x), .. }) => x.get(0).copied().map(Rational::from),
        _ => None,
      };
      let resolver = LensResolver::new()
        .with_camera(&self.camera)
        .with_lens_make(lens_make)
        .with_lens_model(lens_model)
        .with_focal_len(focal_len)
        .with_mounts(&["x-mount".into()]);
      return Ok(resolver.resolve());
    }
    Ok(None)
  }

  fn get_wb(&self) -> Result<[f32; 4]> {
    let levels = fetch_tiff_tag!(self.tiff, TiffCommonTag::AsShotNeutral);
    assert_eq!(levels.count(), 3);
    Ok([1.0 / levels.force_f32(0), 1.0 / levels.force_f32(1), 1.0 / levels.force_f32(2), NAN])
  }

  fn decode_compressed(&self, src: &[u8], width: usize, height: usize, dummy: bool) -> Result<PixU16> {
    let mut out = alloc_image_ok!(width, height, dummy);
    let decompressor = LjpegDecompressor::new_full(src, true, false)?;
    decompressor.decode(&mut out, 0, width, width, height, dummy)?;
    Ok(PixU16::new_with(out, width, height))
  }
}
