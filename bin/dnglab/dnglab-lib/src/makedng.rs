// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use clap::builder::{NonEmptyStringValueParser, TypedValueParser};
use clap::ArgMatches;

use image::DynamicImage;
use itertools::Itertools;
use rawler::decoders::{RawDecodeParams, RawMetadata};
use rawler::dng::writer::DngWriter;
use rawler::dng::{self, CropMode, DngCompression, DngPhotometricConversion, DNG_VERSION_V1_6};
use rawler::exif::Exif;
use rawler::formats::jfif::{is_exif, is_jfif, Jfif};
use rawler::formats::tiff::{self, Rational, SRational};
use rawler::imgop::gamma::{apply_gamma, invert_gamma};

use rawler::imgop::srgb::{srgb_apply_gamma, srgb_invert_gamma};
use rawler::imgop::xyz::{self, Illuminant};
use rawler::imgop::{scale_double_to_u16, scale_u16_to_double, scale_u8_to_double};
use rawler::tags::{DngTag, TiffCommonTag};
use rawler::{get_decoder, RawFile};
use std::fs::{remove_file, File};
use std::io::{BufReader, BufWriter};
use std::num::ParseFloatError;
use std::path::{Path, PathBuf};

use std::str::FromStr;
use std::time::Instant;

fn get_input_path<'a>(inputs: &'a [&PathBuf], maps: &[&InputSourceUsageMap], usage: InputUsage) -> std::io::Result<&'a PathBuf> {
  maps
    .iter()
    .find(|map| map.usage == usage)
    .and_then(|x| inputs.get(x.source).copied())
    .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("No input found for '{:?}'", usage)))
}

pub async fn makedng(options: &ArgMatches) -> crate::Result<()> {
  let dest_path: &PathBuf = options.get_one("OUTPUT").expect("Output path is required");

  match makedng_internal(options, dest_path).await {
    Ok(_) => Ok(()),
    Err(err) => {
      if let Err(err) = remove_file(dest_path) {
        log::error!("Failed to delete DNG file after decompress error: {:?}", err);
      }
      Err(err)
    }
  }
}

