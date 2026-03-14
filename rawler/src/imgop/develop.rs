use std::{
  io::{self, Seek, Write},
  path::Path,
};

use image::{DynamicImage, ImageBuffer};

use crate::{
  RawImage,
  decoders::{RawDecodeParams, RawMetadata},
  dng::CropMode,
  formats::tiff::{DirectoryWriter, TiffWriter},
  imgop::{
    chromatic_adaption::adapt_bradford,
    fuji_rotate::fuji_normalize_rotation,
    matrix::{IDENTITY_MATRIX_3, transform_1d},
    sensor::{Demosaic, SensorType, xtrans::bilinear::XTransBilinearDemosaic},
  },
  pixarray::{Color2D, PixF32},
  rawimage::RawPhotometricInterpretation,
  rawsource::RawSource,
  tags::{ExifTag, TiffCommonTag},
};

use super::{
  Dim2, Rect, convert_from_f32_scaled_u16,
  raw::{map_3ch_to_rgb, map_4ch_to_rgb},
  sensor::bayer::{bilinear::Bilinear4Channel, ppg::PPGDemosaic},
  xyz::Illuminant,
};

/// Parameters for raw image processing
#[derive(Clone, Debug)]
pub struct RawProcessingParams {
  pub crop: CropMode,
  pub thumbnail: bool,
  pub artist: Option<String>,
  pub software: String,
  pub index: usize,
  pub keep_mtime: bool,
}

impl Default for RawProcessingParams {
  fn default() -> Self {
    Self {
      crop: CropMode::Best,
      thumbnail: true,
      artist: None,
      software: "DNGLab".into(),
      index: 0,
      keep_mtime: false,
    }
  }
}

