use std::{
  io::{self, Seek, Write},
  path::Path,
};

use image::{DynamicImage, ImageBuffer};

use crate::{
  RawImage, RawImageData,
  decoders::{RawDecodeParams, RawMetadata, validate_image_index},
  dng::{
    CropMode,
    opcodes::{OpcodeSelector, OpcodeStage},
  },
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
  Dim2, Point, Rect, convert_from_f32_scaled_u16, crop,
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
  /// DNG opcodes to remove before processing
  pub remove_dng_opcodes: Vec<OpcodeSelector>,
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
      remove_dng_opcodes: Vec::new(),
    }
  }
}

pub fn process_raw_image<W: Write + Seek + Send>(raw: &Path, image_file: &mut W, params: &RawProcessingParams) -> crate::Result<()> {
  let rawfile = RawSource::new(raw)?;
  let decoder = crate::get_decoder(&rawfile)?;
  let raw_params = RawDecodeParams { image_index: params.index };
  validate_image_index(decoder.as_ref(), &raw_params)?;
  let mut rawimage = decoder.raw_image(&rawfile, &raw_params, false)?;
  rawimage.remove_opcodes(&params.remove_dng_opcodes);
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
    // OpcodeList1 is applied to the raw image as read from the file
    rawimage.apply_opcode_list(OpcodeStage::List1)?;
    if self.steps.contains(&ProcessingStep::Rescale) {
      rawimage.apply_scaling()?;
      // OpcodeList2 is applied just after mapping to linear reference values
      rawimage.apply_opcode_list(OpcodeStage::List2)?;
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

      // OpcodeList3 is applied just after demosaicing
      if rawimage.opcode_list(OpcodeStage::List3).is_some() {
        let cropped = matches!(rawimage.photometric, RawPhotometricInterpretation::Cfa(_)) && self.steps.contains(&ProcessingStep::CropActiveArea);
        intermediate = apply_opcode_list3(&mut rawimage, intermediate, cropped)?;
      }
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

/// Apply OpcodeList3 to the demosaiced image. While the opcodes are applied, the
/// intermediate is the image data of `rawimage`. If `cropped` is set, the
/// intermediate is the ActiveArea of the raw image and stays cropped to it.
fn apply_opcode_list3(rawimage: &mut RawImage, intermediate: Intermediate, cropped: bool) -> crate::Result<Intermediate> {
  let dim = intermediate.dim();
  let (cpp, data) = match intermediate {
    Intermediate::Monochrome(pixels) => (1, pixels.data),
    Intermediate::ThreeColor(pixels) => (3, pixels.data.into_flattened()),
    Intermediate::FourColor(pixels) => (4, pixels.data.into_flattened()),
  };
  rawimage.width = dim.w;
  rawimage.height = dim.h;
  rawimage.cpp = cpp;
  rawimage.data = RawImageData::Float(data);
  if cropped && let Some(active_area) = rawimage.active_area.take() {
    rawimage.crop_area = rawimage.crop_area.and_then(|crop_area| crop_area.rebase(&active_area));
  }

  rawimage.apply_opcode_list(OpcodeStage::List3)?;

  let mut samples = match std::mem::replace(&mut rawimage.data, RawImageData::Float(Vec::new())) {
    RawImageData::Float(samples) => samples,
    data => data.as_f32().into_owned(),
  };
  // A TrimBounds in OpcodeList3 shrinks the ActiveArea, keep the intermediate cropped to it
  if cropped && let Some(active_area) = rawimage.active_area.take() {
    let window = Rect::new(
      Point::new(active_area.p.x * cpp, active_area.p.y),
      Dim2::new(active_area.d.w * cpp, active_area.d.h),
    );
    samples = crop(&samples, Dim2::new(rawimage.width * cpp, rawimage.height), window);
    rawimage.width = active_area.d.w;
    rawimage.height = active_area.d.h;
    rawimage.crop_area = rawimage.crop_area.and_then(|crop_area| crop_area.rebase(&active_area));
  }
  let (width, height) = (rawimage.width, rawimage.height);
  Ok(match cpp {
    1 => Intermediate::Monochrome(PixF32::new_with(samples, width, height)),
    3 => Intermediate::ThreeColor(Color2D::new_with(samples.as_chunks::<3>().0.to_vec(), width, height)),
    _ => Intermediate::FourColor(Color2D::new_with(samples.as_chunks::<4>().0.to_vec(), width, height)),
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{
    CFA,
    decoders::Camera,
    dng::opcodes::{FixBadPixelsConstant, Opcode, OpcodeList, TrimBounds, WarpRectilinear, WarpRectilinearCoeffs},
    pixarray::PixU16,
    rawimage::{BlackLevel, CFAConfig, WhiteLevel},
  };

  fn warp_rectilinear(k_r0: f64, k_r1: f64) -> Opcode {
    Opcode::WarpRectilinear(WarpRectilinear {
      version: 0x01030000,
      flags: 0,
      coeffs: vec![WarpRectilinearCoeffs {
        k_r0,
        k_r1,
        k_r2: 0.0,
        k_r3: 0.0,
        k_t0: 0.0,
        k_t1: 0.0,
      }],
      c_x: 0.5,
      c_y: 0.5,
    })
  }

  #[test]
  fn develop_applies_opcode_list3() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // LinearRaw ramp, the warp maps all pixels to full or half source pixels
    let ramp = (0..8 * 6 * 3).map(|i| ((i / 3) % 8) as f32 / 8.0).collect();
    let mut rawimage = RawImage::new_with_data(
      Camera::default(),
      RawImageData::Float(ramp),
      8 * 3,
      6,
      3,
      [1.0, 1.0, 1.0, f32::NAN],
      RawPhotometricInterpretation::LinearRaw,
      Some(BlackLevel::zero(1, 1, 3)),
      Some(WhiteLevel::new(vec![1; 3])),
      false,
    );
    rawimage.dng_opcodes_3 = Some(OpcodeList::new(vec![warp_rectilinear(0.5, 0.0)]));
    let develop = RawDevelop::new_with(&[ProcessingStep::Rescale, ProcessingStep::Demosaic]);
    let Intermediate::ThreeColor(pixels) = develop.develop_intermediate(&rawimage)? else {
      panic!("expected three color image");
    };
    for (i, pixel) in pixels.data.iter().enumerate() {
      let x = (i % 8) as f32;
      assert_eq!(*pixel, [(2.0 + x / 2.0) / 8.0; 3], "pixel {}", i);
    }
    Ok(())
  }

  #[test]
  fn develop_opcode_list3_keeps_crop_consistent() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut camera = Camera::new();
    camera.cfa = CFA::new("RGGB");
    let photometric = RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&camera));
    let mut rawimage = RawImage::new(
      camera,
      PixU16::new_with(vec![2048; 16 * 16], 16, 16),
      1,
      [1.0, 1.0, 1.0, f32::NAN],
      photometric,
      Some(BlackLevel::new(&[0_u32, 0, 0, 0], 2, 2, 1)),
      Some(WhiteLevel::new_bits(12, 1)),
      false,
    );
    rawimage.active_area = Some(Rect::new(Point::new(2, 2), Dim2::new(12, 10)));
    rawimage.crop_area = Some(Rect::new(Point::new(4, 4), Dim2::new(8, 6)));
    // TrimBounds moves the stage 3 image origin by (1, 1), DefaultCrop keeps its size
    let trim = Opcode::TrimBounds(TrimBounds {
      version: 0x01030000,
      flags: 0,
      top: 1,
      left: 1,
      bottom: 9,
      right: 11,
    });
    rawimage.dng_opcodes_3 = Some(OpcodeList::new(vec![warp_rectilinear(0.98, 0.03), trim]));
    let develop = RawDevelop::new_with(&[
      ProcessingStep::Rescale,
      ProcessingStep::Demosaic,
      ProcessingStep::CropActiveArea,
      ProcessingStep::CropDefault,
    ]);
    let intermediate = develop.develop_intermediate(&rawimage)?;
    assert_eq!(intermediate.dim(), Dim2::new(8, 6));
    let Intermediate::ThreeColor(pixels) = intermediate else {
      panic!("expected three color image");
    };
    let expected = 2048.0 / 4095.0;
    assert!(pixels.data.iter().flatten().all(|v| (v - expected).abs() < 1e-4));
    Ok(())
  }

  #[test]
  fn develop_applies_opcode_list1() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut camera = Camera::new();
    camera.cfa = CFA::new("RGGB");
    let photometric = RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&camera));
    // Bad red pixel, its red neighbours are 1000, 1200, 1400 and 1600
    let mut pixels = vec![2048; 16 * 16];
    pixels[6 * 16 + 6] = 0;
    for (i, (row, col)) in [(4, 6), (8, 6), (6, 4), (6, 8)].into_iter().enumerate() {
      pixels[row * 16 + col] = 1000 + 200 * i as u16;
    }
    let mut rawimage = RawImage::new(
      camera,
      PixU16::new_with(pixels, 16, 16),
      1,
      [1.0, 1.0, 1.0, f32::NAN],
      photometric,
      Some(BlackLevel::new(&[0_u32, 0, 0, 0], 2, 2, 1)),
      Some(WhiteLevel::new_bits(12, 1)),
      false,
    );
    rawimage.dng_opcodes_1 = Some(OpcodeList::new(vec![Opcode::FixBadPixelsConstant(FixBadPixelsConstant {
      version: 0x01030000,
      flags: 0,
      constant: 0,
      bayer_phase: 0,
    })]));
    let Intermediate::Monochrome(pixels) = RawDevelop::new_with(&[ProcessingStep::Rescale]).develop_intermediate(&rawimage)? else {
      panic!("expected monochrome image");
    };
    assert_eq!(pixels.data[6 * 16 + 6], 1300.0 / 4095.0);
    Ok(())
  }
}
