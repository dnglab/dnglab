// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use crate::exif::Exif;
use crate::formats::tiff::{Rational, SRational};
use crate::imgop::xyz::Illuminant;
use crate::tags::{ExifGpsTag, TiffTag};
use crate::Result;
use crate::{
  decoders::RawDecodeParams,
  formats::tiff::{CompressionMethod, DirectoryWriter, PhotometricInterpretation, PreviewColorSpace, TiffWriter, Value},
  imgop::{raw::develop_raw_srgb, rescale_f32_to_u16},
  tiles::ImageTiler,
  RawFile, RawImage,
};
use crate::{
  dng::{original_compress, original_digest, DNG_VERSION_V1_4},
  ljpeg92::LjpegCompressor,
  tags::{DngTag, ExifTag, TiffCommonTag},
  RawImageData,
};
use core::panic;
use image::{imageops::FilterType, DynamicImage, ImageBuffer};
use log::{debug, error, info};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::{
  fs::File,
  io::{BufReader, BufWriter, Seek, Write},
  mem::size_of,
  thread,
  time::Instant,
  u16, usize,
};

mod raw_writer;

/// Quality of preview images
const PREVIEW_JPEG_QUALITY: f32 = 0.75;
#[derive(Clone, Debug)]
/// Compression mode for DNG
pub enum DngCompression {
  /// No compression is applied
  Uncompressed,
  /// Lossless JPEG-92 compression
  Lossless,
  // Lossy
}

#[derive(Clone, Copy, Debug)]
pub enum DngPhotometricCoversion {
  Original,
  Linear,
}

impl Default for DngPhotometricCoversion {
  fn default() -> Self {
    Self::Original
  }
}

#[derive(Clone, Copy, Debug)]
pub enum CropMode {
  Best,
  ActiveArea,
  None,
}

impl FromStr for CropMode {
  type Err = String;

  fn from_str(mode: &str) -> std::result::Result<Self, Self::Err> {
    Ok(match mode {
      "best" => Self::Best,
      "activearea" => Self::ActiveArea,
      "none" => Self::None,
      _ => return Err(format!("Unknown CropMode value: {}", mode)),
    })
  }
}

/// Parameters for DNG conversion
#[derive(Clone, Debug)]
pub struct ConvertParams {
  pub embedded: bool,
  pub compression: DngCompression,
  pub photometric_conversion: DngPhotometricCoversion,
  pub crop: CropMode,
  pub predictor: u8,
  pub preview: bool,
  pub thumbnail: bool,
  pub artist: Option<String>,
  pub software: String,
  pub index: usize,
}

/// Convert a raw input file into DNG
pub fn raw_to_dng(path: &Path, raw_file: File, dng_file: &mut File, orig_filename: String, params: &ConvertParams) -> Result<()> {
  let mut rawfile = RawFile::new(PathBuf::from(path), BufReader::new(raw_file));
  let mut output = BufWriter::new(dng_file);
  raw_to_dng_internal(&mut rawfile, &mut output, orig_filename, params)?;
  Ok(())
}