/// Entry point for Clap sub command `makedng`
pub async fn makedng_internal(options: &ArgMatches, dest_path: &Path) -> crate::Result<()> {
  let _now = Instant::now();

  let inputs: Vec<&PathBuf> = options.get_many("inputs").expect("inputs are required").collect();
  let maps: Vec<&InputSourceUsageMap> = options.get_many("map").unwrap_or_default().collect();
  let max_used_map_input = maps.iter().map(|x| x.source).max().unwrap_or(0);

  if max_used_map_input >= inputs.len() {
    return Err(crate::AppError::InvalidCmdSwitch(format!(
      "--map argument indicies ({}) exceed number of input files ({})",
      max_used_map_input,
      inputs.len()
    )));
  }

  if dest_path.exists() && !options.get_flag("override") {
    return Err(crate::AppError::AlreadyExists(dest_path.to_owned()));
  }

  let mut stream = BufWriter::new(File::create(dest_path)?);

  let backward_version = options
    .get_one::<DngVersion>("dng_backward_version")
    .unwrap_or(&DngVersion::V1_4)
    .as_dng_value();

  let mut dng = DngWriter::new(&mut stream, backward_version)?;

  if let Some(artist) = options.get_one::<String>("artist") {
    dng.root_ifd_mut().add_tag(TiffCommonTag::Artist, artist);
  }

  if let Some(make) = options.get_one::<String>("make") {
    dng.root_ifd_mut().add_tag(TiffCommonTag::Make, make);
  }

  if let Some(model) = options.get_one::<String>("model") {
    dng.root_ifd_mut().add_tag(TiffCommonTag::Model, model);
  }

  if let Some(unique_camera_model) = options.get_one::<String>("unique_camera_model") {
    dng.root_ifd_mut().add_tag(DngTag::UniqueCameraModel, unique_camera_model);
  }

  if let Some(colorimetric_reference) = options.get_one::<DngColorimetricReference>("colorimetric_reference") {
    dng.root_ifd_mut().add_tag(DngTag::ColorimetricReference, colorimetric_reference.as_dng_value());
  }

  if let Ok(raw_input) = get_input_path(&inputs, &maps, InputUsage::Raw) {
    let mut rawfile = RawFile::new(raw_input, BufReader::new(File::open(raw_input)?));
    if let Ok(decoder) = get_decoder(&mut rawfile) {
      let rawimage = decoder.raw_image(&mut rawfile, RawDecodeParams::default(), false)?;
      let mut rawframe = dng.subframe(0);
      rawframe.raw_image(&rawimage, CropMode::Best, DngCompression::Lossless, DngPhotometricConversion::Original, 1)?;
      rawframe.finalize()?;
    } else {
      let image = image::open(raw_input)?;
      let mut frame = dng.subframe(0);
      match &image {
        DynamicImage::ImageRgb8(img) => {
          let buf = img.as_raw().as_slice();
          frame.rgb_image_u8(buf, img.width() as usize, img.height() as usize, DngCompression::Lossless, 1)?;
          frame.ifd_mut().add_tag(DngTag::BlackLevel, [0u16, 0, 0]);

          if let Some(linear_table_input) = options.get_one::<<LinearizationTableArgParser as TypedValueParser>::Value>("linearization") {
            // We have a linearization table that scales the input values to 16 bit.
            frame.ifd_mut().add_tag(DngTag::WhiteLevel, [u16::MAX, u16::MAX, u16::MAX]);
            frame.ifd_mut().add_tag(DngTag::LinearizationTable, linear_table_input);
          } else {
            // We have no linearization table, so from 8 bit image there is a maximum
            // white level of 255.
            frame.ifd_mut().add_tag(DngTag::WhiteLevel, [255, 255, 255]);
          }

          frame.finalize()?;
        }

        DynamicImage::ImageRgb16(img) => {
          let buf = img.as_raw().as_slice();
          frame.rgb_image_u16(buf, img.width() as usize, img.height() as usize, DngCompression::Lossless, 1)?;
          frame.ifd_mut().add_tag(DngTag::BlackLevel, [0u16, 0, 0]);

          if let Some(linear_table_input) = options.get_one::<<LinearizationTableArgParser as TypedValueParser>::Value>("linearization") {
            // We have a linearization table that scales the input values to 16 bit.
            frame.ifd_mut().add_tag(DngTag::WhiteLevel, [u16::MAX, u16::MAX, u16::MAX]);
            frame.ifd_mut().add_tag(DngTag::LinearizationTable, linear_table_input);
          } else {
            // We have no linearization table, so from 8 bit image there is a maximum
            // white level of 255.
            frame.ifd_mut().add_tag(DngTag::WhiteLevel, [u16::MAX, u16::MAX, u16::MAX]);
          }

          frame.finalize()?;
        }

        _ => {
          return Err(crate::AppError::General("Input format is not supported".to_owned()));
        }
      }
    }
  } else {
    eprintln!("No raw input file found");
  }

  if let Ok(preview_input) = get_input_path(&inputs, &maps, InputUsage::Preview) {
    let mut rawfile = RawFile::new(preview_input, BufReader::new(File::open(preview_input)?));
    if let Ok(decoder) = get_decoder(&mut rawfile) {
      if let Some(preview) = decoder.full_image(&mut rawfile)? {
        let mut frame = dng.subframe(1);
        frame.preview(&preview, 0.7)?;
        frame.finalize()?;
      }
    } else {
      let image = image::open(preview_input)?;
      let mut frame = dng.subframe(1);
      frame.preview(&image, 0.7)?;
      frame.finalize()?;
    }
  } else {
    eprintln!("No preview input file found");
  }

  if let Ok(thumbnail_input) = get_input_path(&inputs, &maps, InputUsage::Thumbnail) {
    let mut rawfile = RawFile::new(thumbnail_input, BufReader::new(File::open(thumbnail_input)?));
    if let Ok(decoder) = get_decoder(&mut rawfile) {
      if let Some(preview) = decoder.full_image(&mut rawfile)? {
        dng.thumbnail(&preview)?;
      }
    } else {
      let image = image::open(thumbnail_input)?;
      dng.thumbnail(&image)?;
    }
  } else {
    eprintln!("No thumbnail input file found");
  }

  if let Ok(exif_input) = get_input_path(&inputs, &maps, InputUsage::Exif) {
    let mut rawfile = RawFile::new(exif_input, BufReader::new(File::open(exif_input)?));
    // First, prefer JFIF decoder for preview files
    if is_jfif(&mut rawfile) || is_exif(&mut rawfile) {
      rawfile.seek_to_start()?;
      let jfif = Jfif::new(&mut rawfile)?;
      if let Some(exif_ifd) = jfif.exif_ifd() {
        let exif = Exif::new(exif_ifd)?;
        RawMetadata::fill_exif_ifd(&exif, dng.exif_ifd_mut())?; // TODO: missing GPS and Root data
      }
    } else if let Ok(decoder) = get_decoder(&mut rawfile) {
      dng.load_metadata(&decoder.raw_metadata(&mut rawfile, RawDecodeParams::default())?)?;
    } else {
      log::warn!("Unable to decode exif file from {:?}", exif_input);
    }
  } else {
    eprintln!("No EXIF input file found");
  }

  if let Ok(xmp_input) = get_input_path(&inputs, &maps, InputUsage::Xmp) {
    let mut rawfile = RawFile::new(xmp_input, BufReader::new(File::open(xmp_input)?));

    if is_jfif(&mut rawfile) || is_exif(&mut rawfile) {
      rawfile.seek_to_start()?;
      let jfif = Jfif::new(&mut rawfile)?;
      if let Some(xpacket) = jfif.xpacket() {
        dng.xpacket(xpacket)?;
      }
    } else if let Ok(decoder) = get_decoder(&mut rawfile) {
      if let Some(xpacket) = decoder.xpacket(&mut rawfile, RawDecodeParams::default())? {
        dng.xpacket(xpacket)?;
      }
    } else {
      log::warn!("Unable to decode XMP file from {:?}", xmp_input);
    }
  }

  if let Some(matrix) = options.get_one::<ColorMatrixArg>("matrix1") {
    let illuminant = options
      .get_one::<CalibrationIlluminantArg>("illuminant1")
      .expect("illuminant1 is required when matrix1 is set");
    dng.root_ifd_mut().add_tag(DngTag::ColorMatrix1, matrix.as_tiff_value());
    dng.root_ifd_mut().add_tag(DngTag::CalibrationIlluminant1, illuminant.as_tiff_value());
  }

  if let Some(matrix) = options.get_one::<ColorMatrixArg>("matrix2") {
    let illuminant = options
      .get_one::<CalibrationIlluminantArg>("illuminant2")
      .expect("illuminant2 is required when matrix2 is set");
    dng.root_ifd_mut().add_tag(DngTag::ColorMatrix2, matrix.as_tiff_value());
    dng.root_ifd_mut().add_tag(DngTag::CalibrationIlluminant2, illuminant.as_tiff_value());
  }

  if backward_version >= DNG_VERSION_V1_6 {
    if let Some(matrix) = options.get_one::<ColorMatrixArg>("matrix3") {
      let illuminant = options
        .get_one::<CalibrationIlluminantArg>("illuminant3")
        .expect("illuminant3 is required when matrix3 is set");
      dng.root_ifd_mut().add_tag(DngTag::ColorMatrix3, matrix.as_tiff_value());
      dng.root_ifd_mut().add_tag(DngTag::CalibrationIlluminant3, illuminant.as_tiff_value());
    }
  }

  if let Some(as_shot_neutral) = options.get_one::<WhiteBalanceInput>("as_shot_neutral") {
    dng.root_ifd_mut().add_tag(DngTag::AsShotNeutral, as_shot_neutral.as_tiff_value());
    dng.root_ifd_mut().remove_tag(DngTag::AsShotWhiteXY);
  }

  if let Some(as_shot_white_xy) = options.get_one::<WhitePointArg>("as_shot_white_xy") {
    dng.root_ifd_mut().add_tag(DngTag::AsShotWhiteXY, as_shot_white_xy.as_tiff_value());
    dng.root_ifd_mut().remove_tag(DngTag::AsShotNeutral);
  }

  // Debugging
  // let _xyz = xy_whitepoint_to_wb_coeff(CIE_1931_WHITE_POINT_D50.0, CIE_1931_WHITE_POINT_D50.1, &XYZ_TO_SRGB_D50);
  //eprintln!("XYZ: {:?}", xyz);

  dng.close()?;

  println!("File saved to: {}", dest_path.display());

  Ok(())
}

