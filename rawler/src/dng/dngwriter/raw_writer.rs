use crate::dng::dngwriter::{dng_put_raw_ljpeg, dng_put_raw_uncompressed, CropMode, DngCompression};
use crate::formats::tiff::Rational;
use crate::tags::{DngTag, ExifTag, TiffCommonTag};
use crate::{
  dng::rect_to_dng_area,
  formats::tiff::{CompressionMethod, DirectoryWriter, PhotometricInterpretation},
  imgop::{Dim2, Point, Rect},
  RawImage,
};

use crate::Result;

use super::ConvertParams;

/// Write RAW image data into DNG
///
/// Encode raw image data as new raw IFD with NewSubFileType 0
pub(super) fn write_rawimage(raw_ifd: &mut DirectoryWriter<'_, '_>, rawimage: &RawImage, params: &ConvertParams) -> Result<()> {
  let full_size = Rect::new(Point::new(0, 0), Dim2::new(rawimage.width, rawimage.height));

  // Active area or uncropped
  let active_area: Rect = match params.crop {
    CropMode::ActiveArea | CropMode::Best => rawimage.active_area.unwrap_or(full_size),
    CropMode::None => full_size,
  };

  assert!(active_area.p.x + active_area.d.w <= rawimage.width);
  assert!(active_area.p.y + active_area.d.h <= rawimage.height);

  raw_ifd.add_tag(TiffCommonTag::NewSubFileType, 0_u16)?; // Raw
  raw_ifd.add_tag(TiffCommonTag::ImageWidth, rawimage.width as u32)?;
  raw_ifd.add_tag(TiffCommonTag::ImageLength, rawimage.height as u32)?;

  raw_ifd.add_tag(DngTag::ActiveArea, rect_to_dng_area(&active_area))?;

  match params.crop {
    CropMode::ActiveArea => {
      let crop = active_area;
      assert!(crop.p.x >= active_area.p.x);
      assert!(crop.p.y >= active_area.p.y);
      raw_ifd.add_tag(
        DngTag::DefaultCropOrigin,
        [(crop.p.x - active_area.p.x) as u16, (crop.p.y - active_area.p.y) as u16],
      )?;
      raw_ifd.add_tag(DngTag::DefaultCropSize, [crop.d.w as u16, crop.d.h as u16])?;
    }
    CropMode::Best => {
      let crop = rawimage.crop_area.unwrap_or(active_area);
      assert!(crop.p.x >= active_area.p.x);
      assert!(crop.p.y >= active_area.p.y);
      raw_ifd.add_tag(
        DngTag::DefaultCropOrigin,
        [(crop.p.x - active_area.p.x) as u16, (crop.p.y - active_area.p.y) as u16],
      )?;
      raw_ifd.add_tag(DngTag::DefaultCropSize, [crop.d.w as u16, crop.d.h as u16])?;
    }
    CropMode::None => {}
  }

  raw_ifd.add_tag(ExifTag::PlanarConfiguration, 1_u16)?;

  raw_ifd.add_tag(
    DngTag::DefaultScale,
    [
      Rational::new(rawimage.camera.default_scale[0][0], rawimage.camera.default_scale[0][1]),
      Rational::new(rawimage.camera.default_scale[1][0], rawimage.camera.default_scale[1][1]),
    ],
  )?;
  raw_ifd.add_tag(
    DngTag::BestQualityScale,
    Rational::new(rawimage.camera.best_quality_scale[0], rawimage.camera.best_quality_scale[1]),
  )?;

  // Whitelevel
  assert_eq!(rawimage.whitelevel.len(), rawimage.cpp, "Whitelevel sample count must match cpp");
  raw_ifd.add_tag(DngTag::WhiteLevel, &rawimage.whitelevel)?;

  // Blacklevel
  let blacklevel = rawimage.blacklevel.shift(active_area.p.x, active_area.p.y);

  raw_ifd.add_tag(DngTag::BlackLevelRepeatDim, [blacklevel.height as u16, blacklevel.width as u16])?;

  assert!(blacklevel.sample_count() == rawimage.cpp || blacklevel.sample_count() == rawimage.cfa.width * rawimage.cfa.height * rawimage.cpp);
  if blacklevel.levels.iter().all(|x| x.d == 1) {
    let payload: Vec<u16> = blacklevel.levels.iter().map(|x| x.n as u16).collect();
    raw_ifd.add_tag(DngTag::BlackLevel, &payload)?;
  } else {
    raw_ifd.add_tag(DngTag::BlackLevel, blacklevel.levels.as_slice())?;
  }

  match rawimage.cpp {
    1 => {
      if !rawimage.blackareas.is_empty() {
        let data: Vec<u16> = rawimage.blackareas.iter().flat_map(rect_to_dng_area).collect();
        raw_ifd.add_tag(DngTag::MaskedAreas, &data)?;
      }
      raw_ifd.add_tag(TiffCommonTag::PhotometricInt, PhotometricInterpretation::CFA)?;
      raw_ifd.add_tag(TiffCommonTag::SamplesPerPixel, 1_u16)?;
      raw_ifd.add_tag(TiffCommonTag::BitsPerSample, [16_u16])?;

      let cfa = rawimage.cfa.shift(active_area.p.x, active_area.p.y);

      raw_ifd.add_tag(TiffCommonTag::CFARepeatPatternDim, [cfa.width as u16, cfa.height as u16])?;
      raw_ifd.add_tag(TiffCommonTag::CFAPattern, &cfa.flat_pattern()[..])?;

      //raw_ifd.add_tag(DngTag::CFAPlaneColor, [0u8, 1u8, 2u8])?; // RGB

      //raw_ifd.add_tag(DngTag::CFAPlaneColor, [1u8, 4u8, 3u8, 5u8])?; // RGB

      raw_ifd.add_tag(DngTag::CFALayout, 1_u16)?; // Square layout

      //raw_ifd.add_tag(LegacyTiffRootTag::CFAPattern, [0u8, 1u8, 1u8, 2u8])?; // RGGB
      //raw_ifd.add_tag(LegacyTiffRootTag::CFARepeatPatternDim, [2u16, 2u16])?;
      //raw_ifd.add_tag(DngTag::CFAPlaneColor, [0u8, 1u8, 2u8])?; // RGGB
    }
    3 => {
      raw_ifd.add_tag(TiffCommonTag::PhotometricInt, PhotometricInterpretation::LinearRaw)?;
      raw_ifd.add_tag(TiffCommonTag::SamplesPerPixel, 3_u16)?;
      raw_ifd.add_tag(TiffCommonTag::BitsPerSample, [16_u16, 16_u16, 16_u16])?;

      //raw_ifd.add_tag(DngTag::CFAPlaneColor, [1u8, 2u8, 0u8])?; //
    }
    cpp => {
      panic!("Unsupported cpp: {}", cpp);
    }
  }

  match params.compression {
    DngCompression::Uncompressed => {
      raw_ifd.add_tag(TiffCommonTag::Compression, CompressionMethod::None)?;
      dng_put_raw_uncompressed(raw_ifd, rawimage)?;
    }
    DngCompression::Lossless => {
      raw_ifd.add_tag(TiffCommonTag::Compression, CompressionMethod::ModernJPEG)?;
      dng_put_raw_ljpeg(raw_ifd, rawimage, params.predictor)?;
    }
  }

  Ok(())
}