/// Convert a raw input file into DNG
pub fn raw_to_dng_internal<W: Write + Seek + Send>(rawfile: &mut RawFile, output: &mut W, orig_filename: String, params: &ConvertParams) -> Result<()> {
  // Get decoder or return
  let decoder = crate::get_decoder(rawfile)?;

  // Compress original if requested
  let orig_compress_handle = if params.embedded {
    let in_buffer_clone = rawfile.as_vec().unwrap();
    Some(thread::spawn(move || {
      let raw_data_compreessed = original_compress(&in_buffer_clone).unwrap();
      let raw_digest = original_digest(&raw_data_compreessed);
      (raw_digest, raw_data_compreessed)
    }))
  } else {
    None
  };

  //decoder.decode_metadata(rawfile)?;

  let raw_params = RawDecodeParams { image_index: params.index };

  info!("Raw image count: {}", decoder.raw_image_count()?);
  let rawimage = decoder.raw_image(rawfile, raw_params.clone(), false)?;
  let metadata = decoder.raw_metadata(rawfile, raw_params.clone())?;

  let full_img = if params.preview || params.thumbnail {
    let image = match decoder.full_image(rawfile) {
      Ok(Some(img)) => Some(img),
      Ok(None) => {
        info!("No embedded image found, generate sRGB from RAW");
        None
      }
      Err(e) => {
        error!("No embedded image found, generate sRGB from RAW, error was: {}", e);
        None
      }
    };
    if image.is_some() {
      image
    } else {
      match rawimage.develop_params() {
        Ok(params) => {
          let buf = match &rawimage.data {
            RawImageData::Integer(buf) => buf,
            RawImageData::Float(_) => todo!(),
          };
          let (srgbf, dim) = develop_raw_srgb(buf, &params)?;
          let output = rescale_f32_to_u16(&srgbf, 0, u16::MAX);
          let img = DynamicImage::ImageRgb16(ImageBuffer::from_raw(dim.w as u32, dim.h as u32, output).expect("Invalid ImageBuffer size"));
          Some(img)
        }
        Err(err) => {
          log::error!("{}", err);
          None
        }
      }
    }
  } else {
    None
  };

  debug!(
    "coeff: {} {} {} {}",
    rawimage.wb_coeffs[0], rawimage.wb_coeffs[1], rawimage.wb_coeffs[2], rawimage.wb_coeffs[3]
  );

  // The count of elements depends on unique colors in CFA and can
  // automatically added to the IFD.
  let wb_coeff = wbcoeff_to_tiff_value(&rawimage);

  let mut dng = TiffWriter::new(output).unwrap();
  let mut root_ifd = dng.new_directory();

  fill_exif_root(&mut root_ifd, &metadata.exif)?;

  if let Some(id) = &metadata.unique_image_id {
    root_ifd.add_tag(DngTag::RawDataUniqueID, id.to_le_bytes())?;
  }

  // Add XPACKET (XMP) information
  if let Some(xpacket) = decoder.xpacket(rawfile, raw_params)? {
    root_ifd.add_tag(ExifTag::ApplicationNotes, &xpacket[..])?;
  }

  root_ifd.add_tag(TiffCommonTag::NewSubFileType, 1_u16)?;
  if let Some(full_img) = &full_img {
    if params.thumbnail {
      dng_put_thumbnail(&mut root_ifd, full_img).unwrap();
    }
  }

  if let Some(artist) = &params.artist {
    root_ifd.add_tag(TiffCommonTag::Artist, artist)?;
  }
  root_ifd.add_tag(TiffCommonTag::Software, &params.software)?;
  root_ifd.add_tag(DngTag::DNGVersion, &DNG_VERSION_V1_4[..])?;
  root_ifd.add_tag(DngTag::DNGBackwardVersion, &DNG_VERSION_V1_4[..])?;
  root_ifd.add_tag(TiffCommonTag::Make, rawimage.clean_make.as_str())?;
  root_ifd.add_tag(TiffCommonTag::Model, rawimage.clean_model.as_str())?;
  let uq_model = format!("{} {}", rawimage.clean_make, rawimage.clean_model);
  root_ifd.add_tag(DngTag::UniqueCameraModel, uq_model.as_str())?;
  root_ifd.add_tag(ExifTag::ModifyDate, chrono::Local::now().format("%Y:%m:%d %H:%M:%S").to_string())?;

  // Add matrix and illumninant
  let mut available_matrices = rawimage.color_matrix.clone();
  if let Some(first_key) = available_matrices.keys().next().cloned() {
    let first_matrix = available_matrices
      .remove_entry(&Illuminant::A)
      .or_else(|| available_matrices.remove_entry(&Illuminant::A))
      .or_else(|| available_matrices.remove_entry(&first_key))
      .expect("No matrix found");
    root_ifd.add_tag(DngTag::CalibrationIlluminant1, u16::from(first_matrix.0))?;
    root_ifd.add_tag(DngTag::ColorMatrix1, &first_matrix.1[..])?;

    if let Some(second_matrix) = available_matrices
      .remove_entry(&Illuminant::D65)
      .or_else(|| available_matrices.remove_entry(&Illuminant::D50))
    {
      root_ifd.add_tag(DngTag::CalibrationIlluminant2, u16::from(second_matrix.0))?;
      root_ifd.add_tag(DngTag::ColorMatrix2, &second_matrix.1[..])?;
    }
  }

  // Add White balance info
  root_ifd.add_tag(DngTag::AsShotNeutral, &wb_coeff[..])?;

  // If compression thread handle is available, embed original file
  if let Some(handle) = orig_compress_handle {
    let (raw_digest, raw_data_compreessed) = handle.join().unwrap();
    root_ifd.add_tag_undefined(DngTag::OriginalRawFileData, raw_data_compreessed)?;
    root_ifd.add_tag(DngTag::OriginalRawFileName, orig_filename)?;
    root_ifd.add_tag(DngTag::OriginalRawFileDigest, raw_digest)?;
  }

  // Add EXIF information
  let exif_offset = {
    let mut exif_ifd = root_ifd.new_directory();
    // Add EXIF version 0220
    exif_ifd.add_tag_undefined(ExifTag::ExifVersion, vec![48, 50, 50, 48])?;
    fill_exif_ifd(&mut exif_ifd, &metadata.exif)?;
    //decoder.populate_dng_exif(&mut exif_ifd).unwrap();
    exif_ifd.build()?
  };
  root_ifd.add_tag(TiffCommonTag::ExifIFDPointer, exif_offset)?;

  let mut sub_ifds = Vec::new();

  // Add raw image
  let raw_offset = {
    let mut raw_ifd = root_ifd.new_directory();
    dng_put_raw(&mut raw_ifd, &rawimage, params)?;
    raw_ifd.build()?
  };
  sub_ifds.push(raw_offset);

  if let Some(full_img) = &full_img {
    if params.preview {
      // Add preview image
      let preview_offset = {
        let mut prev_image_ifd = root_ifd.new_directory();
        dng_put_preview(&mut prev_image_ifd, full_img)?;
        prev_image_ifd.build()?
      };
      sub_ifds.push(preview_offset);
    }
  }

  // Add SubIFDs
  root_ifd.add_tag(TiffCommonTag::SubIFDs, &sub_ifds)?;

  // Add decoder specific entries to DNG root
  // This may override previous entries!
  //decoder.populate_dng_root(&mut root_ifd).unwrap();

  // Finalize DNG file by updating IFD0 offset
  let ifd0_offset = root_ifd.build()?;
  dng.build(ifd0_offset)?;

  Ok(())
}