#[derive(Clone, Debug)]
pub struct WhiteBalanceInput {
  values: Vec<f32>,
}

impl FromStr for WhiteBalanceInput {
  type Err = ParseFloatError;

  fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
    Ok(Self {
      values: s.split(',').map(str::trim).map(str::parse::<f32>).collect::<std::result::Result<Vec<_>, _>>()?,
    })
  }
}

impl WhiteBalanceInput {
  fn as_tiff_value(&self) -> tiff::Value {
    tiff::Value::Rational(self.values.iter().map(|x| Rational::new((x * 10_000.0) as u32, 10_000)).collect_vec())
  }
}

#[derive(Clone, Debug, Copy, PartialEq, PartialOrd)]
pub enum InputUsage {
  Raw,
  Preview,
  Thumbnail,
  Exif,
  Xmp,
}

impl FromStr for InputUsage {
  type Err = String;

  fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
    match s {
      "raw" => Ok(Self::Raw),
      "preview" => Ok(Self::Preview),
      "thumbnail" => Ok(Self::Thumbnail),
      "exif" => Ok(Self::Exif),
      "xmp" => Ok(Self::Xmp),
      _ => Err(format!("Unknown input usage: {}", s)),
    }
  }
}

#[derive(Clone, Debug)]
pub struct InputSourceUsageMap {
  source: usize,
  usage: InputUsage,
}

