use std::{
  ffi::OsStr,
  io::{self, Cursor, Seek, Write},
  path::Path,
};

use image::{DynamicImage, ImageBuffer, ImageEncoder, codecs::jpeg::JpegEncoder};

use crate::{
  RawImage, RawImageData,
  camera_profile::D65_PREFERENCE,
  decoders::{RawDecodeParams, RawMetadata, validate_image_index},
  dng::{
    CropMode,
    opcodes::{OpcodeSelector, OpcodeStage},
    profile_render::{ProfileRender, prophoto_to_srgb, srgb_to_prophoto},
  },
  formats::tiff::{DirectoryWriter, TiffWriter},
  imgop::{
    chromatic_adaption::{adapt_xyz2cam_bradford, bradford_adaption_matrix},
    fuji_rotate::fuji_normalize_rotation,
    matrix::{diagonal, identity, invert, multiply, multiply_vector, normalize, pseudo_inverse, transform_1d},
    sensor::{Demosaic, SensorType, xtrans::bilinear::XTransBilinearDemosaic},
  },
  pixarray::{Color2D, PixF32, RgbF32},
  rawimage::RawPhotometricInterpretation,
  rawsource::RawSource,
  tags::{ExifTag, TiffCommonTag},
};

use super::{
  Dim2, Point, Rect, convert_from_f32_scaled_u16, crop,
  raw::{map_3ch_to_rgb, map_4ch_to_rgb, map_to_3ch},
  sensor::bayer::{bilinear::Bilinear4Channel, ppg::PPGDemosaic},
  xyz::{Illuminant, SRGB_TO_XYZ_D65, XYZ_TO_SRGB_D65},
};

/// File format of a processed raw image
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProcessedImageFormat {
  /// 16-bit TIFF, LZW compressed
  #[default]
  Tiff,
  /// 8-bit JPEG
  Jpeg,
}

impl ProcessedImageFormat {
  /// Default file extension of the format
  pub fn extension(&self) -> &'static str {
    match self {
      Self::Tiff => "tif",
      Self::Jpeg => "jpg",
    }
  }

  /// Format of a file extension, ignoring case
  pub fn from_extension(extension: &OsStr) -> Option<Self> {
    match extension.to_str()?.to_ascii_lowercase().as_str() {
      "tif" | "tiff" => Some(Self::Tiff),
      "jpg" | "jpeg" => Some(Self::Jpeg),
      _ => None,
    }
  }
}

#[cfg(feature = "clap")]
impl clap::ValueEnum for ProcessedImageFormat {
  fn value_variants<'a>() -> &'a [Self] {
    &[Self::Tiff, Self::Jpeg]
  }

  fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
    Some(match self {
      Self::Tiff => clap::builder::PossibleValue::new("tif").alias("tiff"),
      Self::Jpeg => clap::builder::PossibleValue::new("jpeg").alias("jpg"),
    })
  }
}

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
  pub format: ProcessedImageFormat,
  /// JPEG quality (1-100)
  pub jpeg_quality: u8,
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
      format: ProcessedImageFormat::Tiff,
      jpeg_quality: 94,
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
  match params.format {
    ProcessedImageFormat::Tiff => develop.develop(&rawimage, &metadata, image_file)?,
    ProcessedImageFormat::Jpeg => develop.develop_jpeg(&rawimage, &metadata, image_file, params.jpeg_quality)?,
  }
  Ok(())
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum ProcessingStep {
  Rescale,
  Demosaic,
  FujiRotate,
  CropActiveArea,
  BaselineExposure,
  WhiteBalance,
  Calibrate,
  /// Apply the rendering tables of the camera profile (part of Calibrate)
  ProfileRender,
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
        ProcessingStep::BaselineExposure,
        ProcessingStep::WhiteBalance,
        ProcessingStep::Calibrate,
        ProcessingStep::ProfileRender,
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

    // BaselineExposure is applied as linear gain to the scene-linear data, clipping
    // is done by the color space conversion or the final conversion to integers.
    let exposure = if self.steps.contains(&ProcessingStep::BaselineExposure) {
      rawimage.baseline_exposure.unwrap_or(0.0) + rawimage.baseline_exposure_offset().unwrap_or(0.0)
    } else {
      0.0
    };
    if self.steps.contains(&ProcessingStep::BaselineExposure) {
      if exposure != 0.0 {
        let gain = exposure.exp2();
        log::debug!("ProcessingStep: BaselineExposure {} EV, gain: {}", exposure, gain);
        match &mut intermediate {
          Intermediate::Monochrome(pixels) => pixels.for_each(|v| v * gain),
          Intermediate::ThreeColor(pixels) => pixels.for_each(|v| v.map(|v| v * gain)),
          Intermediate::FourColor(pixels) => pixels.for_each(|v| v.map(|v| v * gain)),
        };
      }
    }

    if self.steps.contains(&ProcessingStep::Calibrate) {
      // Some old images may not provide WB coeffs. Assume 1.0 in this case.
      let mut wb = if rawimage.wb_coeffs[0].is_nan() {
        [1.0, 1.0, 1.0, 1.0]
      } else {
        rawimage.wb_coeffs
      };
      if !self.steps.contains(&ProcessingStep::WhiteBalance) {
        wb = [1.0, 1.0, 1.0, 1.0];
      }

      // Rendering tables of the camera profile, for the same slot as the matrices
      let render = if self.steps.contains(&ProcessingStep::ProfileRender) {
        rawimage.best_calibration_slot().and_then(|slot| ProfileRender::new(slot.profile, slot.slot))
      } else {
        None
      };

      intermediate = match intermediate {
        Intermediate::Monochrome(_) => intermediate,
        Intermediate::ThreeColor(pixels) => {
          let wb = [wb[0], wb[1], wb[2]];
          let cam2rgb = camera_to_srgb(&rawimage, &wb);
          log::debug!("wb: {:?}, camera to sRGB: {:?}", wb, cam2rgb);
          Intermediate::ThreeColor(match &render {
            Some(render) => map_with_profile_render(&pixels, &wb, exposure, &cam2rgb, render),
            None => map_3ch_to_rgb(&pixels, &wb, &cam2rgb),
          })
        }
        Intermediate::FourColor(pixels) => {
          let cam2rgb = camera_to_srgb(&rawimage, &wb);
          log::debug!("wb: {:?}, camera to sRGB: {:?}", wb, cam2rgb);
          Intermediate::ThreeColor(match &render {
            Some(render) => map_with_profile_render(&pixels, &wb, exposure, &cam2rgb, render),
            None => map_4ch_to_rgb(&pixels, &wb, &cam2rgb),
          })
        }
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
    let mut root_ifd = Self::metadata_ifd(&mut tiff, rawimage, md)?;

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

  /// Develop raw image and write result as 8-bit JPEG with EXIF data
  pub fn develop_jpeg<W>(&self, rawimage: &RawImage, md: &RawMetadata, writer: W, quality: u8) -> crate::Result<()>
  where
    W: io::Write,
  {
    let image = self
      .develop_intermediate(rawimage)?
      .to_dynamic_image()
      .ok_or("failed to convert to dynamic image")?;

    // EXIF data is a TIFF structure without image data
    let mut exif = Cursor::new(Vec::new());
    let mut tiff = TiffWriter::new(&mut exif)?;
    let root_ifd = Self::metadata_ifd(&mut tiff, rawimage, md)?;
    tiff.build(root_ifd)?;
    let exif = exif.into_inner();

    let mut encoder = JpegEncoder::new_with_quality(writer, quality);
    // The APP1 segment holds 65533 bytes, including the "Exif\0\0" header
    if exif.len() <= 65527 {
      encoder
        .set_exif_metadata(exif)
        .map_err(|err| format!("Failed to set JPEG EXIF data: {}", err))?;
    } else {
      log::warn!("EXIF data of {} bytes doesn't fit into JPEG, it is not written", exif.len());
    }
    image.write_with_encoder(encoder).map_err(|err| format!("Failed to write JPEG: {}", err))?;
    Ok(())
  }

  /// IFD 0 with the EXIF data of a developed image
  fn metadata_ifd<W>(tiff: &mut TiffWriter<W>, rawimage: &RawImage, md: &RawMetadata) -> crate::Result<DirectoryWriter>
  where
    W: io::Write + io::Seek,
  {
    let mut root_ifd = DirectoryWriter::new();
    let mut exif_ifd = DirectoryWriter::new();

    // Add EXIF version 0220
    exif_ifd.add_tag_undefined(ExifTag::ExifVersion, vec![48, 50, 50, 48]);

    md.write_exif_tags(tiff, &mut root_ifd, &mut exif_ifd)?;
    root_ifd.add_tag(TiffCommonTag::Make, rawimage.clean_make.as_str());
    root_ifd.add_tag(TiffCommonTag::Model, rawimage.clean_model.as_str());

    let exif_offset = exif_ifd.build(tiff)?;

    root_ifd.add_tag(TiffCommonTag::ExifIFDPointer, exif_offset);
    root_ifd.add_tag(TiffCommonTag::Orientation, rawimage.orientation.to_u16());
    Ok(root_ifd)
  }
}

/// Map camera values to linear sRGB (D65) by way of the rendering tables of
/// the camera profile, which are applied to linear ProPhoto RGB
///
/// Like a DNG reader, white balanced camera values are clipped at the camera
/// white (scaled by the BaselineExposure gain), so clipped highlights stay
/// neutral, and colors outside of sRGB are clipped per channel.
fn map_with_profile_render<const N: usize>(pixels: &Color2D<f32, N>, wb: &[f32; N], exposure: f32, cam2rgb: &[[f32; N]; 3], render: &ProfileRender) -> RgbF32 {
  let white = exposure.exp2() * wb.iter().copied().fold(f32::INFINITY, f32::min);
  let mut prophoto = map_to_3ch(pixels, wb, white, &multiply(&srgb_to_prophoto(), cam2rgb));
  prophoto.for_each(|pixel| render.apply(pixel));
  let mut srgb = map_to_3ch(&prophoto, &[1.0; 3], f32::INFINITY, &prophoto_to_srgb());
  srgb.for_each(|pixel| pixel.map(|v| v.clamp(0.0, 1.0)));
  srgb
}

/// Matrix from white balanced camera values to linear sRGB (D65)
///
/// Camera values are white balanced by `wb`, the reciprocal of the camera
/// neutral. AnalogBalance and CameraCalibration (AB * CC) of the image are
/// applied. With a ForwardMatrix, the camera values are mapped to XYZ D50 as
/// defined by DNG and adapted to D65 by Bradford. Otherwise the ColorMatrix,
/// adapted to D65, is normalized to map the camera neutral to the sRGB white.
fn camera_to_srgb<const N: usize>(rawimage: &RawImage, wb: &[f32; N]) -> [[f32; N]; 3] {
  let slot = rawimage.best_calibration_slot();

  let analog_balance = rawimage.analog_balance.as_deref().and_then(|balance| {
    let balance = <[f32; N]>::try_from(balance).ok();
    if balance.is_none() {
      log::warn!("AnalogBalance doesn't match {} color planes, ignoring it", N);
    }
    balance
  });
  let calibration = slot.and_then(|slot| slot.camera_calibration).and_then(|matrix| {
    let matrix = transform_1d::<N, N>(matrix);
    if matrix.is_none() {
      log::warn!("CameraCalibration doesn't match {} color planes, ignoring it", N);
    }
    matrix
  });
  let abcc = match (analog_balance, calibration) {
    (None, None) => None,
    (balance, calibration) => Some(multiply(&diagonal(&balance.unwrap_or([1.0; N])), &calibration.unwrap_or(identity()))),
  };

  if let Some(forward_matrix) = slot.and_then(|slot| slot.forward_matrix) {
    match (transform_1d::<3, N>(forward_matrix), invert(&abcc.unwrap_or(identity()))) {
      (Some(forward_matrix), Some(abcc_inv)) => {
        // CameraToXYZ_D50 = FM * D * Inverse(AB * CC), D white balances the reference neutral
        let neutral = wb.map(|coeff| 1.0 / coeff);
        let reference_neutral = multiply_vector(&abcc_inv, &neutral);
        let camera_to_xyz = multiply(&forward_matrix, &multiply(&diagonal(&reference_neutral.map(|v| 1.0 / v)), &abcc_inv));
        // The camera values are already white balanced by `wb`
        let camera_to_xyz = multiply(&camera_to_xyz, &diagonal(&neutral));
        let xyz_to_srgb = multiply(&XYZ_TO_SRGB_D65, &bradford_adaption_matrix(&Illuminant::D50, &Illuminant::D65));
        let cam2rgb = multiply(&xyz_to_srgb, &camera_to_xyz);
        if cam2rgb.as_flattened().iter().all(|v| v.is_finite()) {
          return cam2rgb;
        }
        log::warn!("Invalid ForwardMatrix result for white balance {:?}, using ColorMatrix", wb);
      }
      (None, _) => log::warn!("ForwardMatrix doesn't match {} color planes, using ColorMatrix", N),
      (_, None) => log::warn!("AnalogBalance * CameraCalibration is not invertible, using ColorMatrix"),
    }
  }

  let xyz2cam = match slot.and_then(|slot| Some((slot.illuminant, transform_1d::<N, 3>(slot.color_matrix)?))) {
    Some((Illuminant::D65, matrix)) => matrix,
    Some((illuminant, matrix)) if !D65_PREFERENCE.contains(&illuminant) => {
      log::warn!("Color matrix for {:?} can't be adapted to D65, it is used as it is", illuminant);
      matrix
    }
    Some((illuminant, matrix)) => {
      log::warn!("Image has no D65 illuminant matrix, alternative for Bradford adaption: {:?}", illuminant);
      adapt_xyz2cam_bradford(&illuminant, &Illuminant::D65, &matrix)
    }
    None => {
      log::warn!("No color matrix for {} color planes found, using identity matrix", N);
      std::array::from_fn(|i| std::array::from_fn(|j| if i == j { 1.0 } else { 0.0 }))
    }
  };
  let xyz2cam = match abcc {
    Some(abcc) => multiply(&abcc, &xyz2cam),
    None => xyz2cam,
  };
  pseudo_inverse(normalize(multiply(&xyz2cam, &SRGB_TO_XYZ_D65)))
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
    imgop::matrix::{FlatMatrix, transform_2d},
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

  /// ForwardMatrix of the Adobe Standard profile for the Phase One IQ3 100MP
  const FORWARD_MATRIX: [[f32; 3]; 3] = [[0.8296, 0.0538, 0.0809], [0.3137, 0.8399, -0.1536], [0.0133, -0.4061, 1.2179]];
  const COLOR_MATRIX: [[f32; 3]; 3] = [[0.6722, -0.0635, -0.0963], [-0.4287, 1.2460, 0.2028], [-0.0908, 0.2162, 0.5668]];
  const CALIBRATION: [[f32; 3]; 3] = [[1.0, 0.1, 0.0], [0.0, 0.9, 0.05], [0.0, 0.0, 1.1]];

  /// Image with a D65 profile, `forward_matrix` and the calibration data
  fn calibrated_rawimage(forward_matrix: Option<FlatMatrix>, calibration: Option<FlatMatrix>, analog_balance: Option<Vec<f32>>) -> RawImage {
    use crate::camera_profile::{CameraCalibration, CameraProfile};

    let mut rawimage = RawImage::new_with_data(
      Camera::default(),
      RawImageData::Float(vec![0.0; 3]),
      3,
      1,
      3,
      [f32::NAN; 4],
      RawPhotometricInterpretation::LinearRaw,
      Some(BlackLevel::zero(1, 1, 3)),
      Some(WhiteLevel::new(vec![1; 3])),
      false,
    );
    let mut profile = CameraProfile::default();
    profile.calibration_illuminant[0] = Some(Illuminant::D65);
    profile.color_matrix[0] = Some(transform_2d(&COLOR_MATRIX));
    profile.forward_matrix[0] = forward_matrix;
    rawimage.camera_profiles = vec![profile];
    rawimage.camera_calibration = CameraCalibration {
      matrices: [calibration, None, None],
      signature: None,
    };
    rawimage.analog_balance = analog_balance;
    rawimage
  }

  fn assert_matrix_eq<const N: usize>(a: &[[f32; N]; 3], b: &[[f32; N]; 3]) {
    for (x, y) in a.as_flattened().iter().zip(b.as_flattened()) {
      assert!((x - y).abs() < 1e-4, "{:?} != {:?}", a, b);
    }
  }

  /// The white balanced camera neutral is the sRGB white
  fn assert_neutral_is_white<const N: usize>(cam2rgb: &[[f32; N]; 3]) {
    for v in multiply_vector(cam2rgb, &[1.0; N]) {
      assert!((v - 1.0).abs() < 1e-3, "{:?}", cam2rgb);
    }
  }

  #[test]
  fn forward_matrix_maps_to_xyz_d50() {
    let rawimage = calibrated_rawimage(Some(transform_2d(&FORWARD_MATRIX)), None, None);
    let cam2rgb = camera_to_srgb(&rawimage, &[2.0, 1.0, 1.5]);
    let expected = multiply(
      &multiply(&XYZ_TO_SRGB_D65, &bradford_adaption_matrix(&Illuminant::D50, &Illuminant::D65)),
      &FORWARD_MATRIX,
    );
    assert_matrix_eq(&cam2rgb, &expected);
    assert_neutral_is_white(&cam2rgb);
  }

  #[test]
  fn forward_matrix_with_calibration() {
    let wb = [2.0, 1.0, 1.5];
    let plain = camera_to_srgb(&calibrated_rawimage(Some(transform_2d(&FORWARD_MATRIX)), None, None), &wb);
    // A diagonal AB * CC is removed by the white balance
    let diagonal_calibration = calibrated_rawimage(
      Some(transform_2d(&FORWARD_MATRIX)),
      Some(transform_2d(&diagonal(&[1.1, 1.0, 0.9]))),
      Some(vec![1.2, 1.0, 0.8]),
    );
    assert_matrix_eq(&camera_to_srgb(&diagonal_calibration, &wb), &plain);
    // A full CameraCalibration changes the colors, the neutral stays white
    let calibrated = camera_to_srgb(
      &calibrated_rawimage(Some(transform_2d(&FORWARD_MATRIX)), Some(transform_2d(&CALIBRATION)), Some(vec![1.2, 1.0, 0.8])),
      &wb,
    );
    assert_neutral_is_white(&calibrated);
    assert!(calibrated.as_flattened().iter().zip(plain.as_flattened()).any(|(a, b)| (a - b).abs() > 0.01));
  }

  #[test]
  fn color_matrix_with_calibration() {
    let wb = [2.0, 1.0, 1.5];
    // Without calibration, the matrix is calculated as before
    let plain = camera_to_srgb(&calibrated_rawimage(None, None, None), &wb);
    assert_eq!(plain, pseudo_inverse(normalize(multiply(&COLOR_MATRIX, &SRGB_TO_XYZ_D65))));
    let diagonal_calibration = calibrated_rawimage(None, Some(transform_2d(&diagonal(&[1.1, 1.0, 0.9]))), Some(vec![1.2, 1.0, 0.8]));
    assert_matrix_eq(&camera_to_srgb(&diagonal_calibration, &wb), &plain);
    // XYZtoCamera = AB * CC * CM
    let calibrated = camera_to_srgb(&calibrated_rawimage(None, Some(transform_2d(&CALIBRATION)), Some(vec![1.2, 1.0, 0.8])), &wb);
    let xyz2cam = multiply(&diagonal(&[1.2, 1.0, 0.8]), &multiply(&CALIBRATION, &COLOR_MATRIX));
    assert_matrix_eq(&calibrated, &pseudo_inverse(normalize(multiply(&xyz2cam, &SRGB_TO_XYZ_D65))));
    assert_neutral_is_white(&calibrated);
  }

  #[test]
  fn forward_matrix_with_four_color_planes() {
    let forward_matrix = [[0.4, 0.2, 0.3, 0.0644], [0.2, 0.5, 0.1, 0.2], [0.1, 0.0, 0.6, 0.1252]];
    let mut rawimage = calibrated_rawimage(Some(transform_2d(&forward_matrix)), None, Some(vec![1.0, 1.0, 1.0, 2.0]));
    rawimage.camera_profiles[0].color_matrix[0] = Some(vec![1.0; 12]);
    let cam2rgb = camera_to_srgb(&rawimage, &[2.0, 1.0, 1.5, 1.2]);
    let expected = multiply(
      &multiply(&XYZ_TO_SRGB_D65, &bradford_adaption_matrix(&Illuminant::D50, &Illuminant::D65)),
      &forward_matrix,
    );
    assert_matrix_eq(&cam2rgb, &expected);
  }

  #[test]
  fn develop_applies_profile_render() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut rawimage = calibrated_rawimage(None, None, None);
    rawimage.data = RawImageData::Float(vec![0.2, 0.4, 0.3]);
    let without_render: Vec<ProcessingStep> = RawDevelop::default()
      .steps
      .into_iter()
      .filter(|step| *step != ProcessingStep::ProfileRender)
      .collect();
    let develop = |rawimage: &RawImage, steps: &[ProcessingStep]| -> crate::Result<[f32; 3]> {
      match RawDevelop::new_with(steps).develop_intermediate(rawimage)? {
        Intermediate::ThreeColor(pixels) => Ok(pixels.data[0]),
        _ => panic!("expected three color image"),
      }
    };
    // Without tables, the step has no effect
    assert_eq!(develop(&rawimage, &RawDevelop::default().steps)?, develop(&rawimage, &without_render)?);
    // The tone curve brightens the image
    rawimage.camera_profiles[0].tone_curve = Some(vec![0.0, 0.0, 0.5, 0.8, 1.0, 1.0]);
    let rendered = develop(&rawimage, &RawDevelop::default().steps)?;
    let plain = develop(&rawimage, &without_render)?;
    assert!(rendered.iter().zip(plain).all(|(a, b)| *a > b), "{:?} {:?}", rendered, plain);
    Ok(())
  }

  #[test]
  fn develop_jpeg_with_exif() -> std::result::Result<(), Box<dyn std::error::Error>> {
    use crate::{camera_profile::CameraProfile, exif::Exif};
    use image::{ColorType, ImageDecoder, codecs::jpeg::JpegDecoder};

    let mut camera = Camera::new();
    camera.clean_make = "DNGLab".into();
    camera.clean_model = "Synthetic".into();
    let ramp = (0..16 * 8 * 3).map(|i| ((i / 3) % 16) as f32 / 16.0).collect();
    let mut rawimage = RawImage::new_with_data(
      camera.clone(),
      RawImageData::Float(ramp),
      16 * 3,
      8,
      3,
      [1.0, 1.0, 1.0, f32::NAN],
      RawPhotometricInterpretation::LinearRaw,
      Some(BlackLevel::zero(1, 1, 3)),
      Some(WhiteLevel::new(vec![1; 3])),
      false,
    );
    rawimage.camera_profiles = vec![CameraProfile {
      calibration_illuminant: [Some(Illuminant::D65), None, None],
      color_matrix: [Some(transform_2d(&identity::<3>())), None, None],
      ..Default::default()
    }];
    let metadata = RawMetadata::new(&camera, Exif::default());

    let mut jpeg = Vec::new();
    RawDevelop::default().develop_jpeg(&rawimage, &metadata, &mut jpeg, 90)?;

    let mut decoder = JpegDecoder::new(Cursor::new(&jpeg))?;
    let exif = decoder.exif_metadata()?.ok_or("no EXIF data")?;
    assert!(exif.windows(6).any(|window| window == b"DNGLab"));
    let image = DynamicImage::from_decoder(decoder)?;
    assert_eq!(image.color(), ColorType::Rgb8);
    assert_eq!((image.width(), image.height()), (16, 8));
    Ok(())
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

  #[test]
  fn develop_applies_baseline_exposure() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut rawimage = RawImage::new_with_data(
      Camera::default(),
      RawImageData::Float(vec![0.25; 8 * 6 * 3]),
      8 * 3,
      6,
      3,
      [1.0, 1.0, 1.0, f32::NAN],
      RawPhotometricInterpretation::LinearRaw,
      Some(BlackLevel::zero(1, 1, 3)),
      Some(WhiteLevel::new(vec![1; 3])),
      false,
    );
    rawimage.baseline_exposure = Some(0.5);
    // BaselineExposureOffset is part of the camera profile
    rawimage.camera_profiles = vec![crate::camera_profile::CameraProfile {
      baseline_exposure_offset: Some(0.5),
      ..Default::default()
    }];
    let develop = RawDevelop::new_with(&[ProcessingStep::Rescale, ProcessingStep::Demosaic, ProcessingStep::BaselineExposure]);
    let Intermediate::ThreeColor(pixels) = develop.develop_intermediate(&rawimage)? else {
      panic!("expected three color image");
    };
    assert!(pixels.data.iter().flatten().all(|v| *v == 0.5));
    Ok(())
  }
}