fn transfer_entry<T, V>(raw_ifd: &mut DirectoryWriter, tag: T, entry: &Option<V>) -> Result<()>
where
  T: TiffTag,
  V: Into<Value> + Clone,
{
  if let Some(entry) = entry {
    raw_ifd.add_tag(tag, entry.clone())?;
  }
  Ok(())
}

fn transfer_entry_undefined<T>(raw_ifd: &mut DirectoryWriter, tag: T, entry: &Option<Vec<u8>>) -> Result<()>
where
  T: TiffTag,
{
  if let Some(entry) = entry {
    raw_ifd.add_tag_undefined(tag, entry.clone())?;
  }
  Ok(())
}

fn fill_exif_root(raw_ifd: &mut DirectoryWriter, exif: &Exif) -> Result<()> {
  transfer_entry(raw_ifd, ExifTag::Orientation, &exif.orientation)?;
  transfer_entry(raw_ifd, ExifTag::ModifyDate, &exif.modify_date)?;
  transfer_entry(raw_ifd, ExifTag::Copyright, &exif.copyright)?;
  transfer_entry(raw_ifd, ExifTag::Artist, &exif.artist)?;

  // DNG has a lens info tag that is identical to the LensSpec tag in EXIF IFD
  transfer_entry(raw_ifd, DngTag::LensInfo, &exif.lens_spec)?;

  if let Some(gps) = &exif.gps {
    let gps_offset = {
      let mut gps_ifd = raw_ifd.new_directory();
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSVersionID, &gps.gps_version_id)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSLatitudeRef, &gps.gps_latitude_ref)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSLatitude, &gps.gps_latitude)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSLongitudeRef, &gps.gps_longitude_ref)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSLongitude, &gps.gps_longitude)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSAltitudeRef, &gps.gps_altitude_ref)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSAltitude, &gps.gps_altitude)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSTimeStamp, &gps.gps_timestamp)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSSatellites, &gps.gps_satellites)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSStatus, &gps.gps_status)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSMeasureMode, &gps.gps_measure_mode)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDOP, &gps.gps_dop)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSSpeedRef, &gps.gps_speed_ref)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSSpeed, &gps.gps_speed)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSTrackRef, &gps.gps_track_ref)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSTrack, &gps.gps_track)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSImgDirectionRef, &gps.gps_img_direction_ref)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSImgDirection, &gps.gps_img_direction)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSMapDatum, &gps.gps_map_datum)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDestLatitudeRef, &gps.gps_dest_latitude_ref)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDestLatitude, &gps.gps_dest_latitude)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDestLongitudeRef, &gps.gps_dest_longitude_ref)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDestLongitude, &gps.gps_dest_longitude)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDestBearingRef, &gps.gps_dest_bearing_ref)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDestBearing, &gps.gps_dest_bearing)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDestDistanceRef, &gps.gps_dest_distance_ref)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDestDistance, &gps.gps_dest_distance)?;
      transfer_entry_undefined(&mut gps_ifd, ExifGpsTag::GPSProcessingMethod, &gps.gps_processing_method)?;
      transfer_entry_undefined(&mut gps_ifd, ExifGpsTag::GPSAreaInformation, &gps.gps_area_information)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDateStamp, &gps.gps_date_stamp)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDifferential, &gps.gps_differential)?;
      transfer_entry(&mut gps_ifd, ExifGpsTag::GPSHPositioningError, &gps.gps_h_positioning_error)?;
      if gps_ifd.entry_count() > 0 {
        Some(gps_ifd.build()?)
      } else {
        None
      }
    };
    if let Some(gps_offset) = gps_offset {
      raw_ifd.add_tag(ExifTag::GPSInfo, [gps_offset])?;
    }
  }

  Ok(())
}