impl FromStr for InputSourceUsageMap {
  type Err = String;

  fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
    let ins: Vec<&str> = s.split(':').map(str::trim).collect();
    match ins.len() {
      2 => Ok(Self {
        source: ins[0].parse().map_err(|err| format!("Failed to parse '{}' to integer: {}", ins[0], err))?,
        usage: InputUsage::from_str(ins[1])?,
      }),
      _ => Err(format!("'{}' is not a valid input source map.", s)),
    }
  }
}

#[derive(Clone)]
pub struct LinearizationTableArgParser;

impl clap::builder::TypedValueParser for LinearizationTableArgParser {
  type Value = Vec<u16>;

  fn parse_ref(&self, cmd: &clap::Command, arg: Option<&clap::Arg>, value: &std::ffi::OsStr) -> std::result::Result<Self::Value, clap::Error> {
    let inner = NonEmptyStringValueParser::new();
    let val = inner.parse_ref(cmd, arg, value)?;

    Ok(match val.as_str() {
      "8bit_sRGB" => (0..=u8::MAX).map(scale_u8_to_double).map(srgb_apply_gamma).map(scale_double_to_u16).collect(),
      "8bit_sRGB_invert" => (0..=u8::MAX).map(scale_u8_to_double).map(srgb_invert_gamma).map(scale_double_to_u16).collect(),

      "16bit_sRGB" => (0..=u16::MAX).map(scale_u16_to_double).map(srgb_apply_gamma).map(scale_double_to_u16).collect(),
      "16bit_sRGB_invert" => (0..=u16::MAX)
        .map(scale_u16_to_double)
        .map(srgb_invert_gamma)
        .map(scale_double_to_u16)
        .collect(),

      "8bit_gamma1.8" => (0..=u8::MAX)
        .map(scale_u8_to_double)
        .map(|v| apply_gamma(v, 1.8))
        .map(scale_double_to_u16)
        .collect(),
      "8bit_gamma1.8_invert" => (0..=u8::MAX)
        .map(scale_u8_to_double)
        .map(|v| invert_gamma(v, 1.8))
        .map(scale_double_to_u16)
        .collect(),

      "8bit_gamma2.0" => (0..=u8::MAX)
        .map(scale_u8_to_double)
        .map(|v| apply_gamma(v, 2.0))
        .map(scale_double_to_u16)
        .collect(),
      "8bit_gamma2.0_invert" => (0..=u8::MAX)
        .map(scale_u8_to_double)
        .map(|v| invert_gamma(v, 2.0))
        .map(scale_double_to_u16)
        .collect(),

      "8bit_gamma2.2" => (0..=u8::MAX)
        .map(scale_u8_to_double)
        .map(|v| apply_gamma(v, 2.2))
        .map(scale_double_to_u16)
        .collect(),
      "8bit_gamma2.2_invert" => (0..=u8::MAX)
        .map(scale_u8_to_double)
        .map(|v| invert_gamma(v, 2.2))
        .map(scale_double_to_u16)
        .collect(),

      "8bit_gamma2.4" => (0..=u8::MAX)
        .map(scale_u8_to_double)
        .map(|v| apply_gamma(v, 2.4))
        .map(scale_double_to_u16)
        .collect(),
      "8bit_gamma2.4_invert" => (0..=u8::MAX)
        .map(scale_u8_to_double)
        .map(|v| invert_gamma(v, 2.4))
        .map(scale_double_to_u16)
        .collect(),

      "16bit_gamma1.8" => (0..=u16::MAX)
        .map(scale_u16_to_double)
        .map(|v| apply_gamma(v, 1.8))
        .map(scale_double_to_u16)
        .collect(),
      "16bit_gamma1.8_invert" => (0..=u16::MAX)
        .map(scale_u16_to_double)
        .map(|v| invert_gamma(v, 1.8))
        .map(scale_double_to_u16)
        .collect(),

      "16bit_gamma2.0" => (0..=u16::MAX)
        .map(scale_u16_to_double)
        .map(|v| apply_gamma(v, 2.0))
        .map(scale_double_to_u16)
        .collect(),
      "16bit_gamma2.0_invert" => (0..=u16::MAX)
        .map(scale_u16_to_double)
        .map(|v| invert_gamma(v, 2.0))
        .map(scale_double_to_u16)
        .collect(),

      "16bit_gamma2.2" => (0..=u16::MAX)
        .map(scale_u16_to_double)
        .map(|v| apply_gamma(v, 2.2))
        .map(scale_double_to_u16)
        .collect(),
      "16bit_gamma2.2_invert" => (0..=u16::MAX)
        .map(scale_u16_to_double)
        .map(|v| invert_gamma(v, 2.2))
        .map(scale_double_to_u16)
        .collect(),

      "16bit_gamma2.4" => (0..=u16::MAX)
        .map(scale_u16_to_double)
        .map(|v| apply_gamma(v, 2.4))
        .map(scale_double_to_u16)
        .collect(),
      "16bit_gamma2.4_invert" => (0..=u16::MAX)
        .map(scale_u16_to_double)
        .map(|v| invert_gamma(v, 2.4))
        .map(scale_double_to_u16)
        .collect(),

      _ => match val.split(',').map(str::trim).map(str::parse::<u16>).collect::<std::result::Result<Vec<_>, _>>() {
        Ok(items) => items,
        Err(fail) => {
          let mut err = clap::Error::new(clap::error::ErrorKind::ValueValidation).with_cmd(cmd);
          if let Some(arg) = arg {
            err.insert(clap::error::ContextKind::InvalidArg, clap::error::ContextValue::String(arg.to_string()));
          }
          err.insert(clap::error::ContextKind::InvalidValue, clap::error::ContextValue::String(val));
          err.insert(clap::error::ContextKind::Suggested, clap::error::ContextValue::String(fail.to_string()));
          return Err(err);
        }
      },
    })
  }

