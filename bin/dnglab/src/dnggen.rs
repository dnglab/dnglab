// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use core::panic;
use image::{imageops::FilterType, DynamicImage, ImageBuffer};
use log::{debug, info};
use rawler::formats::tiff::{Rational, SRational};
use rawler::{
  decoders::RawDecodeParams,
  dng::rect_to_dng_area,
  formats::tiff::{CompressionMethod, DirectoryWriter, PhotometricInterpretation, PreviewColorSpace, TiffError, TiffWriter, Value},
  imgop::{raw::develop_raw_srgb, rescale_f32_to_u16, xyz::Illuminant, Dim2, Point, Rect},
  tiles::ImageTiler,
  RawFile, RawImage, RawlerError,
};
use rawler::{
  dng::{original_compress, original_digest, DNG_VERSION_V1_4},
  ljpeg92::LjpegCompressor,
  tags::{DngTag, ExifTag, LegacyTiffRootTag},
  RawImageData,
};
use rayon::prelude::*;
use std::{
  fs::File,
  io::{BufReader, BufWriter, Seek, Write},
  mem::size_of,
  thread,
  time::Instant,
  u16, usize,
};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DngError {
  #[error("{}", _0)]
  DecoderFail(String),

  #[error("{}", _0)]
  TiffFail(#[from] TiffError),
}

impl From<String> for DngError {
  fn from(what: String) -> Self {
    Self::DecoderFail(what)
  }
}

impl From<RawlerError> for DngError {
  fn from(err: RawlerError) -> Self {
    Self::DecoderFail(err.to_string()) // TODO better handling
  }
}

/// Result type for dnggen
type Result<T> = std::result::Result<T, DngError>;

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

/// Parameters for DNG conversion
#[derive(Clone, Debug)]
pub struct ConvertParams {
  pub embedded: bool,
  pub compression: DngCompression,
  pub crop: bool,
  pub predictor: u8,
  pub preview: bool,
  pub thumbnail: bool,
  pub artist: Option<String>,
  pub software: String,
  pub index: usize,
}

/// Convert a raw input file into DNG
pub fn raw_to_dng(raw_file: File, dng_file: &mut File, orig_filename: String, params: &ConvertParams) -> Result<()> {
  let mut rawfile = RawFile::new(BufReader::new(raw_file));
  //let mut rawfile = RawFile::with_box(Box::new(BufReader::new(raw_file)));
  let mut output = BufWriter::new(dng_file);

  raw_to_dng_internal(&mut rawfile, &mut output, orig_filename, &params).unwrap();

  Ok(())
}

/// Convert a raw input file into DNG
pub fn raw_to_dng_internal<W: Write + Seek + Send>(rawfile: &mut RawFile, output: &mut W, orig_filename: String, params: &ConvertParams) -> Result<()> {
  // Get decoder or return
  let mut decoder = rawler::get_decoder(rawfile).map_err(|e| DngError::DecoderFail(format!("failed to get decoder: {:?}", e)))?;

  // Compress original if requested
  let orig_compress_handle = if params.embedded {
    let in_buffer_clone = rawfile.get_buf().unwrap();
    Some(thread::spawn(move || {
      let raw_data_compreessed = original_compress(&in_buffer_clone).unwrap();
      let raw_digest = original_digest(&raw_data_compreessed);
      (raw_digest, raw_data_compreessed)
    }))
  } else {
    None
  };

  decoder.decode_metadata(rawfile)?;

  let raw_params = RawDecodeParams { image_index: params.index };

  info!("Raw image count: {}", decoder.raw_image_count()?);
  let rawimage = decoder.raw_image(rawfile, raw_params, false)?;

  let full_img = if params.preview || params.thumbnail {
    match decoder.full_image(rawfile) {
      Ok(img) => Some(img),
      Err(e) => {
        info!("No embedded image found, generate sRGB from RAW, error was: {}", e);
        let params = rawimage.develop_params()?;
        let buf = match &rawimage.data {
          RawImageData::Integer(buf) => buf,
          RawImageData::Float(_) => todo!(),
        };
        let (srgbf, dim) = develop_raw_srgb(&buf, &params)?;
        let output = rescale_f32_to_u16(&srgbf, 0, u16::MAX);
        let img = DynamicImage::ImageRgb16(ImageBuffer::from_raw(dim.w as u32, dim.h as u32, output).unwrap());
        Some(img)
      }
    }
  } else {
    None
  };

  debug!(
    "coeff: {} {} {} {}",
    rawimage.wb_coeffs[0], rawimage.wb_coeffs[1], rawimage.wb_coeffs[2], rawimage.wb_coeffs[3]
  );

  let wb_coeff = wbcoeff_to_tiff_value(&rawimage.wb_coeffs);
  let color_matrix = rawimage.color_matrix.get(&Illuminant::D65).unwrap(); // TODO fixme
  let matrix1 = matrix_to_tiff_value(color_matrix, 10_000);
  let matrix1_ill: u16 = Illuminant::D65.into();

  let mut dng = TiffWriter::new(output).unwrap();
  let mut root_ifd = dng.new_directory();

  root_ifd.add_tag(LegacyTiffRootTag::NewSubFileType, 1 as u16)?;
  if let Some(full_img) = &full_img {
    if params.thumbnail {
      dng_put_thumbnail(&mut root_ifd, &full_img).unwrap();
    }
  }

  if let Some(artist) = &params.artist {
    root_ifd.add_tag(LegacyTiffRootTag::Artist, artist)?;
  }
  root_ifd.add_tag(LegacyTiffRootTag::Software, &params.software)?;
  root_ifd.add_tag(DngTag::DNGVersion, &DNG_VERSION_V1_4[..])?;
  root_ifd.add_tag(DngTag::DNGBackwardVersion, &DNG_VERSION_V1_4[..])?;
  root_ifd.add_tag(LegacyTiffRootTag::Make, rawimage.make.as_str())?;
  root_ifd.add_tag(LegacyTiffRootTag::Model, rawimage.clean_model.as_str())?;
  let uq_model = String::from(format!("{} {}", rawimage.clean_make, rawimage.clean_model));
  root_ifd.add_tag(DngTag::UniqueCameraModel, uq_model.as_str())?;
  root_ifd.add_tag(ExifTag::ModifyDate, chrono::Local::now().format("%Y:%m:%d %H:%M:%S").to_string())?;

  // Add matrix and illumninant
  root_ifd.add_tag(DngTag::CalibrationIlluminant1, matrix1_ill)?;
  root_ifd.add_tag(DngTag::ColorMatrix1, &matrix1[..])?;

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
    decoder.populate_dng_exif(&mut exif_ifd).unwrap();
    exif_ifd.build()?
  };
  root_ifd.add_tag(LegacyTiffRootTag::ExifIFDPointer, exif_offset)?;

  // Add XPACKET (XMP) information
  if let Some(xpacket) = decoder.xpacket(rawfile) {
    //exif_ifd.write_tag_u8_array(ExifTag::ApplicationNotes, &xpacket)?;
    root_ifd.add_tag(ExifTag::ApplicationNotes, &xpacket[..])?;
  }

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
        dng_put_preview(&mut prev_image_ifd, &full_img)?;
        prev_image_ifd.build()?
      };
      sub_ifds.push(preview_offset);
    }
  }

  // Add SubIFDs
  root_ifd.add_tag(LegacyTiffRootTag::SubIFDs, &sub_ifds)?;

  // Add decoder specific entries to DNG root
  // This may override previous entries!
  decoder.populate_dng_root(&mut root_ifd).unwrap();

  // Finalize DNG file by updating IFD0 offset
  let ifd0_offset = root_ifd.build()?;
  dng.build(ifd0_offset)?;

  Ok(())
}