fn fill_exif_ifd(exif_ifd: &mut DirectoryWriter, exif: &Exif) -> Result<()> {
  transfer_entry(exif_ifd, ExifTag::FNumber, &exif.fnumber)?;
  transfer_entry(exif_ifd, ExifTag::ApertureValue, &exif.aperture_value)?;
  transfer_entry(exif_ifd, ExifTag::BrightnessValue, &exif.brightness_value)?;
  transfer_entry(exif_ifd, ExifTag::RecommendedExposureIndex, &exif.recommended_exposure_index)?;
  transfer_entry(exif_ifd, ExifTag::ExposureTime, &exif.exposure_time)?;
  transfer_entry(exif_ifd, ExifTag::ISOSpeedRatings, &exif.iso_speed_ratings)?;
  transfer_entry(exif_ifd, ExifTag::ISOSpeed, &exif.iso_speed)?;
  transfer_entry(exif_ifd, ExifTag::SensitivityType, &exif.sensitivity_type)?;
  transfer_entry(exif_ifd, ExifTag::ExposureProgram, &exif.exposure_program)?;
  transfer_entry(exif_ifd, ExifTag::TimeZoneOffset, &exif.timezone_offset)?;
  transfer_entry(exif_ifd, ExifTag::DateTimeOriginal, &exif.date_time_original)?;
  transfer_entry(exif_ifd, ExifTag::CreateDate, &exif.create_date)?;
  transfer_entry(exif_ifd, ExifTag::OffsetTime, &exif.offset_time)?;
  transfer_entry(exif_ifd, ExifTag::OffsetTimeOriginal, &exif.offset_time_original)?;
  transfer_entry(exif_ifd, ExifTag::OffsetTimeDigitized, &exif.offset_time_digitized)?;
  transfer_entry(exif_ifd, ExifTag::SubSecTime, &exif.sub_sec_time)?;
  transfer_entry(exif_ifd, ExifTag::SubSecTimeOriginal, &exif.sub_sec_time_original)?;
  transfer_entry(exif_ifd, ExifTag::SubSecTimeDigitized, &exif.sub_sec_time_digitized)?;
  transfer_entry(exif_ifd, ExifTag::ShutterSpeedValue, &exif.shutter_speed_value)?;
  transfer_entry(exif_ifd, ExifTag::MaxApertureValue, &exif.max_aperture_value)?;
  transfer_entry(exif_ifd, ExifTag::SubjectDistance, &exif.subject_distance)?;
  transfer_entry(exif_ifd, ExifTag::MeteringMode, &exif.metering_mode)?;
  transfer_entry(exif_ifd, ExifTag::LightSource, &exif.light_source)?;
  transfer_entry(exif_ifd, ExifTag::Flash, &exif.flash)?;
  transfer_entry(exif_ifd, ExifTag::FocalLength, &exif.focal_length)?;
  transfer_entry(exif_ifd, ExifTag::ImageNumber, &exif.image_number)?;
  transfer_entry(exif_ifd, ExifTag::ColorSpace, &exif.color_space)?;
  transfer_entry(exif_ifd, ExifTag::FlashEnergy, &exif.flash_energy)?;
  transfer_entry(exif_ifd, ExifTag::ExposureMode, &exif.exposure_mode)?;
  transfer_entry(exif_ifd, ExifTag::WhiteBalance, &exif.white_balance)?;
  transfer_entry(exif_ifd, ExifTag::SceneCaptureType, &exif.scene_capture_type)?;
  transfer_entry(exif_ifd, ExifTag::SubjectDistanceRange, &exif.subject_distance_range)?;
  transfer_entry(exif_ifd, ExifTag::OwnerName, &exif.owner_name)?;
  transfer_entry(exif_ifd, ExifTag::SerialNumber, &exif.serial_number)?;
  transfer_entry(exif_ifd, ExifTag::LensSerialNumber, &exif.lens_serial_number)?;
  transfer_entry(exif_ifd, ExifTag::LensSpecification, &exif.lens_spec)?;
  transfer_entry(exif_ifd, ExifTag::LensMake, &exif.lens_make)?;
  transfer_entry(exif_ifd, ExifTag::LensModel, &exif.lens_model)?;

  Ok(())
}

