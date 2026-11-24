// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Decoder for the STI files of Sinarback digital backs
//!
//! STI is a TIFF file. IFD0 holds an 8 bit RGB preview, the EXIF tags and
//! Sinar tags with text lines of the form `<name> <values>`. The CFA data is
//! stored uncompressed with 16 bits per pixel in its own IFD, followed by
//! IFDs with a 16 bit linear RGB preview and an 8 bit RGB thumbnail.

use std::sync::Arc;

use image::DynamicImage;
use serde::{Deserialize, Serialize};

use crate::analyze::FormatDump;
use crate::exif::Exif;
use crate::formats::tiff::reader::TiffReader;
use crate::formats::tiff::{GenericTiffReader, IFD};
use crate::rawsource::RawSource;
use crate::tags::TiffCommonTag;
use crate::{RawImage, RawLoader, RawlerError, Result};

use super::{
  BlackLevel, CFAConfig, CameraLookup, Decoder, FormatHint, Orientation, RawDecodeParams, RawMetadata, RawPhotometricInterpretation, WellKnownIFD,
  dynamic_image_from_ifd, image_bytes, plain_image_from_ifd_ext, tiff_ifd,
};

/// PhotometricInterpretation of the IFD with the CFA data
const PHOTOMETRIC_CFA: u16 = 32803;

/// STI format encapsulation for analyzer
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StiFormat {
  tiff: GenericTiffReader,
}

#[derive(Debug, Clone)]
pub struct StiDecoder<'a> {
  #[allow(unused)]
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
  camera: CameraLookup,
}

impl<'a> StiDecoder<'a> {
  pub fn new(_file: &RawSource, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<StiDecoder<'a>> {
    let root = tiff.root_ifd();
    let make = fetch_tiff_tag!(root, TiffCommonTag::Make).get_string()?.trim_end();
    let model = fetch_tiff_tag!(root, TiffCommonTag::Model).get_string()?.trim_end();
    // The model names the back and the camera it is used on, for example
    // "Sinarback eVolution 75, Sinar p3 / f3". The camera database lists the backs.
    let back = model.split_once(',').map_or(model, |(back, _)| back);
    let camera = rawloader.lookup_camera_with_everything(make, back, "");
    Ok(StiDecoder { rawloader, tiff, camera })
  }
}

impl<'a> Decoder for StiDecoder<'a> {
  fn raw_image(&self, file: &RawSource, _params: &RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let camera = self.camera.supported()?;
    let raw = self.get_raw_ifd()?;
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let cpp = fetch_tiff_tag!(raw, TiffCommonTag::SamplesPerPixel).force_usize(0);
    if cpp != 1 {
      return Err(decoder_error!("STI: CFA data with {} samples per pixel is not supported", cpp));
    }
    let shots = shot_count(raw, width, height);
    if shots > 1 {
      return Err(RawlerError::unsupported(
        camera,
        format!("STI: multi-shot files with {} shots are not supported", shots),
      ));
    }

    let data = plain_image_from_ifd_ext(raw, file, false, dummy)?;
    let blacklevel = self.black_level().map(|black| BlackLevel::new(&[black], 1, 1, cpp));
    let photometric = RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(camera));
    let mut img = RawImage::new_with_data(camera.clone(), data, width, height, cpp, self.get_wb(), photometric, blacklevel, None, dummy);
    img.orientation = self.orientation();
    Ok(img)
  }

  fn check_supported(&self) -> Result<()> {
    self.camera.supported().map(|_| ())
  }

  fn raw_metadata(&self, _file: &RawSource, _params: &RawDecodeParams) -> Result<RawMetadata> {
    let mut exif = Exif::new(self.tiff.root_ifd())?;
    // The Orientation tag of IFD0 is that of the preview
    exif.orientation = self.orientation().map(|orientation| orientation.to_u16());
    Ok(RawMetadata::new(self.camera.metadata(), exif))
  }

  fn thumbnail_image(&self, file: &RawSource, params: &RawDecodeParams) -> Result<Option<DynamicImage>> {
    if params.image_index != 0 {
      return Ok(None);
    }
    self.reduced_ifd(8).map(|ifd| dynamic_image_from_ifd(ifd, file)).transpose()
  }

  fn preview_image(&self, file: &RawSource, params: &RawDecodeParams) -> Result<Option<DynamicImage>> {
    if params.image_index != 0 {
      return Ok(None);
    }
    // The 8 bit preview is in IFD0, but the BitsPerSample tag of IFD0 holds no valid values
    let root = self.tiff.root_ifd();
    let width = fetch_tiff_tag!(root, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(root, TiffCommonTag::ImageLength).force_usize(0);
    let buf = root.singlestrip_data_rawsource(file)?;
    if buf.len() != image_bytes("STI", width, height, 24)? {
      return Err(decoder_error!("STI: preview of {}x{} pixels has {} bytes", width, height, buf.len()));
    }
    let image =
      image::RgbImage::from_raw(width as u32, height as u32, buf.to_vec()).ok_or_else(|| decoder_error!("STI: invalid preview size {}x{}", width, height))?;
    Ok(Some(DynamicImage::ImageRgb8(image)))
  }

  fn format_dump(&self) -> FormatDump {
    FormatDump::Sti(StiFormat { tiff: self.tiff.clone() })
  }

  fn ifd(&self, _file: &RawSource, _params: &RawDecodeParams, wk_ifd: WellKnownIFD) -> Result<Option<Arc<IFD>>> {
    Ok(match wk_ifd {
      WellKnownIFD::Root | WellKnownIFD::Exif | WellKnownIFD::ExifGps => tiff_ifd(self.tiff.root_ifd(), wk_ifd),
      WellKnownIFD::Raw => Some(Arc::new(self.get_raw_ifd()?.clone())),
      WellKnownIFD::Preview => Some(Arc::new(self.tiff.root_ifd().clone())),
      _ => None,
    })
  }

  fn format_hint(&self) -> FormatHint {
    FormatHint::STI
  }
}