  fn possible_values(&self) -> Option<Box<dyn Iterator<Item = clap::builder::PossibleValue> + '_>> {
    Some(Box::new(
      [
        "8bit_sRGB",
        "8bit_sRGB_invert",
        "16bit_sRGB",
        "16bit_sRGB_invert",
        "8bit_gamma1.8",
        "8bit_gamma1.8_invert",
        "8bit_gamma2.0",
        "8bit_gamma2.0_invert",
        "8bit_gamma2.2",
        "8bit_gamma2.2_invert",
        "8bit_gamma2.4",
        "8bit_gamma2.4_invert",
        "16bit_gamma1.8",
        "16bit_gamma1.8_invert",
        "16bit_gamma2.0",
        "16bit_gamma2.0_invert",
        "16bit_gamma2.2",
        "16bit_gamma2.2_invert",
        "16bit_gamma2.4",
        "16bit_gamma2.4_invert",
        "custom table (comma seperated)",
      ]
      .into_iter()
      .map(clap::builder::PossibleValue::from),
    ))
  }
}

#[derive(Clone, Debug)]
pub struct WhitePointArg {
  x: f32,
  y: f32,
}

impl WhitePointArg {
  fn as_tiff_value(&self) -> tiff::Value {
    tiff::Value::Rational(vec![
      Rational::new((self.x * 10_000.0) as u32, 10_000),
      Rational::new((self.y * 10_000.0) as u32, 10_000),
    ])
  }
}

impl From<(f32, f32)> for WhitePointArg {
  fn from(value: (f32, f32)) -> Self {
    Self { x: value.0, y: value.1 }
  }
}

