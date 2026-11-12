// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Dark frame reduction
//!
//! A dark frame is an exposure without light, with the exposure time, the ISO
//! and the temperature of the image. It holds the dark signal of the sensor:
//! hot pixels, the dark current and the fixed pattern of the readout. The dark
//! signal, the dark frame minus its black level, is subtracted from the raw
//! data as read from the file:
//!
//! `image - (dark frame - black level of the dark frame)`
//!
//! The image keeps its black level, like the long exposure noise reduction of
//! cameras.

use std::borrow::Cow;

use rayon::prelude::*;

use crate::{
  RawImage, RawImageData,
  decoders::RawMetadata,
  exif::Exif,
  imgop::{Dim2, Point, Rect, raw::apply_linearization_table},
  rawimage::BlackLevel,
};

/// Relative difference of the exposure times of the dark frame and the image
/// that is reported
const EXPOSURE_TIME_TOLERANCE: f32 = 0.1;

/// Dark frame to subtract from raw images, see [`subtract_dark_frame`]
#[derive(Clone, PartialEq)]
pub struct DarkFrame {
  /// Name for messages, e.g. the file name
  pub name: String,
  pub width: usize,
  pub height: usize,
  /// Samples per pixel
  pub cpp: usize,
  /// `width` * `height` * `cpp` samples
  pub data: RawImageData,
  /// Black level of the dark frame, `None` for the black level of the image
  pub blacklevel: Option<BlackLevel>,
  /// ActiveArea of the dark frame, the differences of `blacklevel` refer to it
  pub active_area: Option<Rect>,
  /// The samples are linear values. Otherwise they are stored like the raw
  /// data of the image, and its LinearizationTable applies to them.
  pub linear: bool,
  /// Camera of the dark frame, make and model
  pub camera: Option<String>,
  /// ISO of the dark frame
  pub iso: Option<u32>,
  /// Exposure time of the dark frame in seconds
  pub exposure_time: Option<f32>,
}

impl DarkFrame {
  /// Dark frame of samples that are stored like the raw data of the image,
  /// e.g. read from a PGM file. `blacklevel` is the black level of the
  /// samples, `None` for the black level of the image.
  pub fn new(name: impl Into<String>, data: RawImageData, width: usize, height: usize, cpp: usize, blacklevel: Option<BlackLevel>) -> Self {
    Self {
      name: name.into(),
      width,
      height,
      cpp,
      data,
      blacklevel,
      active_area: None,
      linear: false,
      camera: None,
      iso: None,
      exposure_time: None,
    }
  }

  /// Dark frame of a raw image and its metadata
  ///
  /// The LinearizationTable of `rawimage` is applied to its samples, they keep
  /// the black level of `rawimage`.
  pub fn from_raw(name: impl Into<String>, mut rawimage: RawImage, metadata: &RawMetadata) -> Self {
    if let RawImageData::Integer(samples) = &mut rawimage.data
      && let Some(table) = rawimage.linearization_table.take()
    {
      apply_linearization_table(samples, rawimage.width * rawimage.cpp, &table);
    }
    Self {
      name: name.into(),
      width: rawimage.width,
      height: rawimage.height,
      cpp: rawimage.cpp,
      camera: Some(rawimage.unique_camera_model()),
      blacklevel: Some(rawimage.blacklevel),
      active_area: rawimage.active_area,
      data: rawimage.data,
      linear: true,
      iso: iso(&metadata.exif),
      exposure_time: exposure_time(&metadata.exif),
    }
  }

  /// The dark frame must have the size and the samples per pixel of `rawimage`
  pub fn check_size(&self, rawimage: &RawImage) -> crate::Result<()> {
    if (self.width, self.height, self.cpp) != (rawimage.width, rawimage.height, rawimage.cpp) {
      return Err(input_error!(
        "Dark frame '{}' has {}x{} pixels with {} samples, the raw image {}x{} pixels with {}",
        self.name,
        self.width,
        self.height,
        self.cpp,
        rawimage.width,
        rawimage.height,
        rawimage.cpp
      ));
    }
    Ok(())
  }

  /// Warn if the dark frame is from another camera than `rawimage`, or has
  /// another ISO or exposure time than its `metadata`
  pub fn check_image(&self, rawimage: &RawImage, metadata: &RawMetadata) {
    let camera = rawimage.unique_camera_model();
    if let Some(dark) = self.camera.as_deref().filter(|dark| !dark.eq_ignore_ascii_case(&camera)) {
      log::warn!("Dark frame '{}' is from the camera '{}', the image is from '{}'", self.name, dark, camera);
    }
    if let (Some(dark), Some(image)) = (self.iso, iso(&metadata.exif))
      && dark != image
    {
      log::warn!("Dark frame '{}' has ISO {}, the image ISO {}", self.name, dark, image);
    }
    if let (Some(dark), Some(image)) = (self.exposure_time, exposure_time(&metadata.exif))
      && (dark - image).abs() > EXPOSURE_TIME_TOLERANCE * dark.max(image)
    {
      log::warn!("Dark frame '{}' has an exposure time of {} s, the image {} s", self.name, dark, image);
    }
  }
}

