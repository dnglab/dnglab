// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Dark frames of `--dark-frame`
//!
//! A dark frame is a raw file, or a 16-bit TIFF, PGM or PPM file of the raw
//! pixels, e.g. written by `dnglab process-raw --skip-steps all`. It must have
//! the size and the color planes of the raw images it is subtracted from.

use std::{
  path::{Path, PathBuf},
  sync::Arc,
};

use clap::ArgMatches;
use image::{DynamicImage, ImageReader};
use rawler::{RawImageData, decoders::RawDecodeParams, imgop::darkframe::DarkFrame, rawimage::BlackLevel, rawsource::RawSource};

use crate::{AppError, Result};

/// Extensions of image files with raw pixels, other files are raw files
const IMAGE_EXTENSIONS: [&str; 5] = ["tif", "tiff", "pgm", "ppm", "pnm"];

/// Black level of a dark frame, option `--dark-frame-black`
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DarkFrameBlack {
  /// The black level of a raw dark frame, the black level of the image for
  /// an image file
  Auto,
  /// This black level for all samples
  Level(f32),
}

/// Parse `auto` or a black level of 0 or more
pub fn dark_frame_black_parser(value: &str) -> std::result::Result<DarkFrameBlack, String> {
  if value.eq_ignore_ascii_case("auto") {
    return Ok(DarkFrameBlack::Auto);
  }
  let level: f32 = value.parse().map_err(|err| format!("{}", err))?;
  if level.is_finite() && level >= 0.0 {
    Ok(DarkFrameBlack::Level(level))
  } else {
    Err(format!("{} is not a black level of 0 or more", value))
  }
}

/// Read the dark frame of option `id`, with the black level of option `black_id`
pub fn dark_frame_option(options: &ArgMatches, id: &str, black_id: &str) -> Result<Option<Arc<DarkFrame>>> {
  let Some(path) = options.get_one::<PathBuf>(id) else {
    return Ok(None);
  };
  if !path.exists() {
    return Err(AppError::NotFound(path.clone()));
  }
  let mut dark = open_dark_frame(path).map_err(|err| AppError::General(format!("Dark frame {}: {}", path.display(), err)))?;
  if let Some(DarkFrameBlack::Level(level)) = options.get_one::<DarkFrameBlack>(black_id) {
    dark.blacklevel = Some(BlackLevel::new(&vec![*level; dark.cpp], 1, 1, dark.cpp));
  }
  log::info!("Dark frame {:?}, black level {:?}", dark, dark.blacklevel);
  Ok(Some(Arc::new(dark)))
}

/// Dark frame of the raw or image file `path`
fn open_dark_frame(path: &Path) -> Result<DarkFrame> {
  let name = path
    .file_name()
    .map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
  let image_file = path
    .extension()
    .and_then(|ext| ext.to_str())
    .is_some_and(|ext| IMAGE_EXTENSIONS.iter().any(|image| ext.eq_ignore_ascii_case(image)));
  if image_file {
    image_dark_frame(name, path)
  } else {
    raw_dark_frame(name, path)
  }
}

/// Dark frame of the first raw image of the raw file `path`
fn raw_dark_frame(name: String, path: &Path) -> Result<DarkFrame> {
  let rawfile = RawSource::new(path)?;
  let decoder = rawler::get_decoder(&rawfile)?;
  let params = RawDecodeParams::default();
  let rawimage = decoder.raw_image(&rawfile, &params, false)?;
  let metadata = decoder.raw_metadata(&rawfile, &params)?;
  Ok(DarkFrame::from_raw(name, rawimage, &metadata))
}

/// Dark frame of the TIFF, PGM or PPM file `path`, with the samples stored
/// like the raw data of the image and its black level
fn image_dark_frame(name: String, path: &Path) -> Result<DarkFrame> {
  let image = ImageReader::open(path)?.decode()?;
  let (width, height) = (image.width() as usize, image.height() as usize);
  let (data, cpp) = match image {
    DynamicImage::ImageLuma16(image) => (RawImageData::Integer(image.into_raw()), 1),
    DynamicImage::ImageRgb16(image) => (RawImageData::Integer(image.into_raw()), 3),
    DynamicImage::ImageRgba16(image) => (RawImageData::Integer(image.into_raw()), 4),
    DynamicImage::ImageRgb32F(image) => (RawImageData::Float(image.into_raw()), 3),
    DynamicImage::ImageRgba32F(image) => (RawImageData::Float(image.into_raw()), 4),
    image => {
      return Err(AppError::General(format!(
        "{:?} samples are not supported, a dark frame needs 16-bit integer or 32-bit float samples",
        image.color()
      )));
    }
  };
  Ok(DarkFrame::new(name, data, width, height, cpp, None))
}