/// Write RAW image data into DNG
///
/// Encode raw image data as new raw IFD with NewSubFileType 0
fn dng_put_raw(raw_ifd: &mut DirectoryWriter<'_, '_>, rawimage: &RawImage, params: &ConvertParams) -> Result<()> {
  let black_level = blacklevel_to_tiff_value(&rawimage.blacklevels);
  let white_level = rawimage.whitelevels[0]; // TODO: use defaults if not available!

  // Active area or uncropped
  let active_area = if !params.crop {
    Some(Rect::new(Point::new(0, 0), Dim2::new(rawimage.width, rawimage.height)))
  } else {
    rawimage.active_area
  };

  raw_ifd.add_tag(LegacyTiffRootTag::NewSubFileType, 0 as u16)?; // Raw
  raw_ifd.add_tag(LegacyTiffRootTag::ImageWidth, rawimage.width as u32)?;
  raw_ifd.add_tag(LegacyTiffRootTag::ImageLength, rawimage.height as u32)?;
  if let Some(area) = active_area {
    let data = rect_to_dng_area(&area);
    raw_ifd.add_tag(DngTag::ActiveArea, data)?;
  }

  if let Some(crop) = rawimage.crop_area {
    let active = active_area.unwrap_or(Rect::new(Point::zero(), rawimage.dim()));
    assert!(crop.p.x >= active.p.x);
    assert!(crop.p.y >= active.p.y);
    raw_ifd.add_tag(DngTag::DefaultCropOrigin, [(crop.p.x - active.p.x) as u16, (crop.p.y - active.p.y) as u16])?;
    raw_ifd.add_tag(DngTag::DefaultCropSize, [crop.d.w as u16, crop.d.h as u16])?;
  }

  raw_ifd.add_tag(DngTag::WhiteLevel, white_level as u16)?;
  raw_ifd.add_tag(ExifTag::PlanarConfiguration, 1_u16)?;
  raw_ifd.add_tag(DngTag::DefaultScale, [Rational::new(1, 1), Rational::new(1, 1)])?;
  raw_ifd.add_tag(DngTag::BestQualityScale, Rational::new(1, 1))?;

  match rawimage.cpp {
    1 => {
      if !rawimage.blackareas.is_empty() {
        let data: Vec<u16> = rawimage.blackareas.iter().map(|area| rect_to_dng_area(area)).flatten().collect();
        raw_ifd.add_tag(DngTag::MaskedAreas, &data)?;
      }

      raw_ifd.add_tag(DngTag::BlackLevel, black_level)?;
      raw_ifd.add_tag(DngTag::BlackLevelRepeatDim, [2_u16, 2_u16])?;
      raw_ifd.add_tag(LegacyTiffRootTag::PhotometricInt, PhotometricInterpretation::CFA)?;
      raw_ifd.add_tag(LegacyTiffRootTag::SamplesPerPixel, 1_u16)?;
      raw_ifd.add_tag(LegacyTiffRootTag::BitsPerSample, [16_u16])?;

      let cfa = if let Some(area) = active_area {
        info!("CFA pattern is shifted as active area is not at CFA boundary"); // TODO false
        rawimage.cfa.shift(area.p.x, area.p.y)
      } else {
        rawimage.cfa.clone()
      };

      raw_ifd.add_tag(LegacyTiffRootTag::CFARepeatPatternDim, [cfa.width as u16, cfa.height as u16])?;
      raw_ifd.add_tag(LegacyTiffRootTag::CFAPattern, &cfa.flat_pattern()[..])?;

      //raw_ifd.add_tag(DngTag::CFAPlaneColor, [0u8, 1u8, 2u8])?; // RGB

      raw_ifd.add_tag(DngTag::CFALayout, 1 as u16)?; // Square layout

      //raw_ifd.add_tag(LegacyTiffRootTag::CFAPattern, [0u8, 1u8, 1u8, 2u8])?; // RGGB
      //raw_ifd.add_tag(LegacyTiffRootTag::CFARepeatPatternDim, [2u16, 2u16])?;
      //raw_ifd.add_tag(DngTag::CFAPlaneColor, [0u8, 1u8, 2u8])?; // RGGB
    }
    3 => {
      raw_ifd.add_tag(DngTag::BlackLevel, &black_level[0..3])?;
      raw_ifd.add_tag(DngTag::BlackLevelRepeatDim, [1_u16, 1_u16])?;
      raw_ifd.add_tag(LegacyTiffRootTag::PhotometricInt, PhotometricInterpretation::LinearRaw)?;
      raw_ifd.add_tag(LegacyTiffRootTag::SamplesPerPixel, 3_u16)?;
      raw_ifd.add_tag(LegacyTiffRootTag::BitsPerSample, [16_u16, 16_u16, 16_u16])?;

      //raw_ifd.add_tag(DngTag::CFAPlaneColor, [1u8, 2u8, 0u8])?; //
    }
    _ => {
      panic!("Unsupported");
    }
  }

  //raw_ifd.add_tag(TiffRootTag::RowsPerStrip, rawimage.height as u16)?;

  //raw_ifd.add_tag(DngTag::DefaultCropOrigin, &default_crop[..])?;
  //raw_ifd.add_tag(DngTag::DefaultCropSize, &default_size[..])?;

  match params.compression {
    DngCompression::Uncompressed => {
      raw_ifd.add_tag(LegacyTiffRootTag::Compression, CompressionMethod::None)?;
      dng_put_raw_uncompressed(raw_ifd, rawimage)?;
    }
    DngCompression::Lossless => {
      raw_ifd.add_tag(LegacyTiffRootTag::Compression, CompressionMethod::ModernJPEG)?;
      dng_put_raw_ljpeg(raw_ifd, rawimage, params.predictor)?;
    }
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

      let tiled_data: Vec<Vec<u16>> = ImageTiler::new(&data, rawimage.width, rawimage.height, rawimage.cpp, tile_w, tile_h).collect();

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
  raw_ifd.add_tag(LegacyTiffRootTag::TileOffsets, &tile_offsets)?;
  raw_ifd.add_tag(LegacyTiffRootTag::TileByteCounts, &tile_sizes)?;
  //raw_ifd.add_tag(LegacyTiffRootTag::TileWidth, lj92_data.1 as u16)?; // FIXME
  //raw_ifd.add_tag(LegacyTiffRootTag::TileLength, lj92_data.2 as u16)?;
  raw_ifd.add_tag(LegacyTiffRootTag::TileWidth, tile_w as u16)?; // FIXME
  raw_ifd.add_tag(LegacyTiffRootTag::TileLength, tile_h as u16)?;

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

      raw_ifd.add_tag(LegacyTiffRootTag::StripOffsets, &strip_offsets)?;
      raw_ifd.add_tag(LegacyTiffRootTag::StripByteCounts, &strip_sizes)?;
      raw_ifd.add_tag(LegacyTiffRootTag::RowsPerStrip, &strip_rows)?;
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

  ifd.add_tag(LegacyTiffRootTag::ImageWidth, thumb_img.width() as u32)?;
  ifd.add_tag(LegacyTiffRootTag::ImageLength, thumb_img.height() as u32)?;
  ifd.add_tag(LegacyTiffRootTag::Compression, CompressionMethod::None)?;
  ifd.add_tag(LegacyTiffRootTag::BitsPerSample, 8_u16)?;
  ifd.add_tag(LegacyTiffRootTag::SampleFormat, [1_u16, 1, 1])?;
  ifd.add_tag(LegacyTiffRootTag::PhotometricInt, PhotometricInterpretation::RGB)?;
  ifd.add_tag(LegacyTiffRootTag::SamplesPerPixel, 3_u16)?;
  //ifd.add_tag(TiffRootTag::XResolution, Rational { n: 1, d: 1 })?;
  //ifd.add_tag(TiffRootTag::YResolution, Rational { n: 1, d: 1 })?;
  //ifd.add_tag(TiffRootTag::ResolutionUnit, ResolutionUnit::None.to_u16())?;

  let offset = ifd.write_data(&thumb_img)?;

  ifd.add_tag(LegacyTiffRootTag::StripOffsets, offset)?;
  ifd.add_tag(LegacyTiffRootTag::StripByteCounts, thumb_img.len() as u32)?;
  ifd.add_tag(LegacyTiffRootTag::RowsPerStrip, thumb_img.height() as u32)?;

  Ok(())
}

fn dng_put_preview(ifd: &mut DirectoryWriter<'_, '_>, img: &DynamicImage) -> Result<()> {
  let now = Instant::now();
  let preview_img = DynamicImage::ImageRgb8(img.resize(1024, 768, FilterType::Nearest).to_rgb8());
  debug!("preview downscale: {} s", now.elapsed().as_secs_f32());

  ifd.add_tag(LegacyTiffRootTag::NewSubFileType, 1 as u16)?;
  ifd.add_tag(LegacyTiffRootTag::ImageWidth, preview_img.width() as u32)?;
  ifd.add_tag(LegacyTiffRootTag::ImageLength, preview_img.height() as u32)?;
  ifd.add_tag(LegacyTiffRootTag::Compression, CompressionMethod::ModernJPEG)?;
  ifd.add_tag(LegacyTiffRootTag::BitsPerSample, 8_u16)?;
  ifd.add_tag(LegacyTiffRootTag::SampleFormat, [1_u16, 1, 1])?;
  ifd.add_tag(LegacyTiffRootTag::PhotometricInt, PhotometricInterpretation::YCbCr)?;
  ifd.add_tag(LegacyTiffRootTag::RowsPerStrip, preview_img.height() as u32)?;
  ifd.add_tag(LegacyTiffRootTag::SamplesPerPixel, 3_u16)?;
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

  ifd.add_value(LegacyTiffRootTag::StripOffsets, Value::Long(vec![offset]))?;
  ifd.add_tag(LegacyTiffRootTag::StripByteCounts, [data_len as u32])?;

  Ok(())
}

fn blacklevel_to_tiff_value(blacklevel: &[u16; 4]) -> [SRational; 4] {
  [
    SRational::new(blacklevel[0] as i32, 1),
    SRational::new(blacklevel[1] as i32, 1),
    SRational::new(blacklevel[2] as i32, 1),
    SRational::new(blacklevel[3] as i32, 1),
  ]
}

fn wbcoeff_to_tiff_value(wb_coeffs: &[f32; 4]) -> [Rational; 3] {
  [
    Rational::new_f32(1.0 / (wb_coeffs[0] / 1024.0), 1000000),
    Rational::new_f32(1.0 / (wb_coeffs[1] / 1024.0), 1000000),
    Rational::new_f32(1.0 / (wb_coeffs[2] / 1024.0), 1000000),
    //Rational::new_f32(1.0 / (wb_coeffs[3] / 1024.0), 1000000),
  ]
}

fn matrix_to_tiff_value(xyz_to_cam: &Vec<f32>, d: i32) -> Vec<SRational> {
  xyz_to_cam.iter().map(|a| SRational::new((a * d as f32) as i32, d)).collect()
}
