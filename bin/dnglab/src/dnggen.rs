// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use core::panic;
use image::{imageops::FilterType, DynamicImage, GenericImageView};
use log::debug;
use rawler::{
  dng::dng_active_area,
  tiff::{CompressionMethod, DirectoryWriter, PhotometricInterpretation, PreviewColorSpace, TiffError, Value},
  RawImage,
};
use rawler::{
  dng::{original_compress, original_digest, DNG_VERSION_V1_4},
  ljpeg92::LjpegCompressor,
  tags::{DngTag, ExifTag, TiffRootTag},
  tiff::TiffWriter,
  Buffer, RawImageData,
};
use rawler::{
  tiff::{Rational, SRational},
  tiles::TiledData,
};
use rayon::prelude::*;
use std::{
  fs::File,
  io::{BufReader, BufWriter},
  mem::size_of,
  rc::Rc,
  sync::Arc,
  thread,
  time::Instant,
  u16,
};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DngError {
  #[error("Decoder failed: _0")]
  DecoderFail(String),

  #[error("Decoder failed: _0")]
  TiffFail(#[from] TiffError),
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
pub struct DngParams {
  pub no_embedded: bool,
  pub compression: DngCompression,
  pub no_crop: bool,
  pub software: String,
}

/// Convert a raw input file into DNG
pub fn raw_to_dng(raw_file: &mut File, dng_file: &mut File, orig_filename: &str, params: &DngParams) -> Result<()> {
  let mut raw_file = BufReader::new(raw_file);
  let mut dng_file = BufWriter::new(dng_file);

  // Read whole raw file
  // TODO: Large input file bug, we need to test the raw file before open it
  let in_buffer = Arc::new(Buffer::new(&mut raw_file).unwrap());

  // Get decoder or return
  let mut decoder = rawler::get_decoder(&in_buffer).map_err(|_s| DngError::DecoderFail("failed".into()))?;

  // Compress original if requested
  let orig_compress_handle = if !params.no_embedded {
    let in_buffer_clone = in_buffer.clone();
    Some(thread::spawn(move || {
      let raw_digest = original_digest(in_buffer_clone.raw_buf());
      let raw_data_compreessed = original_compress(in_buffer_clone.raw_buf()).unwrap();
      (raw_digest, raw_data_compreessed)
    }))
  } else {
    None
  };

  decoder.decode_metadata().unwrap();

  let full_img = decoder.full_image().unwrap();
  let rawimage = decoder.raw_image(false).unwrap();

  debug!(
    "coeff: {} {} {} {}",
    rawimage.wb_coeffs[0], rawimage.wb_coeffs[1], rawimage.wb_coeffs[2], rawimage.wb_coeffs[3]
  );

  let wb_coeff = wbcoeff_to_tiff_value(&rawimage.wb_coeffs);

  let matrix1 = matrix_to_tiff_value(&rawimage.xyz_to_cam2, rawimage.illuminant2_denominator);
  let matrix1_ill = rawimage.illuminant2;

  let mut dng = TiffWriter::new(&mut dng_file).unwrap();
  let mut root_ifd = dng.new_directory();

  dng_put_thumbnail(&mut root_ifd, &full_img).unwrap();

  root_ifd.add_tag(TiffRootTag::Software, &params.software)?;
  root_ifd.add_tag(DngTag::DNGVersion, &DNG_VERSION_V1_4[..])?;
  root_ifd.add_tag(DngTag::DNGBackwardVersion, &DNG_VERSION_V1_4[..])?;
  root_ifd.add_tag(TiffRootTag::Make, rawimage.make.as_str())?;
  root_ifd.add_tag(TiffRootTag::Model, rawimage.clean_model.as_str())?;
  let uq_model = String::from(format!("{} {}", rawimage.clean_make, rawimage.clean_model));
  root_ifd.add_tag(DngTag::UniqueCameraModel, uq_model.as_str())?;
  root_ifd.add_tag(ExifTag::ModifyDate, chrono::Local::now().format("%Y:%m:%d %H:%M:%S").to_string())?;


  // Add matrix and illumninant
  root_ifd.add_tag(DngTag::CalibrationIlluminant1, matrix1_ill)?;
  root_ifd.add_tag(DngTag::ColorMatrix1, matrix1)?;

  // Add White balance info
  root_ifd.add_tag(DngTag::AsShotNeutral, &wb_coeff[..])?;

  // If compression thread handle is available, embed original file
  if let Some(handle) = orig_compress_handle {
    let (raw_digest, raw_data_compreessed) = handle.join().unwrap();
    root_ifd.add_tag_undefined(DngTag::OriginalRawFileData, Rc::new(raw_data_compreessed))?;
    root_ifd.add_tag(DngTag::OriginalRawFileName, orig_filename)?;
    root_ifd.add_tag(DngTag::OriginalRawFileDigest, raw_digest)?;
  }

  // Add EXIF information
  let exif_offset = {
    let mut exif_ifd = root_ifd.new_directory();
    // Add EXIF version 0220
    exif_ifd.add_tag_undefined(ExifTag::ExifVersion, Rc::new(vec![48, 50, 50, 48]))?;
    decoder.populate_dng_exif(&mut exif_ifd).unwrap();
    exif_ifd.build()?
  };
  root_ifd.add_tag(TiffRootTag::ExifIFDPointer, exif_offset)?;

  // Add XPACKET (XMP) information
  if let Some(xpacket) = decoder.xpacket() {
    //exif_ifd.write_tag_u8_array(ExifTag::ApplicationNotes, &xpacket)?;
    root_ifd.add_tag(ExifTag::ApplicationNotes, &xpacket[..])?;
  }

  // Add raw image
  let raw_offset = {
    let mut raw_ifd = root_ifd.new_directory();
    dng_put_raw(&mut raw_ifd, &rawimage, params)?;
    raw_ifd.build()?
  };

  // Add preview image
  let preview_offset = {
    let mut prev_image_ifd = root_ifd.new_directory();
    dng_put_preview(&mut prev_image_ifd, &full_img)?;
    prev_image_ifd.build()?
  };

  // Add SubIFDs
  root_ifd.add_tag(TiffRootTag::SubIFDs, [raw_offset as u32, preview_offset as u32])?;

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
fn dng_put_raw(raw_ifd: &mut DirectoryWriter<'_, '_>, rawimage: &RawImage, params: &DngParams) -> Result<()> {
  let black_level = blacklevel_to_tiff_value(&rawimage.blacklevels);
  let white_level = rawimage.whitelevels[0];

  // Active area or uncropped
  let active_area = if params.no_crop {
    [0, 0, rawimage.height as u16, rawimage.width as u16]
  } else {
    dng_active_area(&rawimage)
  };

  raw_ifd.add_tag(TiffRootTag::NewSubFileType, 0 as u16)?; // Raw
  raw_ifd.add_tag(TiffRootTag::ImageWidth, rawimage.width as u32)?;
  raw_ifd.add_tag(TiffRootTag::ImageLength, rawimage.height as u32)?;
  raw_ifd.add_tag(DngTag::ActiveArea, active_area)?;
  raw_ifd.add_tag(DngTag::BlackLevel, black_level)?;
  raw_ifd.add_tag(DngTag::BlackLevelRepeatDim, [2_u16, 2_u16])?;
  raw_ifd.add_tag(DngTag::WhiteLevel, white_level as u16)?;
  raw_ifd.add_tag(TiffRootTag::PhotometricInt, PhotometricInterpretation::CFA)?;
  raw_ifd.add_tag(DngTag::CFALayout, 1 as u16)?;
  raw_ifd.add_tag(TiffRootTag::CFAPattern, [0u8, 1u8, 1u8, 2u8])?; // RGGB
  raw_ifd.add_tag(TiffRootTag::CFARepeatPatternDim, [2u16, 2u16])?;
  raw_ifd.add_tag(ExifTag::PlanarConfiguration, 1_u16)?;
  raw_ifd.add_tag(DngTag::DefaultScale, [Rational::new(1, 1), Rational::new(1, 1)])?;
  raw_ifd.add_tag(DngTag::BestQualityScale, Rational::new(1, 1))?;

  //raw_ifd.add_tag(TiffRootTag::RowsPerStrip, rawimage.height as u16)?;
  raw_ifd.add_tag(TiffRootTag::SamplesPerPixel, 1_u16)?;
  raw_ifd.add_tag(TiffRootTag::BitsPerSample, [16_u16])?;
  //raw_ifd.add_tag(DngTag::DefaultCropOrigin, &default_crop[..])?;
  //raw_ifd.add_tag(DngTag::DefaultCropSize, &default_size[..])?;

  match params.compression {
    DngCompression::Uncompressed => {
      raw_ifd.add_tag(TiffRootTag::Compression, CompressionMethod::None)?;
      dng_put_raw_uncompressed(raw_ifd, rawimage)?;
    }
    DngCompression::Lossless => {
      raw_ifd.add_tag(TiffRootTag::Compression, CompressionMethod::ModernJPEG)?;
      dng_put_raw_ljpeg(raw_ifd, rawimage)?;
    }
  }

  Ok(())
}

/// Compress RAW image with LJPEG-92
///
/// Data is split into multiple tiles, each tile is compressed seperately
fn dng_put_raw_ljpeg(raw_ifd: &mut DirectoryWriter<'_, '_>, rawimage: &RawImage) -> Result<()> {
  let lj92_data = match rawimage.data {
    RawImageData::Integer(ref data) => {
      let tiled_data = TiledData::new(data, rawimage.width, rawimage.height);
      let tiles_compr: Vec<Vec<u8>> = tiled_data
        .tiles
        .par_iter()
        .map(|tile| {
          let state = LjpegCompressor::new(tile, tiled_data.tile_width, tiled_data.tile_length, 16, 0).unwrap();
          state.encode().unwrap()
        })
        .collect();
      (tiles_compr, tiled_data.tile_width, tiled_data.tile_length)
    }
    RawImageData::Float(ref _data) => {
      panic!("invalid format");
    }
  };

  let mut tile_offsets: Vec<u32> = Vec::new();
  let mut tile_sizes: Vec<u32> = Vec::new();

  lj92_data.0.iter().for_each(|tile| {
    let offs = raw_ifd.write_data(tile).unwrap();
    tile_offsets.push(offs);
    tile_sizes.push((tile.len() * size_of::<u8>()) as u32);
  });

  //let offs = raw_ifd.write_data(&lj92_data)?;
  raw_ifd.add_tag(TiffRootTag::TileOffsets, &tile_offsets)?;
  raw_ifd.add_tag(TiffRootTag::TileByteCounts, &tile_sizes)?;
  raw_ifd.add_tag(TiffRootTag::TileWidth, lj92_data.1 as u16)?; // FIXME
  raw_ifd.add_tag(TiffRootTag::TileLength, lj92_data.2 as u16)?;

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

      for strip in data.chunks(rows_per_strip * rawimage.width) {
        let offset = raw_ifd.write_data_u16_be(strip)?;
        strip_offsets.push(offset);
        strip_sizes.push((strip.len() * size_of::<u16>()) as u32);
        strip_rows.push((strip.len() / rawimage.width) as u32);
      }

      raw_ifd.add_tag(TiffRootTag::StripOffsets, &strip_offsets)?;
      raw_ifd.add_tag(TiffRootTag::StripByteCounts, &strip_sizes)?;
      raw_ifd.add_tag(TiffRootTag::RowsPerStrip, &strip_rows)?;
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

  ifd.add_tag(TiffRootTag::NewSubFileType, 1 as u16)?;
  ifd.add_tag(TiffRootTag::ImageWidth, thumb_img.width() as u32)?;
  ifd.add_tag(TiffRootTag::ImageLength, thumb_img.height() as u32)?;
  ifd.add_tag(TiffRootTag::Compression, CompressionMethod::None)?;
  ifd.add_tag(TiffRootTag::BitsPerSample, 8_u16)?;
  ifd.add_tag(TiffRootTag::SampleFormat, [1_u16, 1, 1])?;
  ifd.add_tag(TiffRootTag::PhotometricInt, PhotometricInterpretation::RGB)?;
  ifd.add_tag(TiffRootTag::SamplesPerPixel, 3_u16)?;
  //ifd.add_tag(TiffRootTag::XResolution, Rational { n: 1, d: 1 })?;
  //ifd.add_tag(TiffRootTag::YResolution, Rational { n: 1, d: 1 })?;
  //ifd.add_tag(TiffRootTag::ResolutionUnit, ResolutionUnit::None.to_u16())?;

  let offset = ifd.write_data(&thumb_img)?;

  ifd.add_tag(TiffRootTag::StripOffsets, offset)?;
  ifd.add_tag(TiffRootTag::StripByteCounts, thumb_img.len() as u32)?;
  ifd.add_tag(TiffRootTag::RowsPerStrip, thumb_img.height() as u32)?;

  Ok(())
}

fn dng_put_preview(ifd: &mut DirectoryWriter<'_, '_>, img: &DynamicImage) -> Result<()> {
  let now = Instant::now();
  let preview_img = DynamicImage::ImageRgb8(img.resize(1024, 768, FilterType::Nearest).to_rgb8());
  debug!("preview downscale: {} s", now.elapsed().as_secs_f32());

  ifd.add_tag(TiffRootTag::NewSubFileType, 1 as u16)?;
  ifd.add_tag(TiffRootTag::ImageWidth, preview_img.width() as u32)?;
  ifd.add_tag(TiffRootTag::ImageLength, preview_img.height() as u32)?;
  ifd.add_tag(TiffRootTag::Compression, CompressionMethod::ModernJPEG)?;
  ifd.add_tag(TiffRootTag::BitsPerSample, 8_u16)?;
  ifd.add_tag(TiffRootTag::SampleFormat, [1_u16, 1, 1])?;
  ifd.add_tag(TiffRootTag::PhotometricInt, PhotometricInterpretation::YCbCr)?;
  ifd.add_tag(TiffRootTag::RowsPerStrip, preview_img.height() as u32)?;
  ifd.add_tag(TiffRootTag::SamplesPerPixel, 3_u16)?;
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

  ifd.add_value(TiffRootTag::StripOffsets, Value::Long(vec![offset]))?;
  ifd.add_tag(TiffRootTag::StripByteCounts, [data_len as u32])?;

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
  ]
}

fn matrix_to_tiff_value(xyz_to_cam: &[[i32; 3]; 4], d: i32) -> [SRational; 9] {
  [
    SRational::new(xyz_to_cam[0][0], d),
    SRational::new(xyz_to_cam[0][1], d),
    SRational::new(xyz_to_cam[0][2], d),
    SRational::new(xyz_to_cam[1][0], d),
    SRational::new(xyz_to_cam[1][1], d),
    SRational::new(xyz_to_cam[1][2], d),
    SRational::new(xyz_to_cam[2][0], d),
    SRational::new(xyz_to_cam[2][1], d),
    SRational::new(xyz_to_cam[2][2], d),
  ]
}