impl std::fmt::Debug for DarkFrame {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "DarkFrame('{}', {}x{}, {} samples per pixel)", self.name, self.width, self.height, self.cpp)
  }
}

/// ISO of the EXIF data
fn iso(exif: &Exif) -> Option<u32> {
  exif.iso_speed_ratings.map(u32::from).or(exif.recommended_exposure_index).or(exif.iso_speed)
}

/// Exposure time of the EXIF data in seconds
fn exposure_time(exif: &Exif) -> Option<f32> {
  exif.exposure_time.map(|time| time.as_f32()).filter(|time| time.is_finite())
}

/// Subtract the dark signal of `dark` from the raw data of `rawimage`
///
/// The raw data must be as read from the file. A LinearizationTable of
/// `rawimage` is applied first, also to the samples of `dark` if they are not
/// linear. Samples at or above the white level stay unchanged, so clipped
/// highlights stay white, unless the dark frame is at the white level too: a
/// hot pixel that saturates without light is subtracted, like cameras do.
/// Integer samples are rounded and limited to
/// 0 .. 65535. The black and white levels and the masked areas of `rawimage`
/// stay valid: the result keeps the black level of the image.
pub fn subtract_dark_frame(rawimage: &mut RawImage, dark: &DarkFrame) -> crate::Result<()> {
  dark.check_size(rawimage)?;
  let (width, height, cpp) = (rawimage.width, rawimage.height, rawimage.cpp);
  let row_len = width * cpp;
  if row_len == 0 {
    return Ok(());
  }
  let mut dark_data = Cow::Borrowed(&dark.data);
  if let RawImageData::Integer(samples) = &mut rawimage.data
    && let Some(table) = rawimage.linearization_table.take()
  {
    apply_linearization_table(samples, row_len, &table);
    if !dark.linear
      && let RawImageData::Integer(dark_samples) = dark_data.to_mut()
    {
      apply_linearization_table(dark_samples, row_len, &table);
    }
  }
  let (black, black_area) = match &dark.blacklevel {
    Some(black) => (black, dark.active_area),
    None => (&rawimage.blacklevel, rawimage.active_area),
  };
  let black_area = black_area.unwrap_or_else(|| Rect::new(Point::zero(), Dim2::new(width, height)));
  if black.cpp != cpp && black.cpp != 1 {
    return Err(input_error!(
      "Black level of dark frame '{}' has {} samples per pixel, the image {}",
      dark.name,
      black.cpp,
      cpp
    ));
  }
  let white = &rawimage.whitelevel.0;
  if white.len() != cpp && white.len() != 1 {
    return Err(input_error!("White level has {} values, the image has {} samples per pixel", white.len(), cpp));
  }
  let white: Vec<f32> = (0..cpp).map(|sample| white[sample.min(white.len() - 1)] as f32).collect();
  // Dark signal of the samples of a row, the dark frame minus its black level,
  // and whether the dark frame is at the white level
  let signal = |row: usize| -> Vec<(f32, bool)> {
    let range = row * row_len..(row + 1) * row_len;
    let samples: Box<dyn Iterator<Item = f32>> = match dark_data.as_ref() {
      RawImageData::Integer(samples) => Box::new(samples[range].iter().map(|v| f32::from(*v))),
      RawImageData::Float(samples) => Box::new(samples[range].iter().copied()),
    };
    samples
      .enumerate()
      .map(|(i, v)| {
        let black = black.level(row, i / cpp, (i % cpp).min(black.cpp - 1), black_area);
        (v - black, v >= white[i % cpp])
      })
      .collect()
  };
  // Sum of the dark signal of a row
  let sum = |signal: &[(f32, bool)]| signal.iter().map(|(s, _)| f64::from(*s)).sum::<f64>();
  let total: f64 = match &mut rawimage.data {
    RawImageData::Integer(samples) => {
      if rawimage.blacklevel.levels.iter().all(|level| level.as_f32() == 0.0) {
        log::warn!("The image has a black level of 0, values below it are clipped after the dark frame reduction");
      }
      samples
        .par_chunks_exact_mut(row_len)
        .enumerate()
        .map(|(row, samples)| {
          let signal = signal(row);
          for (i, (value, (signal, saturated))) in samples.iter_mut().zip(&signal).enumerate() {
            let v = f32::from(*value);
            if v < white[i % cpp] || *saturated {
              // The conversion saturates at 0 and 65535
              *value = (v - signal).round() as u16;
            }
          }
          sum(&signal)
        })
        .sum()
    }
    RawImageData::Float(samples) => samples
      .par_chunks_exact_mut(row_len)
      .enumerate()
      .map(|(row, samples)| {
        let signal = signal(row);
        for (i, (value, (signal, saturated))) in samples.iter_mut().zip(&signal).enumerate() {
          if *value < white[i % cpp] || *saturated {
            *value -= signal;
          }
        }
        sum(&signal)
      })
      .sum(),
  };
  log::info!(
    "Dark frame '{}' is subtracted, the mean dark signal is {:.2}",
    dark.name,
    total / (width * height * cpp).max(1) as f64
  );
  Ok(())
}