pub fn process_raw_image<W: Write + Seek + Send>(raw: &Path, image_file: &mut W, params: &RawProcessingParams) -> crate::Result<()> {
  let rawfile = RawSource::new(raw)?;
  let decoder = crate::get_decoder(&rawfile)?;
  let raw_params = RawDecodeParams { image_index: params.index };
  let rawimage = decoder.raw_image(&rawfile, &raw_params, false)?;
  let metadata = decoder.raw_metadata(&rawfile, &raw_params)?;
  let develop = RawDevelop::default();
  develop.develop(&rawimage, &metadata, image_file)?;
  Ok(())
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum ProcessingStep {
  Rescale,
  Demosaic,
  FujiRotate,
  CropActiveArea,
  WhiteBalance,
  Calibrate,
  CropDefault,
  SRgb,
}

pub struct RawDevelopBuilder {}

#[derive(Clone)]
pub enum Intermediate {
  Monochrome(PixF32),
  ThreeColor(Color2D<f32, 3>),
  FourColor(Color2D<f32, 4>),
}

impl Intermediate {
  pub fn dim(&self) -> Dim2 {
    match self {
      Intermediate::Monochrome(pixels) => pixels.dim(),
      Intermediate::ThreeColor(pixels) => pixels.dim(),
      Intermediate::FourColor(pixels) => pixels.dim(),
    }
  }

  pub fn rect(&self) -> Rect {
    match self {
      Intermediate::Monochrome(pixels) => pixels.rect(),
      Intermediate::ThreeColor(pixels) => pixels.rect(),
      Intermediate::FourColor(pixels) => pixels.rect(),
    }
  }

  pub fn to_dynamic_image(self) -> Option<DynamicImage> {
    Some(match self {
      Intermediate::Monochrome(pixels) => {
        let data = convert_from_f32_scaled_u16(&pixels.data, 0, u16::MAX);
        DynamicImage::ImageLuma16(ImageBuffer::from_raw(pixels.dim().w as u32, pixels.dim().h as u32, data)?)
      }
      Intermediate::ThreeColor(pixels) => {
        let data = convert_from_f32_scaled_u16(&pixels.flatten(), 0, u16::MAX);
        DynamicImage::ImageRgb16(ImageBuffer::from_raw(pixels.dim().w as u32, pixels.dim().h as u32, data)?)
      }
      Intermediate::FourColor(pixels) => {
        let data = convert_from_f32_scaled_u16(&pixels.flatten(), 0, u16::MAX);
        DynamicImage::ImageRgba16(ImageBuffer::from_raw(pixels.dim().w as u32, pixels.dim().h as u32, data)?)
      }
    })
  }
}

#[derive(Clone)]
pub struct RawDevelop {
  pub steps: Vec<ProcessingStep>,
}

impl Default for RawDevelop {
  fn default() -> Self {
    Self {
      steps: vec![
        ProcessingStep::Rescale,
        ProcessingStep::Demosaic,
        ProcessingStep::FujiRotate,
        ProcessingStep::CropActiveArea,
        ProcessingStep::WhiteBalance,
        ProcessingStep::Calibrate,
        ProcessingStep::CropDefault,
        ProcessingStep::SRgb,
      ],
    }
  }
}

impl RawDevelop {
  pub fn new_with(steps: &[ProcessingStep]) -> Self {
    Self { steps: Vec::from(steps) }
  }

  /*
  pub fn linearize(rawimage: &RawImage) -> crate::Result<RgbF32> {
    todo!()
  }

  pub fn develop_monochrome_image(&self, rawimage: &RawImage) -> crate::Result<PixF32> {
    todo!()
  }

  pub fn develop_rgb_image(&self, rawimage: &RawImage) -> crate::Result<RgbF32> {
    todo!()
  }
   */

  /// Develop raw image and write result into TIFF.
  /// If demosaic is disabled or camera raw is monochrome, the TIFF
  /// has only one color channel.
  pub fn develop_intermediate(&self, rawimage: &RawImage) -> crate::Result<Intermediate> {
    let mut rawimage = rawimage.clone();
    if self.steps.contains(&ProcessingStep::Rescale) {
      rawimage.apply_scaling()?;
    }

    let mut intermediate = match rawimage.cpp {
      1 => Intermediate::Monochrome(PixF32::new_with(rawimage.data.as_f32().into_owned(), rawimage.width, rawimage.height)),
      3 => Intermediate::ThreeColor(Color2D::<f32, 3>::new_with(
        rawimage.data.as_f32().chunks_exact(3).map(|x| [x[0], x[1], x[2]]).collect(),
        rawimage.width,
        rawimage.height,
      )),
      4 => Intermediate::FourColor(Color2D::<f32, 4>::new_with(
        rawimage.data.as_f32().chunks_exact(4).map(|x| [x[0], x[1], x[2], x[3]]).collect(),
        rawimage.width,
        rawimage.height,
      )),
      _ => todo!(),
    };

    if self.steps.contains(&ProcessingStep::Demosaic) {
      intermediate = match &rawimage.photometric {
        RawPhotometricInterpretation::Cfa(config) => {
          if let Intermediate::Monochrome(pixels) = intermediate {
            let roi = if self.steps.contains(&ProcessingStep::CropActiveArea) {
              if rawimage.active_area.is_some() && rawimage.fuji_rotation_width.is_some() {
                panic!("ActiveArea is not possible when rotation is not normalized");
              }
              rawimage.active_area.unwrap_or(pixels.rect())
            } else {
              pixels.rect()
            };
            if config.cfa.is_rgb() && config.sensor == SensorType::Bayer {
              let ppg = PPGDemosaic::new();
              let mut rgb = ppg.demosaic(&pixels, &config.cfa, &config.colors, roi);

              // Fuji Rotate
              if self.steps.contains(&ProcessingStep::FujiRotate)
                && let Some(fuji_rotation_width) = rawimage.fuji_rotation_width
              {
                let extra_rotate = rawimage.camera.find_hint("fuji_rotate_90cw");
                rgb = fuji_normalize_rotation(&rgb, fuji_rotation_width, extra_rotate);
                log::debug!("dimension after rotate {:?}", rgb.dim());
              }
              Intermediate::ThreeColor(rgb)
            } else if config.cfa.unique_colors() == 4 && config.sensor == SensorType::Bayer {
              let linear = Bilinear4Channel::new();
              Intermediate::FourColor(linear.demosaic(&pixels, &config.cfa, &config.colors, roi))
            } else if config.cfa.is_rgb() && config.sensor == SensorType::Xtrans {
              let xtrans_demosaic = XTransBilinearDemosaic::new();
              Intermediate::ThreeColor(xtrans_demosaic.demosaic(&pixels, &config.cfa, &config.colors, roi))
            } else {
              todo!()
            }
          } else {
            intermediate
          }
        }
        _ => intermediate,
      };
    }

    if self.steps.contains(&ProcessingStep::Calibrate) {
      let mut xyz2cam: [[f32; 3]; 4] = [[0.0; 3]; 4];
      let d65_matrix: Vec<f32>;
      let (illu, matrix) = rawimage
        .color_matrix_find_first([
          Illuminant::D65, // Best option
          Illuminant::A,
          Illuminant::B,
          Illuminant::C,
          Illuminant::D50,
          Illuminant::D55,
          Illuminant::D75,
          Illuminant::Daylight,
          Illuminant::Flash,
        ])
        .unwrap_or_else(|| {
          log::warn!("No alternative matrix found, using identity matrix");
          (Illuminant::D65, IDENTITY_MATRIX_3.as_flattened().to_vec())
        });
      if illu == Illuminant::D65 {
        d65_matrix = matrix;
      } else {
        log::warn!("Image has no D65 illuminant matrix, alternative for Bradford adaption: {:?}", illu);
        d65_matrix = match matrix.len() {
          9 => adapt_bradford(&illu, &Illuminant::D65, &transform_1d::<3, 3>(&matrix).expect("Transformation failed"))
            .as_flattened()
            .to_vec(),
          _ => unimplemented!(),
        }
      }

      assert_eq!(d65_matrix.len() % 3, 0); // this is not so nice...
      let components = d65_matrix.len() / 3;
      for i in 0..components {
        for j in 0..3 {
          xyz2cam[i][j] = d65_matrix[i * 3 + j];
        }
      }

      // Some old images may not provide WB coeffs. Assume 1.0 in this case.
      let mut wb = if rawimage.wb_coeffs[0].is_nan() {
        [1.0, 1.0, 1.0, 1.0]
      } else {
        rawimage.wb_coeffs
      };
      if !self.steps.contains(&ProcessingStep::WhiteBalance) {
        wb = [1.0, 1.0, 1.0, 1.0];
      }

      log::debug!("wb: {:?}, coeff: {:?}", wb, xyz2cam);

      intermediate = match intermediate {
        Intermediate::Monochrome(_) => intermediate,
        Intermediate::ThreeColor(pixels) => Intermediate::ThreeColor(map_3ch_to_rgb(&pixels, &wb, xyz2cam)),
        Intermediate::FourColor(pixels) => Intermediate::ThreeColor(map_4ch_to_rgb(&pixels, &wb, xyz2cam)),
      };
    }

    if self.steps.contains(&ProcessingStep::CropDefault) {
      log::debug!("ProcessingStep: CropDefault");
      log::debug!("Crop: {:?}", rawimage.crop_area);
      log::debug!("active_area: {:?}", rawimage.active_area);
      if let Some(mut crop) = rawimage.crop_area.or(rawimage.active_area) {
        if self.steps.contains(&ProcessingStep::Demosaic) && self.steps.contains(&ProcessingStep::CropActiveArea) {
          // If active area crop was already applied during demosaic, we need to
          // adapt default crop to active area crop.
          if let Some(active_area) = &rawimage.active_area {
            crop = crop.adapt(active_area);
            log::debug!("Adapt crop to active_area: {:?}", crop);
          }
        }
        if intermediate.dim().w == rawimage.active_area.map(|area| area.d).unwrap_or(rawimage.dim()).w / 2 {
          // Superpixel debayer used
          crop.scale(0.5);
          log::debug!("Scale crop to 0.5: {:?}", crop);
        }
        // Only apply crop if dimensions differ.
        if crop.d != intermediate.dim() {
          log::debug!("crop: {:?}, intermediate dim: {:?}, rawimage: {:?}", crop, intermediate.dim(), rawimage.dim());
          intermediate = match intermediate {
            Intermediate::Monochrome(pixels) => Intermediate::Monochrome(pixels.crop(crop)),
            Intermediate::ThreeColor(pixels) => Intermediate::ThreeColor(pixels.crop(crop)),
            Intermediate::FourColor(pixels) => Intermediate::FourColor(pixels.crop(crop)),
          };
        }
      }
    }

    if self.steps.contains(&ProcessingStep::SRgb) {
      match &mut intermediate {
        Intermediate::Monochrome(pixels) => pixels.for_each(super::srgb::srgb_apply_gamma),
        Intermediate::ThreeColor(pixels) => pixels.for_each(super::srgb::srgb_apply_gamma_n),
        Intermediate::FourColor(pixels) => pixels.for_each(super::srgb::srgb_apply_gamma_n),
      };
    }

    Ok(intermediate)
  }

  /// Develop raw image and write result into TIFF.
  /// If demosaic is disabled or camera raw is monochrome, the TIFF
  /// has only one color channel.
  pub fn develop<W>(&self, rawimage: &RawImage, md: &RawMetadata, writer: W) -> crate::Result<()>
  where
    W: io::Write + io::Seek,
  {
    let intermediate = self.develop_intermediate(rawimage)?;

    let mut tiff = TiffWriter::new(writer)?;
    let mut root_ifd = DirectoryWriter::new();
    let mut exif_ifd = DirectoryWriter::new();

    // Add EXIF version 0220
    exif_ifd.add_tag_undefined(ExifTag::ExifVersion, vec![48, 50, 50, 48]);

    md.write_exif_tags(&mut tiff, &mut root_ifd, &mut exif_ifd)?;
    root_ifd.add_tag(TiffCommonTag::Make, rawimage.clean_make.as_str());
    root_ifd.add_tag(TiffCommonTag::Model, rawimage.clean_model.as_str());

    let exif_offset = exif_ifd.build(&mut tiff)?;

    root_ifd.add_tag(TiffCommonTag::ExifIFDPointer, exif_offset);
    root_ifd.add_tag(TiffCommonTag::Orientation, rawimage.orientation.to_u16());

    match intermediate {
      Intermediate::Monochrome(pixels) => {
        let data = convert_from_f32_scaled_u16(&pixels.data, 0, u16::MAX);
        let (strip_rows, strips) = tiff.write_strips_lzw(&data, 1, pixels.dim(), 0)?;
        let strip_offsets: Vec<u32> = strips.iter().map(|(offset, _)| *offset).collect();
        let strip_bytes: Vec<u32> = strips.iter().map(|(_, bytes)| *bytes).collect();
        root_ifd.add_tag(TiffCommonTag::Compression, 5);
        root_ifd.add_tag(TiffCommonTag::Predictor, 1);
        root_ifd.add_tag(TiffCommonTag::StripOffsets, &strip_offsets);
        root_ifd.add_tag(TiffCommonTag::StripByteCounts, &strip_bytes);
        root_ifd.add_tag(TiffCommonTag::BitsPerSample, [16_u16]);
        root_ifd.add_tag(TiffCommonTag::SamplesPerPixel, [1_u16]);
        root_ifd.add_tag(TiffCommonTag::PhotometricInt, [1_u16]);
        root_ifd.add_tag(TiffCommonTag::RowsPerStrip, strip_rows);
        root_ifd.add_tag(TiffCommonTag::ImageWidth, pixels.width as u16);
        root_ifd.add_tag(TiffCommonTag::ImageLength, pixels.height as u16);
      }
      Intermediate::ThreeColor(pixels) => {
        let data = convert_from_f32_scaled_u16(&pixels.flatten(), 0, u16::MAX);
        let (strip_rows, strips) = tiff.write_strips_lzw(&data, 3, pixels.dim(), 0)?;
        let strip_offsets: Vec<u32> = strips.iter().map(|(offset, _)| *offset).collect();
        let strip_bytes: Vec<u32> = strips.iter().map(|(_, bytes)| *bytes).collect();
        root_ifd.add_tag(TiffCommonTag::Compression, 5);
        root_ifd.add_tag(TiffCommonTag::Predictor, 1);
        root_ifd.add_tag(TiffCommonTag::StripOffsets, &strip_offsets);
        root_ifd.add_tag(TiffCommonTag::StripByteCounts, &strip_bytes);
        root_ifd.add_tag(TiffCommonTag::BitsPerSample, [16_u16, 16, 16]);
        root_ifd.add_tag(TiffCommonTag::SamplesPerPixel, [3_u16]);
        root_ifd.add_tag(TiffCommonTag::PhotometricInt, [2_u16]);
        root_ifd.add_tag(TiffCommonTag::RowsPerStrip, strip_rows);
        root_ifd.add_tag(TiffCommonTag::ImageWidth, pixels.width as u16);
        root_ifd.add_tag(TiffCommonTag::ImageLength, pixels.height as u16);
      }
      Intermediate::FourColor(pixels) => {
        let data = convert_from_f32_scaled_u16(&pixels.flatten(), 0, u16::MAX);
        let (strip_rows, strips) = tiff.write_strips_lzw(&data, 4, pixels.dim(), 0)?;
        let strip_offsets: Vec<u32> = strips.iter().map(|(offset, _)| *offset).collect();
        let strip_bytes: Vec<u32> = strips.iter().map(|(_, bytes)| *bytes).collect();
        root_ifd.add_tag(TiffCommonTag::Compression, 5);
        root_ifd.add_tag(TiffCommonTag::Predictor, 1);
        root_ifd.add_tag(TiffCommonTag::StripOffsets, &strip_offsets);
        root_ifd.add_tag(TiffCommonTag::StripByteCounts, &strip_bytes);
        root_ifd.add_tag(TiffCommonTag::BitsPerSample, [16_u16, 16, 16, 16]); // Extra-channel, even if PhotometricInt is RGB!
        root_ifd.add_tag(TiffCommonTag::SamplesPerPixel, [4_u16]);
        root_ifd.add_tag(TiffCommonTag::PhotometricInt, [2_u16]);
        root_ifd.add_tag(TiffCommonTag::RowsPerStrip, strip_rows);
        root_ifd.add_tag(TiffCommonTag::ImageWidth, pixels.width as u16);
        root_ifd.add_tag(TiffCommonTag::ImageLength, pixels.height as u16);
      }
    }

    tiff.build(root_ifd)?;

    Ok(())
  }
}