/// Write RAW image data into DNG
///
/// Encode raw image data as new raw IFD with NewSubFileType 0
fn dng_put_raw(raw_ifd: &mut DirectoryWriter<'_, '_>, rawimage: &RawImage, params: &ConvertParams) -> Result<()> {
  match params.photometric_conversion {
    DngPhotometricCoversion::Original => raw_writer::write_rawimage(raw_ifd, rawimage, params)?,

    DngPhotometricCoversion::Linear => {
      if rawimage.cpp == 3 {
        raw_writer::write_rawimage(raw_ifd, rawimage, params)?;
      } else {
        let rawimage = rawimage.linearize()?;
        raw_writer::write_rawimage(raw_ifd, &rawimage, params)?;
      }
    }
  }

  for (tag, value) in rawimage.dng_tags.iter() {
    raw_ifd.add_untyped_tag(*tag, value.clone())?;
  }

  Ok(())
}

/// Compress RAW image with LJPEG-92
///
/// Data is split into multiple tiles, each tile is compressed seperately
///
/// Predictor mode 4,5,6,7 is best for images where two images
/// lines are merged, because then the image bayer pattern is:
/// RGRGGBGB
/// RGRGGBGB
/// Instead of the default:
/// RGRG
/// GBGB
/// RGRG
/// GBGB
fn dng_put_raw_ljpeg(raw_ifd: &mut DirectoryWriter<'_, '_>, rawimage: &RawImage, predictor: u8) -> Result<()> {
  let tile_w = 256 & !0b111; // ensure div 16
  let tile_h = 256 & !0b111;

  let lj92_data = match rawimage.data {
    RawImageData::Integer(ref data) => {
      // Inject black pixel data for testing purposes.
      // let data = vec![0x0000; data.len()];
      //let tiled_data = TiledData::new(&data, rawimage.width, rawimage.height, rawimage.cpp);

      // Only merge two lines into one for higher predictors, if image is CFA
      let realign = if (4..=7).contains(&predictor) && rawimage.cfa.width == 2 && rawimage.cfa.height == 2 {
        2
      } else {
        1
      };

      let tiled_data: Vec<Vec<u16>> = ImageTiler::new(data, rawimage.width, rawimage.height, rawimage.cpp, tile_w, tile_h).collect();

      let j_height = tile_h;
      let (j_width, components) = if rawimage.cpp == 3 {
        (tile_w, 3) /* RGB LinearRaw */
      } else {
        (tile_w / 2, 2) /* CFA */
      };

      debug!("LJPEG compression: bit depth: {}", rawimage.bps);

      let tiles_compr: Vec<Vec<u8>> = tiled_data
        .par_iter()
        .map(|tile| {
          //assert_eq!((tile_w * rawimage.cpp) % components, 0);
          //assert_eq!((tile_w * rawimage.cpp) % 2, 0);
          //assert_eq!(tile_h % 2, 0);
          let state = LjpegCompressor::new(tile, j_width * realign, j_height / realign, components, rawimage.bps as u8, predictor, 0, 0).unwrap();
          state.encode().unwrap()
        })
        .collect();
      tiles_compr
    }
    RawImageData::Float(ref _data) => {
      panic!("invalid format");
    }
  };

  let mut tile_offsets: Vec<u32> = Vec::new();
  let mut tile_sizes: Vec<u32> = Vec::new();

  lj92_data.iter().for_each(|tile| {
    let offs = raw_ifd.write_data(tile).unwrap();
    tile_offsets.push(offs);
    tile_sizes.push((tile.len() * size_of::<u8>()) as u32);
  });

  //let offs = raw_ifd.write_data(&lj92_data)?;
  raw_ifd.add_tag(TiffCommonTag::TileOffsets, &tile_offsets)?;
  raw_ifd.add_tag(TiffCommonTag::TileByteCounts, &tile_sizes)?;
  //raw_ifd.add_tag(LegacyTiffRootTag::TileWidth, lj92_data.1 as u16)?; // FIXME
  //raw_ifd.add_tag(LegacyTiffRootTag::TileLength, lj92_data.2 as u16)?;
  raw_ifd.add_tag(TiffCommonTag::TileWidth, tile_w as u16)?; // FIXME
  raw_ifd.add_tag(TiffCommonTag::TileLength, tile_h as u16)?;

  Ok(())
}

