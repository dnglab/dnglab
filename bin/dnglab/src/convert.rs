// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use anyhow::Context;
use clap::{App, ArgMatches};
use core::panic;
use image::{imageops::FilterType, DynamicImage, GenericImageView};
use log::debug;
use rawler::tiff::{Rational, SRational};
use rawler::{
  dngencoder::TiledData,
  tiff::{CompressionMethod, DirectoryWriter, PhotometricInterpretation, PreviewColorSpace, Value},
};
use rawler::{
  dngencoder::{compression::compress_original, DNG_VERSION_V1_4},
  ljpeg92::LjpegCompressor,
  tags::{DngTag, ExifTag, TiffRootTag},
  tiff::TiffWriter,
  Buffer, RawImage, RawImageData,
};
use rayon::prelude::*;
use std::fs;
use std::{
  fs::{read_dir, File},
  io::{BufReader, BufWriter},
  ops::Deref,
  path::{Path, PathBuf},
  rc::Rc,
  sync::Arc,
  thread,
  time::Instant,
  u16,
};

use crate::AppError;

const SUPPORTED_FILE_EXT: [&'static str; 3] = ["CR3", "CR2", "CRW"];

const PREVIEW_JPEG_QUALITY: f32 = 0.75;

fn digest_original_buf(data: &[u8]) -> Result<md5::Digest, std::io::Error> {
  Ok(md5::compute(&data))
}

fn build_output_path(in_path: &Path, out_path: &Path) -> anyhow::Result<PathBuf> {
  if out_path.exists() {
    let out_md = fs::metadata(out_path).context("Unable to determine metadata for given output")?;
    if out_md.is_file() {
      return Ok(PathBuf::from(out_path));
    } else if out_md.is_dir() {
      let new_filename = in_path.with_extension("DNG").file_name().unwrap().to_str().unwrap().to_string();
      let mut tmp = PathBuf::from(out_path);
      tmp.push(new_filename);
      return Ok(tmp);
    } else {
      return Err(AppError::InvalidArgs.into());
    }
  } else {
    match out_path.parent() {
      Some(parent) => {
        let out_md = fs::metadata(parent).context("Unable to determine metadata for given output")?;
        if out_md.is_dir() {
          // Ok, parent exists an is directory
          return Ok(PathBuf::from(out_path));
        } else {
          println!("Output or parent directory not exists");
          return Err(AppError::InvalidArgs.into());
        }
      }
      None => {
        println!("Output or parent directory not exists");
        return Err(AppError::InvalidArgs.into());
      }
    }
  }
}

pub fn convert(options: &ArgMatches<'_>) -> anyhow::Result<()> {
  let in_path = PathBuf::from(options.value_of("INPUT").expect("INPUT not available"));
  let out_path = PathBuf::from(options.value_of("OUTPUT").expect("OUTPUT not available"));
  if !in_path.exists() {
    println!("INPUT path '{}' not exists", in_path.display());
    return Err(AppError::InvalidArgs.into());
  }
  let in_md = fs::metadata(&in_path).context("Unable to determine metadata for given input")?;
  if in_md.is_file() {
    // Convert a single file
    return convert_file(&in_path, &build_output_path(&in_path, &out_path)?, options);
  } else if in_md.is_dir() {
    // Convert whole directory
    return convert_dir(&in_path, &out_path, options);
  } else {
    println!("INPUT is not a file nor directory");
    return Err(AppError::InvalidArgs.into());
  }
}

fn is_ext_supported<T: AsRef<str>>(ext: T) -> bool {
  let uc = ext.as_ref().to_uppercase();
  SUPPORTED_FILE_EXT.iter().any(|ext| ext.eq(&uc))
}

fn convert_dir(in_path: &Path, out_path: &Path, options: &ArgMatches<'_>) -> anyhow::Result<()> {
  let in_files: Vec<PathBuf> = read_dir(in_path)?
    .map(|entry| entry.unwrap().path())
    .filter(|entry| fs::metadata(entry).map(|md| md.is_file()).unwrap_or(true))
    .filter(|file| match file.extension() {
      Some(ext) => is_ext_supported(ext.to_string_lossy()),
      None => false,
    })
    .collect();

  let results: Vec<bool> = in_files
    .iter()
    .map(|in_path| {
      if let Ok(out_path) = build_output_path(in_path, out_path) {
        convert_file(&in_path, &out_path, options).is_ok()
      } else {
        false
      }
    })
    .collect();

  let success_count = results.iter().filter(|r| **r).count();
  let failed_count = results.iter().filter(|r| !**r).count();

  if failed_count == 0 {
    println!("Finished {}/{}", success_count, results.len());
  } else {
    println!("Finished {}/{}, {} failed", success_count, results.len(), failed_count);
  }

  Ok(())
}

fn convert_file(in_file: &Path, out_file: &Path, options: &ArgMatches<'_>) -> anyhow::Result<()> {
  debug!("Infile: {:?}, Outfile: {:?}", in_file, out_file);

  if out_file.exists() && !options.is_present("override") {
    println!("File {} already exists and --override was not given", out_file.to_str().unwrap_or_default());
    return Err(AppError::InvalidArgs.into());
  }

  let start_event = Instant::now();

  let file_name = String::from(in_file.file_name().unwrap().to_os_string().to_str().unwrap());

  let mut in_f = BufReader::new(File::open(in_file)?);
  let out_f = File::create(out_file)?;

  let mut out_f = BufWriter::new(out_f);

  let in_buffer = Arc::new(Buffer::new(&mut in_f).unwrap());

  let buffer_for_compress = in_buffer.clone();

  let compress_handle = thread::spawn(move || {
    let raw_digest = digest_original_buf(buffer_for_compress.raw_buf()).unwrap();

    let raw_data_compreessed = compress_original(buffer_for_compress.raw_buf()).unwrap();
    (raw_digest, raw_data_compreessed)
  });

  let mut decoder = rawler::get_decoder(&in_buffer).unwrap();

  let now = Instant::now();
  decoder.decode_metadata().unwrap();
  debug!("Meta decoding: {} s", now.elapsed().as_secs_f32());

  let full_img = decoder.full_image().unwrap();

  let now = Instant::now();
  let rawimage = decoder.raw_image(false).unwrap();
  debug!("crx decoding: {} s", now.elapsed().as_secs_f32());

  let mut dng = TiffWriter::new(&mut out_f).unwrap();

  let mut root_ifd = dng.new_directory();

  dng_put_thumbnail(&mut root_ifd, &full_img).unwrap();



  let black_level: [SRational; 4] = [
    SRational {
      n: rawimage.blacklevels[0] as i32,
      d: 1,
    },
    SRational {
      n: rawimage.blacklevels[1] as i32,
      d: 1,
    },
    SRational {
      n: rawimage.blacklevels[2] as i32,
      d: 1,
    },
    SRational {
      n: rawimage.blacklevels[3] as i32,
      d: 1,
    },
  ];

  let white_level = rawimage.whitelevels[0];

  let wb_coeff: [Rational; 3] = [
    Rational {
      n: ((1.0 / (rawimage.wb_coeffs[0] / 1024.0)) * 1000000.0) as u32,
      d: 1000000,
    },
    Rational {
      n: ((1.0 / (rawimage.wb_coeffs[1] / 1024.0)) * 1000000.0) as u32,
      d: 1000000,
    },
    Rational {
      n: ((1.0 / (rawimage.wb_coeffs[2] / 1024.0)) * 1000000.0) as u32,
      d: 1000000,
    },
  ];

  let active_area = dng_compatible_active_area(&rawimage);

  debug!("DNG active: {:?}", active_area);
  //let default_crop = [16_u16, 16_u16];
  //let default_size = [8192_u16, 5464_u16];

  //let active_area: [u16; 4] = [80, 122, 3950, 5918]; //] 80 122 3950 5918

  debug!("crop: {} {} {} {}", rawimage.crops[0], rawimage.crops[1], rawimage.crops[2], rawimage.crops[3]);

  debug!(
    "coeff: {} {} {} {}",
    rawimage.wb_coeffs[0], rawimage.wb_coeffs[1], rawimage.wb_coeffs[2], rawimage.wb_coeffs[3]
  );

  // You can encode tags here
  root_ifd.add_tag(TiffRootTag::Software, "DNGLab v0.1").unwrap();
  root_ifd.add_tag(DngTag::DNGVersion, &DNG_VERSION_V1_4[..]).unwrap();
  root_ifd.add_tag(DngTag::DNGBackwardVersion, &DNG_VERSION_V1_4[..]).unwrap();
  root_ifd.add_tag(TiffRootTag::Make, rawimage.make.as_str()).unwrap();
  root_ifd.add_tag(TiffRootTag::Model, rawimage.clean_model.as_str()).unwrap();
  let uq_model = String::from(format!("{} {}", rawimage.make, rawimage.model));
  root_ifd.add_tag(DngTag::UniqueCameraModel, uq_model.as_str()).unwrap();

  //root_ifd.add_tag(TiffRootTag::CalibrationIlluminant1, 17u16).unwrap(); // Color A
  root_ifd.add_tag(DngTag::CalibrationIlluminant1, rawimage.illuminant2).unwrap(); // D65

  let matrix1 = [
    SRational::new(rawimage.xyz_to_cam2[0][0], rawimage.illuminant2_denominator),
    SRational::new(rawimage.xyz_to_cam2[0][1], rawimage.illuminant2_denominator),
    SRational::new(rawimage.xyz_to_cam2[0][2], rawimage.illuminant2_denominator),
    SRational::new(rawimage.xyz_to_cam2[1][0], rawimage.illuminant2_denominator),
    SRational::new(rawimage.xyz_to_cam2[1][1], rawimage.illuminant2_denominator),
    SRational::new(rawimage.xyz_to_cam2[1][2], rawimage.illuminant2_denominator),
    SRational::new(rawimage.xyz_to_cam2[2][0], rawimage.illuminant2_denominator),
    SRational::new(rawimage.xyz_to_cam2[2][1], rawimage.illuminant2_denominator),
    SRational::new(rawimage.xyz_to_cam2[2][2], rawimage.illuminant2_denominator),
  ];

  //root_ifd.add_tag(TiffRootTag::CameraCalibration1, &cameraCalibration2[..]).unwrap();
  //root_ifd.add_tag(TiffRootTag::CameraCalibration2, &cameraCalibration2[..]).unwrap();
  root_ifd.add_tag(DngTag::ColorMatrix1, matrix1).unwrap();
  //root_ifd.add_tag(TiffRootTag::ColorMatrix2, &matrix2[..]).unwrap();

  root_ifd.add_tag(DngTag::AsShotNeutral, &wb_coeff[..]).unwrap();

  //root_ifd.add_tag(TiffRootTag::BaselineExposure, SRational{ n: 1, d: 100}).unwrap();

  let (raw_digest, raw_data_compreessed) = compress_handle.join().unwrap();

  root_ifd.add_tag_undefined(DngTag::OriginalRawFileData, Rc::new(raw_data_compreessed)).unwrap();

  root_ifd.add_tag(DngTag::OriginalRawFileName, file_name).unwrap();
  root_ifd.add_tag(DngTag::OriginalRawFileDigest, *raw_digest.deref()).unwrap();

  let exif_offset = {
    let mut exif_ifd = root_ifd.new_directory();
    // Add at least one tag
    exif_ifd.add_tag(ExifTag::Orientation, 1_u16).unwrap();
    decoder.populate_dng_exif(&mut exif_ifd).unwrap();
    exif_ifd.build().unwrap()
  };
  root_ifd.add_tag(TiffRootTag::ExifIFDPointer, exif_offset).unwrap();

  decoder.populate_dng_root(&mut root_ifd).unwrap();

  if let Some(xpacket) = decoder.xpacket() {
    //exif_ifd.write_tag_u8_array(ExifTag::ApplicationNotes, &xpacket)?;
    root_ifd.add_tag(ExifTag::ApplicationNotes, &xpacket[..]).unwrap();
  }

  let now = Instant::now();

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
  debug!("LJpeg ecoding: {} s", now.elapsed().as_secs_f32());

  //let mut jfile = File::create("/tmp/ljpeg.jpg").unwrap();
  //jfile.write_all(&lj92_data).unwrap();

  let raw_offset = {
    let mut raw_ifd = root_ifd.new_directory();
    //let mut raw_image = image
    //  .new_plain_image::<colortype::Gray16>(rawimage.width as u32, rawimage.height as u32)
    //  .unwrap();

    //let mut raw_ifd = image.new_directory().unwrap();

    raw_ifd.add_tag(TiffRootTag::ImageWidth, rawimage.width as u32).unwrap();
    raw_ifd.add_tag(TiffRootTag::ImageLength, rawimage.height as u32).unwrap();

    raw_ifd.add_tag(TiffRootTag::NewSubFileType, 0 as u16).unwrap(); // Raw
    raw_ifd.add_tag(DngTag::ActiveArea, &active_area[..]).unwrap();
    //raw_ifd.add_tag(DngTag::DefaultCropOrigin, &default_crop[..]).unwrap();
    //raw_ifd.add_tag(DngTag::DefaultCropSize, &default_size[..]).unwrap();
    raw_ifd.add_tag(DngTag::BlackLevel, &black_level[..]).unwrap();
    raw_ifd.add_tag(DngTag::BlackLevelRepeatDim, &[2u16, 2u16][..]).unwrap();
    raw_ifd.add_tag(DngTag::WhiteLevel, white_level as u16).unwrap();
    raw_ifd.add_tag(TiffRootTag::PhotometricInt, PhotometricInterpretation::CFA).unwrap();
    raw_ifd.add_tag(DngTag::CFALayout, 1 as u16).unwrap();
    raw_ifd.add_tag(TiffRootTag::CFAPattern, &[0u8, 1u8, 1u8, 2u8][..]).unwrap(); // RGGB
    raw_ifd.add_tag(TiffRootTag::CFARepeatPatternDim, &[2u16, 2u16][..]).unwrap();
    raw_ifd.add_tag(ExifTag::PlanarConfiguration, 1_u16).unwrap();
    raw_ifd
      .add_tag(DngTag::DefaultScale, &[Rational { n: 1, d: 1 }, Rational { n: 1, d: 1 }][..])
      .unwrap();
    raw_ifd.add_tag(DngTag::BestQualityScale, Rational { n: 1, d: 1 }).unwrap();
    raw_ifd.add_tag(TiffRootTag::Compression, CompressionMethod::ModernJPEG).unwrap();
    raw_ifd.add_tag(TiffRootTag::RowsPerStrip, rawimage.height as u16).unwrap();
    raw_ifd.add_tag(TiffRootTag::SamplesPerPixel, 1_u16).unwrap();

    raw_ifd.add_tag(TiffRootTag::BitsPerSample, [16_u16]).unwrap();
    //raw_ifd.add_tag(TiffRootTag::SampleFormat, [SampleFormat::Uint as u16]).unwrap();

    let mut tile_offsets: Vec<u32> = Vec::new();
    let mut tile_sizes: Vec<u32> = Vec::new();

    lj92_data.0.iter().for_each(|tile| {
      let offs = raw_ifd.write_data(tile).unwrap();
      tile_offsets.push(offs);
      tile_sizes.push(tile.len() as u32);
    });

    //let offs = raw_ifd.write_data(&lj92_data).unwrap();
    raw_ifd.add_tag(TiffRootTag::TileOffsets, &tile_offsets[..]).unwrap();
    raw_ifd.add_tag(TiffRootTag::TileByteCounts, &tile_sizes[..]).unwrap();
    raw_ifd.add_tag(TiffRootTag::TileWidth, lj92_data.1 as u16).unwrap(); // FIXME
    raw_ifd.add_tag(TiffRootTag::TileLength, lj92_data.2 as u16).unwrap();

    /*
    // Strip size can be configured before writing data
    raw_image.rows_per_strip(2).unwrap();

    let mut idx = 0;
    while raw_image.next_strip_sample_count() > 0 {
      let sample_count = raw_image.next_strip_sample_count() as usize;

      match rawimage.data {
        RawImageData::Integer(ref data) => {
          raw_image.write_strip(&data[idx..idx + sample_count]).unwrap();
        }
        RawImageData::Float(ref _data) => {
          panic!("invalid format");
        }
      }

      idx += sample_count;
    }
     */

    raw_ifd.build().unwrap()
  };

  let now = Instant::now();
  let preview_offset = {
    let mut prev_image = root_ifd.new_directory();
    dng_put_preview(&mut prev_image, &full_img).unwrap();
    prev_image.build().unwrap()
  };
  debug!("add preview: {} s", now.elapsed().as_secs_f32());

  root_ifd.add_tag(TiffRootTag::SubIFDs, [raw_offset as u32, preview_offset as u32]).unwrap();

  /*
  let thumbnail: [u8; 256*171*3] = [0x99; 256*171*3];

   // Strip size can be configured before writing data
   image.rows_per_strip(2).unwrap();

   let mut idx = 0;
   while image.next_strip_sample_count() > 0 {
       let sample_count = image.next_strip_sample_count() as usize;
       image.write_strip(&thumbnail[idx..idx+sample_count]).unwrap();
       idx += sample_count;
   }



  */

  //let mut thumb_buf = Vec::new();

  //mini_preview.read_to_end(&mut thumb_buf);

  // Strip size can be configured before writing data
  /* TODO
  root_ifd.rows_per_strip(171).unwrap();

  let mut idx = 0;
  while image.next_strip_sample_count() > 0 {
    let sample_count = image.next_strip_sample_count() as usize;
    image.write_strip(&thumb_buf[idx..idx + sample_count]).unwrap();
    idx += sample_count;
  }
   */

  let ifd0_offset = root_ifd.build().unwrap();

  let now = Instant::now();
  dng.build(ifd0_offset).unwrap();
  debug!("build: {} s", now.elapsed().as_secs_f32());

  //dng.update_ifd0_offset(ifd0_offset).unwrap();

  println!(
    "Converted '{}' => '{}' (in {:.2}s)",
    shorten_path(in_file),
    shorten_path(out_file),
    start_event.elapsed().as_secs_f32()
  );

  Ok(())
}

// DNG ActiveArea  is:
//  Top, Left, Bottom, Right
// RawImage.crop is:
// Top, Right, Bottom, Left
fn dng_compatible_active_area(image: &RawImage) -> [u16; 4] {
  [
    image.crops[0] as u16, // top
    image.crops[3] as u16, // left
    //(image.height-image.crops[0]-image.crops[2]) as u16, // bottom
    //(image.width-image.crops[1]-image.crops[3]) as u16, // Right
    (image.height - (image.crops[2])) as u16, // bottom coord
    (image.width - (image.crops[1])) as u16,  // Right coord
  ]
}

//impl<'a, 'w, W: Write + Seek + 'w> DirectoryWriter<'a, 'w, W> {

fn dng_put_thumbnail(ifd: &mut DirectoryWriter<'_, '_>, img: &DynamicImage) -> Result<(), ()> {
  let thumb_img = img.resize(240, 120, FilterType::Nearest).to_rgb8();

  ifd.add_tag(TiffRootTag::NewSubFileType, 1 as u16).unwrap();
  ifd.add_tag(TiffRootTag::ImageWidth, thumb_img.width() as u32).unwrap();
  ifd.add_tag(TiffRootTag::ImageLength, thumb_img.height() as u32).unwrap();
  ifd.add_tag(TiffRootTag::Compression, CompressionMethod::None).unwrap();
  ifd.add_tag(TiffRootTag::BitsPerSample, 8_u16).unwrap();
  //let sample_format: Vec<_> = <T>::SAMPLE_FORMAT.iter().map(|s| s.to_u16()).collect();
  ifd.add_tag(TiffRootTag::SampleFormat, [1_u16, 1, 1]).unwrap();
  ifd.add_tag(TiffRootTag::PhotometricInt, PhotometricInterpretation::RGB).unwrap();
  ifd.add_tag(TiffRootTag::RowsPerStrip, thumb_img.height() as u32).unwrap();
  ifd.add_tag(TiffRootTag::SamplesPerPixel, 3_u16).unwrap();
  //ifd.add_tag(TiffRootTag::XResolution, Rational { n: 1, d: 1 })?;
  //ifd.add_tag(TiffRootTag::YResolution, Rational { n: 1, d: 1 })?;
  //ifd.add_tag(TiffRootTag::ResolutionUnit, ResolutionUnit::None.to_u16())?;

  let offset = ifd.write_data(&thumb_img).unwrap();

  ifd.add_value(TiffRootTag::StripOffsets, Value::Long(vec![offset])).unwrap();
  ifd.add_tag(TiffRootTag::StripByteCounts, [thumb_img.len() as u32]).unwrap();

  Ok(())
}

fn dng_put_preview(ifd: &mut DirectoryWriter<'_, '_>, img: &DynamicImage) -> Result<(), ()> {
  let now = Instant::now();
  let preview_img = DynamicImage::ImageRgb8(img.resize(1024, 768, FilterType::Nearest).to_rgb8());
  debug!("preview downscale: {} s", now.elapsed().as_secs_f32());

  ifd.add_tag(TiffRootTag::NewSubFileType, 1 as u16).unwrap();
  ifd.add_tag(TiffRootTag::ImageWidth, preview_img.width() as u32).unwrap();
  ifd.add_tag(TiffRootTag::ImageLength, preview_img.height() as u32).unwrap();
  ifd.add_tag(TiffRootTag::Compression, CompressionMethod::ModernJPEG).unwrap();
  ifd.add_tag(TiffRootTag::BitsPerSample, 8_u16).unwrap();
  ifd.add_tag(TiffRootTag::SampleFormat, [1_u16, 1, 1]).unwrap();
  ifd.add_tag(TiffRootTag::PhotometricInt, PhotometricInterpretation::YCbCr).unwrap();
  ifd.add_tag(TiffRootTag::RowsPerStrip, preview_img.height() as u32).unwrap();
  ifd.add_tag(TiffRootTag::SamplesPerPixel, 3_u16).unwrap();
  ifd.add_tag(DngTag::PreviewColorSpace, PreviewColorSpace::SRgb).unwrap(); // ??

  //ifd.add_tag(TiffRootTag::XResolution, Rational { n: 1, d: 1 })?;
  //ifd.add_tag(TiffRootTag::YResolution, Rational { n: 1, d: 1 })?;
  //ifd.add_tag(TiffRootTag::ResolutionUnit, ResolutionUnit::None.to_u16())?;

  let now = Instant::now();
  let offset = ifd.tiff.position().unwrap();

  preview_img
    .write_to(
      &mut ifd.tiff.writer,
      image::ImageOutputFormat::Jpeg((PREVIEW_JPEG_QUALITY * u8::MAX as f32) as u8),
    )
    .unwrap();
  let data_len = ifd.tiff.position().unwrap() - offset;
  debug!("writing preview: {} s", now.elapsed().as_secs_f32());

  ifd.add_value(TiffRootTag::StripOffsets, Value::Long(vec![offset])).unwrap();
  ifd.add_tag(TiffRootTag::StripByteCounts, [data_len as u32]).unwrap();

  Ok(())
}

fn shorten_path(path: &Path) -> String {
  let os_str = path.as_os_str();
  if os_str.len() <= 30 {
    String::from(os_str.to_string_lossy())
  } else {
    let full = String::from(os_str.to_string_lossy());
    //let a = &full[..full.len()-8];
    //let b = &full[full.len()-8..];
    //format!("{}...{}", a, b)
    full
  }
}
