use image::{DynamicImage, ImageBuffer, Rgb};
use log::debug;
use serde::{Deserialize, Serialize};

use crate::analyze::FormatDump;
use crate::decompressors::ljpeg::*;
use crate::exif::Exif;
use crate::formats::tiff::reader::TiffReader;
use crate::formats::tiff::{Entry, GenericTiffReader, Rational, Value};
use crate::imgop::{Dim2, Rect};
use crate::lens::{LensDescription, LensResolver};
use crate::packed::decode_16le;
use crate::pixarray::PixU16;
use crate::rawsource::RawSource;
use crate::tags::{DngTag, ExifTag, TiffCommonTag};
use crate::{RawImage, RawLoader, alloc_image_ok};
use crate::{RawlerError, Result};

use super::{BlackLevel, CFAConfig, Camera, Decoder, FormatHint, RawDecodeParams, RawMetadata, RawPhotometricInterpretation, WhiteLevel};

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
  pub fn new(_file: &RawSource, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<TfrDecoder<'a>> {
    debug!("3FR decoder choosen");
    let camera = rawloader.check_supported(tiff.root_ifd())?;
    //let makernotes = new_makernote(file, 8).map_err(|ioerr| RawlerError::with_io_error("load 3FR makernotes", file.path(), ioerr))?;
    Ok(TfrDecoder {
      camera,
      tiff,
      rawloader,
      // makernotes,
    })
  }
}

impl<'a> Decoder for TfrDecoder<'a> {
  fn raw_image(&self, file: &RawSource, _params: &RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let raw = self
      .tiff
      .find_first_ifd_with_tag(TiffCommonTag::WhiteLevel)
      .ok_or_else(|| RawlerError::DecoderFailed(format!("Failed to find a IFD with WhilteLevel tag")))?;

    let whitelevel = raw
      .get_entry(TiffCommonTag::WhiteLevel)
      .map(|tag| WhiteLevel::new(vec![tag.force_u16(0) as u32]));

    let blacklevel = raw
      .get_entry(TiffCommonTag::BlackLevels)
      .map(|tag| BlackLevel::new(&[tag.force_u16(0)], 1, 1, 1));

    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let offset = fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_usize(0);

    let src = file.subview_until_eof(offset as u64)?;

    let image = if self.camera.find_hint("uncompressed") {
      decode_16le(src, width, height, dummy)
    } else {
      self.decode_compressed(src, width, height, dummy)?
    };

    let crop = Rect::from_tiff(raw).or_else(|| self.camera.crop_area.map(|area| Rect::new_with_borders(Dim2::new(width, height), &area)));

    //crate::devtools::dump_image_u16(&image.data, width, height, "/tmp/tfrdump.pnm");

    let cpp = 1;
    let photometric = RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&self.camera));
    let mut img = RawImage::new(self.camera.clone(), image, cpp, self.get_wb()?, photometric, blacklevel, whitelevel, dummy);

    img.crop_area = crop;

    Ok(img)
  }

  fn full_image(&self, file: &RawSource, params: &RawDecodeParams) -> Result<Option<DynamicImage>> {
    if params.image_index != 0 {
      return Ok(None);
    }
    let root_ifd = &self.tiff.root_ifd();
    let buf = root_ifd
      .singlestrip_data_rawsource(file)
      .map_err(|e| RawlerError::DecoderFailed(format!("Failed to get strip data: {}", e)))?;
    let compression = root_ifd.get_entry(TiffCommonTag::Compression).ok_or("Missing tag")?.force_usize(0);
    let width = fetch_tiff_tag!(root_ifd, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(root_ifd, TiffCommonTag::ImageLength).force_usize(0);
    if compression == 1 {
      Ok(Some(DynamicImage::ImageRgb8(
        ImageBuffer::<Rgb<u8>, Vec<u8>>::from_raw(width as u32, height as u32, buf.to_vec()).unwrap(),
      )))
    } else {
      let img = image::load_from_memory_with_format(buf, image::ImageFormat::Jpeg)
        .map_err(|err| RawlerError::DecoderFailed(format!("Failed to read JPEG: {:?}", err)))?;
      Ok(Some(img))
    }
  }

  fn format_dump(&self) -> FormatDump {
    FormatDump::Tfr(TfrFormat { tiff: self.tiff.clone() })
  }

  fn format_hint(&self) -> FormatHint {
    FormatHint::TFR
  }

  fn raw_metadata(&self, _file: &RawSource, _params: &RawDecodeParams) -> Result<RawMetadata> {
    let exif = Exif::new(self.tiff.root_ifd())?;
    let mut mdata = RawMetadata::new_with_lens(&self.camera, exif, self.get_lens_description()?.cloned());
    // Read Unique ID
    if let Some(Entry {
      value: Value::Byte(unique_id), ..
    }) = self.tiff.root_ifd().get_entry(DngTag::RawDataUniqueID)
    {
      if let Ok(id) = unique_id.as_slice().try_into() {
        mdata.unique_image_id = Some(u128::from_le_bytes(id));
      }
    }
    Ok(mdata)
  }

  fn xpacket(&self, _file: &RawSource, _params: &RawDecodeParams) -> Result<Option<Vec<u8>>> {
    match self.tiff.root_ifd().get_entry(TiffCommonTag::Xmp) {
      Some(Entry { value: Value::Byte(buf), .. }) => Ok(Some(buf.clone())),
      _ => Ok(None),
    }
  }
}

impl<'a> TfrDecoder<'a> {
  /// Get lens description by analyzing TIFF tags and makernotes
  fn get_lens_description(&self) -> Result<Option<&'static LensDescription>> {
    if let Some(exif) = self.tiff.root_ifd().get_sub_ifd(TiffCommonTag::ExifIFDPointer) {
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
    Ok([1.0 / levels.force_f32(0), 1.0 / levels.force_f32(1), 1.0 / levels.force_f32(2), f32::NAN])
  }

  fn decode_compressed(&self, src: &[u8], width: usize, height: usize, dummy: bool) -> Result<PixU16> {
    let mut out = alloc_image_ok!(width, height, dummy);
    let decompressor = LjpegDecompressor::new_full(src, true, false)?;
    decompressor.decode(out.pixels_mut(), 0, width, width, height, dummy)?;
    Ok(out)
  }
}