#[derive(Clone)]
pub struct WhitePointArgParser;

impl clap::builder::TypedValueParser for WhitePointArgParser {
  type Value = WhitePointArg;

  fn parse_ref(&self, cmd: &clap::Command, arg: Option<&clap::Arg>, value: &std::ffi::OsStr) -> std::result::Result<Self::Value, clap::Error> {
    let inner = NonEmptyStringValueParser::new();
    let val = inner.parse_ref(cmd, arg, value)?;

    Ok(
      match val.as_str() {
        "D50" => xyz::CIE_1931_WHITE_POINT_D50,
        "D65" => xyz::CIE_1931_WHITE_POINT_D65,
        _ => {
          match val
            .as_str()
            .split(',')
            .map(str::trim)
            .map(str::parse::<f32>)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())
            .and_then(|items| items.into_iter().collect_tuple().ok_or(String::from("Not enough arguments")))
          {
            Ok(items) => items,
            Err(fail) => {
              let mut err = clap::Error::new(clap::error::ErrorKind::ValueValidation).with_cmd(cmd);
              if let Some(arg) = arg {
                err.insert(clap::error::ContextKind::InvalidArg, clap::error::ContextValue::String(arg.to_string()));
              }
              err.insert(clap::error::ContextKind::InvalidValue, clap::error::ContextValue::String(val));
              err.insert(clap::error::ContextKind::Suggested, clap::error::ContextValue::String(fail.to_string()));
              return Err(err);
            }
          }
        }
      }
      .into(),
    )
  }

  fn possible_values(&self) -> Option<Box<dyn Iterator<Item = clap::builder::PossibleValue> + '_>> {
    Some(Box::new(
      ["D50", "D65", "custom x,y value (comma seperated)"]
        .into_iter()
        .map(clap::builder::PossibleValue::from),
    ))
  }
}

#[derive(Clone)]
pub struct ColorMatrixArg {
  matrix: Vec<f32>,
}

impl ColorMatrixArg {
  fn as_tiff_value(&self) -> tiff::Value {
    tiff::Value::SRational(self.matrix.iter().map(|x| SRational::new((x * 10_000.0) as i32, 10_000)).collect_vec())
  }
}

#[derive(Clone)]
pub struct ColorMatrixArgParser;

impl clap::builder::TypedValueParser for ColorMatrixArgParser {
  type Value = ColorMatrixArg;

  fn parse_ref(&self, cmd: &clap::Command, arg: Option<&clap::Arg>, value: &std::ffi::OsStr) -> std::result::Result<Self::Value, clap::Error> {
    let inner = NonEmptyStringValueParser::new();
    let val = inner.parse_ref(cmd, arg, value)?;

    Ok(ColorMatrixArg {
      matrix: match val.as_str() {
        "XYZ_sRGB_D50" => xyz::XYZ_TO_SRGB_D50.into_iter().flatten().collect_vec(),
        "XYZ_sRGB_D65" => xyz::XYZ_TO_SRGB_D65.into_iter().flatten().collect_vec(),
        "XYZ_AdobeRGB_D50" => xyz::XYZ_TO_ADOBERGB_D50.into_iter().flatten().collect_vec(),
        "XYZ_AdobeRGB_D65" => xyz::XYZ_TO_ADOBERGB_D65.into_iter().flatten().collect_vec(),
        _ => match val.split(',').map(str::trim).map(str::parse::<f32>).collect::<std::result::Result<Vec<_>, _>>() {
          Ok(items) => items,
          Err(fail) => {
            let mut err = clap::Error::new(clap::error::ErrorKind::ValueValidation).with_cmd(cmd);
            if let Some(arg) = arg {
              err.insert(clap::error::ContextKind::InvalidArg, clap::error::ContextValue::String(arg.to_string()));
            }
            err.insert(clap::error::ContextKind::InvalidValue, clap::error::ContextValue::String(val));
            err.insert(clap::error::ContextKind::Suggested, clap::error::ContextValue::String(fail.to_string()));
            return Err(err);
          }
        },
      },
    })
  }

  fn possible_values(&self) -> Option<Box<dyn Iterator<Item = clap::builder::PossibleValue> + '_>> {
    Some(Box::new(
      [
        "XYZ_sRGB_D50",
        "XYZ_sRGB_D65",
        "XYZ_AdobeRGB_D50",
        "XYZ_AdobeRGB_D65",
        "custom 3x3 matrix (comma seperated)",
      ]
      .into_iter()
      .map(clap::builder::PossibleValue::from),
    ))
  }
}