/// Write RAW uncompressed into DNG
///
/// This uses unsigned 16 bit values for storage
/// Data is split into multiple strips
fn dng_put_raw_uncompressed(raw_ifd: &mut DirectoryWriter<'_, '_>, rawimage: &RawImage) -> Result<()> {
  match rawimage.data {
    RawImageData::Integer(ref data) => {
      let mut strip_offsets: Vec<u32> = Vec::new();
      let mut strip_sizes: Vec<u32> = Vec::new();
      let mut strip_rows: Vec<u32> = Vec::new();

      // 8 Strips
      let rows_per_strip = rawimage.height / 8;

      for strip in data.chunks(rows_per_strip * rawimage.width * rawimage.cpp) {
        let offset = raw_ifd.write_data_u16_be(strip)?;
        strip_offsets.push(offset);
        strip_sizes.push((strip.len() * size_of::<u16>()) as u32);
        strip_rows.push((strip.len() / (rawimage.width * rawimage.cpp)) as u32);
      }

      raw_ifd.add_tag(TiffCommonTag::StripOffsets, &strip_offsets)?;
      raw_ifd.add_tag(TiffCommonTag::StripByteCounts, &strip_sizes)?;
      raw_ifd.add_tag(TiffCommonTag::RowsPerStrip, &strip_rows)?;
    }
    RawImageData::Float(ref _data) => {
      panic!("invalid format");
    }
  };

  Ok(())
}