impl<'a> StiDecoder<'a> {
  /// The IFD with the CFA data
  fn get_raw_ifd(&self) -> Result<&IFD> {
    self
      .tiff
      .find_ifds_with_filter(|ifd| ifd.get_entry(TiffCommonTag::PhotometricInt).map(|entry| entry.force_u16(0)) == Some(PHOTOMETRIC_CFA))
      .first()
      .copied()
      .ok_or_else(|| decoder_error!("STI: Failed to find the IFD with the CFA data"))
  }

  /// A reduced resolution RGB image after IFD0 with `bits` per sample
  fn reduced_ifd(&self, bits: u32) -> Option<&IFD> {
    self.tiff.chains().iter().skip(1).find(|ifd| {
      ifd.get_entry(TiffCommonTag::NewSubFileType).map(|entry| entry.force_u32(0)) == Some(1)
        && ifd.get_entry(TiffCommonTag::BitsPerSample).map(|entry| entry.force_u32(0)) == Some(bits)
    })
  }

  /// The orientation of the CFA data, IFD0 has that of the preview
  fn orientation(&self) -> Option<Orientation> {
    self.get_raw_ifd().ok().and_then(Orientation::from_tiff)
  }

  /// The black level, CaptureShop shifts the black of the CFA data to this value
  fn black_level(&self) -> Option<u32> {
    let level = self.sinar_param(TiffCommonTag::SinarBackParams, "TargetBlackValue")?;
    level.parse().map_err(|_| log::warn!("STI: invalid TargetBlackValue '{}'", level)).ok()
  }

  /// White balance coefficients from the processing settings. The second set
  /// of settings is preferred, it has the full image as region of interest.
  fn get_wb(&self) -> [f32; 4] {
    let neutral = self
      .sinar_param(TiffCommonTag::SinarProcessingParams2, "Neutral")
      .or_else(|| self.sinar_param(TiffCommonTag::SinarProcessingParams, "Neutral"));
    neutral.map_or([f32::NAN; 4], wb_from_neutral)
  }

  /// Values of the line `name` of the Sinar text tag `tag`
  fn sinar_param(&self, tag: TiffCommonTag, name: &str) -> Option<&str> {
    param_value(self.tiff.root_ifd().get_entry(tag)?.as_string()?, name)
  }
}

/// Values of the line `<name> <values>` of a Sinar text tag
fn param_value<'t>(text: &'t str, name: &str) -> Option<&'t str> {
  text
    .lines()
    .find_map(|line| line.trim().split_once(' ').filter(|(key, _)| *key == name).map(|(_, value)| value.trim()))
}

/// White balance coefficients from the `Neutral` values, they are the
/// multipliers for red, green and blue. Unknown for missing or invalid values.
fn wb_from_neutral(values: &str) -> [f32; 4] {
  let coeffs: Vec<f32> = values.split_whitespace().take(3).map(|value| value.parse().unwrap_or(f32::NAN)).collect();
  match coeffs[..] {
    [r, g, b] if coeffs.iter().all(|coeff| coeff.is_finite() && *coeff > 0.0) => [r, g, b, f32::NAN],
    _ => {
      log::warn!("STI: invalid Neutral values '{}', the white balance is unknown", values);
      [f32::NAN; 4]
    }
  }
}

/// Number of exposures in the IFD with the CFA data. Multi-shot files have
/// one tile of the full image size for each shot.
fn shot_count(raw: &IFD, width: usize, height: usize) -> usize {
  let tile_width = raw.get_entry(TiffCommonTag::TileWidth).map(|entry| entry.force_usize(0));
  let tile_length = raw.get_entry(TiffCommonTag::TileLength).map(|entry| entry.force_usize(0));
  match (tile_width, tile_length, raw.get_entry(TiffCommonTag::TileOffsets)) {
    (Some(tile_width), Some(tile_length), Some(offsets)) if tile_width >= width && tile_length >= height => offsets.count() as usize,
    _ => 1,
  }
}