#[derive(Clone, Debug)]
pub struct CalibrationIlluminantArg(Illuminant);

impl CalibrationIlluminantArg {
  fn as_tiff_value(&self) -> tiff::Value {
    tiff::Value::from(u16::from(self.0))
  }
}

#[derive(Clone)]
pub struct CalibrationIlluminantArgParser;

impl clap::builder::TypedValueParser for CalibrationIlluminantArgParser {
  type Value = CalibrationIlluminantArg;

  fn parse_ref(&self, cmd: &clap::Command, arg: Option<&clap::Arg>, value: &std::ffi::OsStr) -> std::result::Result<Self::Value, clap::Error> {
    let inner = NonEmptyStringValueParser::new();
    let val = inner.parse_ref(cmd, arg, value)?;

    match Illuminant::new_from_str(&val) {
      Ok(illu) => Ok(CalibrationIlluminantArg(illu)),
      Err(fail) => {
        let mut err = clap::Error::new(clap::error::ErrorKind::ValueValidation).with_cmd(cmd);
        if let Some(arg) = arg {
          err.insert(clap::error::ContextKind::InvalidArg, clap::error::ContextValue::String(arg.to_string()));
        }
        err.insert(clap::error::ContextKind::InvalidValue, clap::error::ContextValue::String(val));
        err.insert(clap::error::ContextKind::Suggested, clap::error::ContextValue::String(fail.to_string()));
        Err(err)
      }
    }
  }

  fn possible_values(&self) -> Option<Box<dyn Iterator<Item = clap::builder::PossibleValue> + '_>> {
    Some(Box::new(
      ["Unknown", "A", "B", "C", "D50", "D55", "D65", "D75"]
        .into_iter()
        .map(clap::builder::PossibleValue::from),
    ))
  }
}

#[derive(Clone, Debug)]
pub enum DngVersion {
  V1_0,
  V1_1,
  V1_3,
  V1_2,
  V1_4,
  V1_5,
  V1_6,
}

impl DngVersion {
  fn as_dng_value(&self) -> [u8; 4] {
    match self {
      DngVersion::V1_0 => dng::DNG_VERSION_V1_0,
      DngVersion::V1_1 => dng::DNG_VERSION_V1_1,
      DngVersion::V1_3 => dng::DNG_VERSION_V1_2,
      DngVersion::V1_2 => dng::DNG_VERSION_V1_3,
      DngVersion::V1_4 => dng::DNG_VERSION_V1_4,
      DngVersion::V1_5 => dng::DNG_VERSION_V1_5,
      DngVersion::V1_6 => dng::DNG_VERSION_V1_6,
    }
  }
}

impl clap::ValueEnum for DngVersion {
  fn value_variants<'a>() -> &'a [Self] {
    &[Self::V1_0, Self::V1_1, Self::V1_2, Self::V1_3, Self::V1_4, Self::V1_5, Self::V1_6]
  }

  fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
    Some(match self {
      Self::V1_0 => clap::builder::PossibleValue::new("1.0"),
      Self::V1_1 => clap::builder::PossibleValue::new("1.1"),
      Self::V1_2 => clap::builder::PossibleValue::new("1.2"),
      Self::V1_3 => clap::builder::PossibleValue::new("1.3"),
      Self::V1_4 => clap::builder::PossibleValue::new("1.4"),
      Self::V1_5 => clap::builder::PossibleValue::new("1.5"),
      Self::V1_6 => clap::builder::PossibleValue::new("1.6"),
    })
  }
}

#[derive(Clone, Debug)]
pub enum DngColorimetricReference {
  Scene,
  Output,
}

impl DngColorimetricReference {
  fn as_dng_value(&self) -> tiff::Value {
    match self {
      Self::Scene => 0_u16.into(),
      Self::Output => 1_u16.into(),
    }
  }
}

impl clap::ValueEnum for DngColorimetricReference {
  fn value_variants<'a>() -> &'a [Self] {
    &[Self::Scene, Self::Output]
  }

  fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
    Some(match self {
      Self::Scene => clap::builder::PossibleValue::new("scene"),
      Self::Output => clap::builder::PossibleValue::new("output"),
    })
  }
}