/// Write thumbnail image into DNG
fn dng_put_thumbnail(ifd: &mut DirectoryWriter<'_, '_>, img: &DynamicImage) -> Result<()> {
  let thumb_img = img.resize(240, 120, FilterType::Nearest).to_rgb8();

  ifd.add_tag(TiffCommonTag::ImageWidth, thumb_img.width() as u32)?;
  ifd.add_tag(TiffCommonTag::ImageLength, thumb_img.height() as u32)?;
  ifd.add_tag(TiffCommonTag::Compression, CompressionMethod::None)?;
  ifd.add_tag(TiffCommonTag::BitsPerSample, 8_u16)?;
  ifd.add_tag(TiffCommonTag::SampleFormat, [1_u16, 1, 1])?;
  ifd.add_tag(TiffCommonTag::PhotometricInt, PhotometricInterpretation::RGB)?;
  ifd.add_tag(TiffCommonTag::SamplesPerPixel, 3_u16)?;
  //ifd.add_tag(TiffRootTag::XResolution, Rational { n: 1, d: 1 })?;
  //ifd.add_tag(TiffRootTag::YResolution, Rational { n: 1, d: 1 })?;
  //ifd.add_tag(TiffRootTag::ResolutionUnit, ResolutionUnit::None.to_u16())?;

  let offset = ifd.write_data(&thumb_img)?;

  ifd.add_tag(TiffCommonTag::StripOffsets, offset)?;
  ifd.add_tag(TiffCommonTag::StripByteCounts, thumb_img.len() as u32)?;
  ifd.add_tag(TiffCommonTag::RowsPerStrip, thumb_img.height() as u32)?;

  Ok(())
}

fn dng_put_preview(ifd: &mut DirectoryWriter<'_, '_>, img: &DynamicImage) -> Result<()> {
  let now = Instant::now();
  let preview_img = DynamicImage::ImageRgb8(img.resize(1024, 768, FilterType::Nearest).to_rgb8());
  debug!("preview downscale: {} s", now.elapsed().as_secs_f32());

  ifd.add_tag(TiffCommonTag::NewSubFileType, 1_u16)?;
  ifd.add_tag(TiffCommonTag::ImageWidth, preview_img.width() as u32)?;
  ifd.add_tag(TiffCommonTag::ImageLength, preview_img.height() as u32)?;
  ifd.add_tag(TiffCommonTag::Compression, CompressionMethod::ModernJPEG)?;
  ifd.add_tag(TiffCommonTag::BitsPerSample, 8_u16)?;
  ifd.add_tag(TiffCommonTag::SampleFormat, [1_u16, 1, 1])?;
  ifd.add_tag(TiffCommonTag::PhotometricInt, PhotometricInterpretation::YCbCr)?;
  ifd.add_tag(TiffCommonTag::RowsPerStrip, preview_img.height() as u32)?;
  ifd.add_tag(TiffCommonTag::SamplesPerPixel, 3_u16)?;
  ifd.add_tag(DngTag::PreviewColorSpace, PreviewColorSpace::SRgb)?; // ??

  //ifd.add_tag(TiffRootTag::XResolution, Rational { n: 1, d: 1 })?;
  //ifd.add_tag(TiffRootTag::YResolution, Rational { n: 1, d: 1 })?;
  //ifd.add_tag(TiffRootTag::ResolutionUnit, ResolutionUnit::None.to_u16())?;

  let now = Instant::now();
  let offset = ifd.tiff.position()?;

  preview_img
    .write_to(
      &mut ifd.tiff.writer,
      image::ImageOutputFormat::Jpeg((PREVIEW_JPEG_QUALITY * u8::MAX as f32) as u8),
    )
    .unwrap();
  let data_len = ifd.tiff.position()? - offset;
  debug!("writing preview: {} s", now.elapsed().as_secs_f32());

  ifd.add_value(TiffCommonTag::StripOffsets, Value::Long(vec![offset]))?;
  ifd.add_tag(TiffCommonTag::StripByteCounts, [data_len as u32])?;

  Ok(())
}

/// DNG requires the WB values to be the reciprocal
fn wbcoeff_to_tiff_value(rawimage: &RawImage) -> Vec<Rational> {
  assert!([3, 4].contains(&rawimage.cfa.unique_colors()));
  let wb = &rawimage.wb_coeffs;
  let mut values = Vec::with_capacity(4);

  values.push(Rational::new_f32(1.0 / wb[0], 100000));
  values.push(Rational::new_f32(1.0 / wb[1], 100000));
  values.push(Rational::new_f32(1.0 / wb[2], 100000));

  if rawimage.cfa.unique_colors() == 4 {
    values.push(Rational::new_f32(1.0 / wb[3], 100000));
  }
  values
}

fn matrix_to_tiff_value(xyz_to_cam: &[f32], d: i32) -> Vec<SRational> {
  xyz_to_cam.iter().map(|a| SRational::new((a * d as f32) as i32, d)).collect()
}
