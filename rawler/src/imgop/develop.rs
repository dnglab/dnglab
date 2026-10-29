use std::{
  ffi::OsStr,
  io::{self, Cursor, Seek, Write},
  path::Path,
};

use image::{ImageEncoder, codecs::jpeg::JpegEncoder};

use crate::{
  RawImage,
  decoders::{RawDecodeParams, RawMetadata, validate_image_index},
  dng::{CropMode, opcodes::OpcodeSelector},
  formats::tiff::{DirectoryWriter, TiffWriter},
  imgop::render::{RenderOptions, RenderedImage, render},
  rawsource::RawSource,
  tags::{ExifTag, TiffCommonTag},
};

use super::convert_from_f32_scaled_u16;

pub use super::render::ProcessingStep;

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
  let develop = RawDevelop::with_crop(params.crop);
  match params.format {
    ProcessedImageFormat::Tiff => develop.develop(rawimage, &metadata, image_file)?,
    ProcessedImageFormat::Jpeg => develop.develop_jpeg(rawimage, &metadata, image_file, params.jpeg_quality)?,
  }
  Ok(())
}

/// Development of raw images with a set of processing steps
///
/// The order of the steps is defined by the rendering, see
/// [`plan`](crate::imgop::render::plan).
#[derive(Clone)]
pub struct RawDevelop {
  pub steps: Vec<ProcessingStep>,
}

impl Default for RawDevelop {
  fn default() -> Self {
    Self {
      steps: ProcessingStep::ALL.to_vec(),
    }
  }
}

impl RawDevelop {
  pub fn new_with(steps: &[ProcessingStep]) -> Self {
    Self { steps: Vec::from(steps) }
  }

  /// Development with all steps, the image is cropped by `crop`: to the
  /// DefaultCrop, the ActiveArea or not at all
  pub fn with_crop(crop: CropMode) -> Self {
    let excluded: &[ProcessingStep] = match crop {
      CropMode::Best => &[],
      CropMode::ActiveArea => &[ProcessingStep::CropDefault],
      CropMode::None => &[ProcessingStep::CropActiveArea, ProcessingStep::CropDefault],
    };
    Self {
      steps: ProcessingStep::ALL.into_iter().filter(|step| !excluded.contains(step)).collect(),
    }
  }

  /// Options of the rendering with the steps of this development
  pub fn render_options(&self) -> RenderOptions {
    RenderOptions {
      steps: self.steps.iter().copied().collect(),
      ..Default::default()
    }
  }

  /// Develop raw image. If demosaic is disabled or camera raw is monochrome,
  /// the image has one channel.
  ///
  /// The image data of `rawimage` is processed in place, clone it to keep it.
  pub fn develop_image(&self, rawimage: RawImage) -> crate::Result<RenderedImage> {
    render(rawimage, &self.render_options())
  }

  /// Develop raw image and write result into TIFF.
  /// If demosaic is disabled or camera raw is monochrome, the TIFF
  /// has only one color channel.
  pub fn develop<W>(&self, rawimage: RawImage, md: &RawMetadata, writer: W) -> crate::Result<()>
  where
    W: io::Write + io::Seek,
  {
    let mut tiff = TiffWriter::new(writer)?;
    let mut root_ifd = Self::metadata_ifd(&mut tiff, &rawimage, md)?;

    let image = self.develop_image(rawimage)?;

    let data = convert_from_f32_scaled_u16(&image.data, 0, u16::MAX);
    let (strip_rows, strips) = tiff.write_strips_lzw(&data, image.channels, image.dim(), 0)?;
    let strip_offsets: Vec<u32> = strips.iter().map(|(offset, _)| *offset).collect();
    let strip_bytes: Vec<u32> = strips.iter().map(|(_, bytes)| *bytes).collect();
    root_ifd.add_tag(TiffCommonTag::Compression, 5);
    root_ifd.add_tag(TiffCommonTag::Predictor, 1);
    root_ifd.add_tag(TiffCommonTag::StripOffsets, &strip_offsets);
    root_ifd.add_tag(TiffCommonTag::StripByteCounts, &strip_bytes);
    // Four channels are written as RGB with an extra channel
    root_ifd.add_tag(TiffCommonTag::BitsPerSample, &vec![16_u16; image.channels]);
    root_ifd.add_tag(TiffCommonTag::SamplesPerPixel, [image.channels as u16]);
    root_ifd.add_tag(TiffCommonTag::PhotometricInt, [if image.channels == 1 { 1_u16 } else { 2 }]);
    root_ifd.add_tag(TiffCommonTag::RowsPerStrip, strip_rows);
    root_ifd.add_tag(TiffCommonTag::ImageWidth, image.width as u16);
    root_ifd.add_tag(TiffCommonTag::ImageLength, image.height as u16);

    tiff.build(root_ifd)?;

    Ok(())
  }

  /// Develop raw image and write result as 8-bit JPEG with EXIF data
  pub fn develop_jpeg<W>(&self, rawimage: RawImage, md: &RawMetadata, writer: W, quality: u8) -> crate::Result<()>
  where
    W: io::Write,
  {
    // EXIF data is a TIFF structure without image data
    let mut exif = Cursor::new(Vec::new());
    let mut tiff = TiffWriter::new(&mut exif)?;
    let root_ifd = Self::metadata_ifd(&mut tiff, &rawimage, md)?;
    tiff.build(root_ifd)?;
    let exif = exif.into_inner();

    let image = self.develop_image(rawimage)?.to_dynamic_image().ok_or("failed to convert to dynamic image")?;

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

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{
    CFA, RawImageData,
    decoders::Camera,
    dng::opcodes::{FixBadPixelsConstant, Opcode, OpcodeList, TrimBounds, WarpRectilinear, WarpRectilinearCoeffs},
    imgop::{
      Dim2, Point, Rect,
      matrix::{FlatMatrix, identity, transform_2d},
      xyz::Illuminant,
    },
    pixarray::PixU16,
    rawimage::{BlackLevel, CFAConfig, RawPhotometricInterpretation, WhiteLevel},
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

  /// Image of one pixel with a D65 profile
  fn d65_rawimage() -> RawImage {
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
    let mut profile = crate::camera_profile::CameraProfile::default();
    profile.calibration_illuminant[0] = Some(Illuminant::D65);
    profile.color_matrix[0] = Some(transform_2d(&COLOR_MATRIX));
    rawimage.camera_profiles = vec![profile];
    rawimage
  }

  /// Pixels of an image with three channels
  fn rgb_pixels(image: &RenderedImage) -> &[[f32; 3]] {
    image.view().as_array_slice::<3>().expect("three color image")
  }

  #[test]
  fn develop_applies_profile_render() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut rawimage = d65_rawimage();
    rawimage.data = RawImageData::Float(vec![0.2, 0.4, 0.3]);
    let without_render: Vec<ProcessingStep> = RawDevelop::default()
      .steps
      .into_iter()
      .filter(|step| *step != ProcessingStep::ProfileRender)
      .collect();
    let develop = |rawimage: &RawImage, steps: &[ProcessingStep]| -> crate::Result<[f32; 3]> {
      Ok(rgb_pixels(&RawDevelop::new_with(steps).develop_image(rawimage.clone())?)[0])
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
    use image::{ColorType, DynamicImage, ImageDecoder, codecs::jpeg::JpegDecoder};

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
    RawDevelop::default().develop_jpeg(rawimage, &metadata, &mut jpeg, 90)?;

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
    let image = develop.develop_image(rawimage)?;
    let pixels = rgb_pixels(&image);
    for (i, pixel) in pixels.iter().enumerate() {
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
    let image = develop.develop_image(rawimage)?;
    assert_eq!(image.dim(), Dim2::new(8, 6));
    let pixels = rgb_pixels(&image);
    let expected = 2048.0 / 4095.0;
    assert!(pixels.iter().flatten().all(|v| (v - expected).abs() < 1e-4));
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
    let image = RawDevelop::new_with(&[ProcessingStep::Rescale]).develop_image(rawimage)?;
    assert_eq!(image.channels, 1);
    assert_eq!(image.data[6 * 16 + 6], 1300.0 / 4095.0);
    Ok(())
  }

  /// White and a dark gray pixel with a D65 profile and a BaselineExposure of -1 EV
  fn negative_exposure_rawimage() -> RawImage {
    let mut rawimage = d65_rawimage();
    rawimage.width = 2;
    rawimage.data = RawImageData::Float(vec![1.0, 1.0, 1.0, 0.2, 0.2, 0.2]);
    rawimage.baseline_exposure = Some(-1.0);
    rawimage
  }

  #[test]
  fn develop_applies_negative_baseline_exposure() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let rawimage = negative_exposure_rawimage();
    // White stays white, dark values are darkened by 1 EV
    let steps: Vec<ProcessingStep> = RawDevelop::default()
      .steps
      .into_iter()
      .filter(|step| *step != ProcessingStep::TransferFunction)
      .collect();
    let image = RawDevelop::new_with(&steps).develop_image(rawimage.clone())?;
    let pixels = rgb_pixels(&image);
    for (pixel, expected) in pixels.iter().zip([1.0, 0.1]) {
      assert!(pixel.iter().all(|v| (v - expected).abs() < 1e-3), "{:?}", pixel);
    }
    // Without Calibrate, the BaselineExposure step applies the curve
    let develop = RawDevelop::new_with(&[ProcessingStep::Rescale, ProcessingStep::Demosaic, ProcessingStep::BaselineExposure]);
    let image = develop.develop_image(rawimage)?;
    let pixels = rgb_pixels(&image);
    for (pixel, expected) in pixels.iter().zip([1.0, 0.1]) {
      assert!(pixel.iter().all(|v| (v - expected).abs() < 1e-5), "{:?}", pixel);
    }
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
    let image = develop.develop_image(rawimage)?;
    let pixels = rgb_pixels(&image);
    assert!(pixels.iter().flatten().all(|v| *v == 0.5));
    Ok(())
  }

  /// Output of the development of synthetic images, pinned to detect unintended
  /// changes. Run with `RAWLER_UPDATE_GOLDEN=1` and `--nocapture` to print the
  /// current values instead of comparing them.
  mod golden {
    use super::*;
    use crate::{
      camera_profile::{CameraCalibration, CameraProfile, GainData, GainTableMap},
      cfa::PlaneColor,
      imgop::render::{DemosaicQuality, StepSet, WhiteBalancePlacement},
    };

    /// Allowed deviation of the fingerprints, relative to the expected value,
    /// for differences of floating point functions between platforms
    const TOLERANCE: f64 = 1e-6;

    const XTRANS: &str = "GGRGGBGGBGGRBRGRBGGGBGGRGGRGGBRBGBRG";

    /// Gradient with noise between `black` and `white`, and a clipped block
    fn samples(width: usize, height: usize, channels: usize, black: u16, white: u16) -> Vec<u16> {
      let mut state = 0x2545_f491_u32;
      let range = u32::from(white - black);
      let scale = (width * 7 + height * 5 + channels * 11) as u32;
      (0..width * height * channels)
        .map(|i| {
          state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
          let (x, y, c) = ((i / channels) % width, (i / channels) / width, i % channels);
          if (4..8).contains(&x) && (4..8).contains(&y) {
            return white;
          }
          let gradient = (x * 7 + y * 5 + c * 11) as u32 * range / scale;
          let noise = (state >> 22) % (range / 16);
          black + (gradient + noise).min(range) as u16
        })
        .collect()
    }

    fn d65_profile(color_matrix: FlatMatrix) -> CameraProfile {
      CameraProfile {
        calibration_illuminant: [Some(Illuminant::D65), None, None],
        color_matrix: [Some(color_matrix), None, None],
        ..Default::default()
      }
    }

    /// Hue shifts, saturation and value scales that vary across the table
    fn hue_sat_map(dims: [u32; 3]) -> Vec<f32> {
      let count = (dims[0] * dims[1] * dims[2]) as usize;
      (0..count)
        .flat_map(|i| [(i % 6) as f32 * 4.0 - 10.0, 1.0 + 0.05 * (i % 3) as f32, 1.0 - 0.03 * (i % 2) as f32])
        .collect()
    }

    /// Rendering tables, a ForwardMatrix and a positive BaselineExposure
    fn add_profile_tables(rawimage: &mut RawImage) {
      let profile = &mut rawimage.camera_profiles[0];
      profile.forward_matrix[0] = Some(transform_2d(&FORWARD_MATRIX));
      profile.hue_sat_map_dims = Some([6, 2, 1]);
      profile.hue_sat_map_data[0] = Some(hue_sat_map([6, 2, 1]));
      profile.look_table_dims = Some([6, 2, 2]);
      profile.look_table_data = Some(hue_sat_map([6, 2, 2]));
      profile.tone_curve = Some(vec![0.0, 0.0, 0.25, 0.3, 0.5, 0.6, 1.0, 1.0]);
      profile.gain_table_map = Some(GainTableMap {
        points: [2, 2, 1],
        spacing: [1.0, 1.0],
        origin: [0.0, 0.0],
        input_weights: [0.0, 0.0, 0.0, 0.0, 1.0],
        gamma: 1.0,
        gain_min: 0.9,
        gain_max: 1.2,
        gains: GainData::F32(vec![1.0, 1.2, 0.9, 1.1]),
      });
      rawimage.baseline_exposure = Some(0.5);
    }

    /// CFA image of 12 bits with a black level of 64, an ActiveArea and a DefaultCrop
    fn cfa_rawimage(pattern: &str, colors: &str, width: usize, height: usize, wb: [f32; 4], color_matrix: FlatMatrix) -> RawImage {
      let mut camera = Camera::new();
      camera.cfa = CFA::new(pattern);
      camera.plane_color = PlaneColor::new(colors);
      let photometric = RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&camera));
      let mut rawimage = RawImage::new(
        camera,
        PixU16::new_with(samples(width, height, 1, 64, 4095), width, height),
        1,
        wb,
        photometric,
        Some(BlackLevel::new(&[64_u32; 4], 2, 2, 1)),
        Some(WhiteLevel::new_bits(12, 1)),
        false,
      );
      rawimage.active_area = Some(Rect::new(Point::new(2, 2), Dim2::new(width - 4, height - 4)));
      rawimage.crop_area = Some(Rect::new(Point::new(4, 4), Dim2::new(width - 8, height - 8)));
      rawimage.camera_profiles = vec![d65_profile(color_matrix)];
      rawimage
    }

    fn rggb() -> RawImage {
      cfa_rawimage("RGGB", "RGB", 32, 24, [2.0, 1.0, 1.5, f32::NAN], transform_2d(&COLOR_MATRIX))
    }

    fn mono() -> RawImage {
      let (width, height) = (32, 24);
      let mut rawimage = RawImage::new(
        Camera::new(),
        PixU16::new_with(samples(width, height, 1, 64, 4095), width, height),
        1,
        [f32::NAN; 4],
        RawPhotometricInterpretation::BlackIsZero,
        Some(BlackLevel::new(&[64_u32], 1, 1, 1)),
        Some(WhiteLevel::new_bits(12, 1)),
        false,
      );
      rawimage.crop_area = Some(Rect::new(Point::new(4, 4), Dim2::new(width - 8, height - 8)));
      rawimage
    }

    fn linear_raw() -> RawImage {
      let (width, height) = (16, 12);
      let data = samples(width, height, 3, 0, 4095).iter().map(|v| f32::from(*v) / 4095.0).collect();
      let mut rawimage = RawImage::new_with_data(
        Camera::default(),
        RawImageData::Float(data),
        width * 3,
        height,
        3,
        [1.8, 1.0, 1.4, f32::NAN],
        RawPhotometricInterpretation::LinearRaw,
        Some(BlackLevel::zero(1, 1, 3)),
        Some(WhiteLevel::new(vec![1; 3])),
        false,
      );
      rawimage.camera_profiles = vec![d65_profile(transform_2d(&COLOR_MATRIX))];
      add_profile_tables(&mut rawimage);
      rawimage
    }

    fn cases() -> Vec<(&'static str, RawImage, RenderOptions)> {
      use ProcessingStep::*;
      let all = RenderOptions::default();
      let without = |step: ProcessingStep| RenderOptions {
        steps: StepSet::all().without(step),
        ..Default::default()
      };
      let wb_after_demosaic = RenderOptions {
        white_balance: WhiteBalancePlacement::AfterDemosaic,
        ..Default::default()
      };
      let camera = RenderOptions {
        steps: [Rescale, Demosaic, CropActiveArea, CropDefault].into_iter().collect(),
        ..Default::default()
      };
      let preview = RenderOptions {
        demosaic: DemosaicQuality::Preview,
        ..Default::default()
      };

      let mut rggb_profile = rggb();
      add_profile_tables(&mut rggb_profile);
      rggb_profile.analog_balance = Some(vec![1.1, 1.0, 0.9]);
      rggb_profile.camera_calibration = CameraCalibration {
        matrices: [Some(transform_2d(&CALIBRATION)), None, None],
        signature: None,
      };
      let mut rggb_negative_exposure = rggb();
      rggb_negative_exposure.baseline_exposure = Some(-1.0);
      let cygm_matrix = [[0.2, 0.6, 0.5], [0.7, 0.8, 0.0], [0.1, 0.9, 0.1], [0.8, 0.2, 0.6]];
      let cygm = cfa_rawimage("CYGM", "CYGM", 32, 24, [1.5, 1.2, 1.0, 1.4], transform_2d(&cygm_matrix));
      let xtrans = cfa_rawimage(XTRANS, "RGB", 36, 24, [2.0, 1.0, 1.5, f32::NAN], transform_2d(&COLOR_MATRIX));

      vec![
        ("rggb", rggb(), all.clone()),
        ("rggb_wb_after_demosaic", rggb(), wb_after_demosaic),
        ("rggb_preview", rggb(), preview.clone()),
        ("rggb_camera", rggb(), camera),
        ("rggb_no_wb", rggb(), without(WhiteBalance)),
        ("rggb_linear", rggb(), without(TransferFunction)),
        ("rggb_full_frame", rggb(), without(CropActiveArea)),
        ("rggb_profile", rggb_profile, all.clone()),
        ("rggb_negative_exposure", rggb_negative_exposure, all.clone()),
        ("cygm", cygm.clone(), all.clone()),
        ("cygm_preview", cygm, preview.clone()),
        ("xtrans", xtrans, all.clone()),
        ("mono", mono(), all.clone()),
        ("linear_raw_profile", linear_raw(), all.clone()),
      ]
    }

    /// Width, height, channels and per channel: sum, position weighted sum and
    /// sum of squares of the samples
    fn fingerprint(rawimage: &RawImage, options: &RenderOptions) -> (usize, usize, usize, Vec<f64>) {
      let image = render(rawimage.clone(), options).expect("develop");
      let (dim, channels, data) = (image.dim(), image.channels, image.data);
      let mut sums = vec![0.0; channels * 3];
      for (i, v) in data.iter().map(|v| f64::from(*v)).enumerate() {
        let c = i % channels;
        sums[c * 3] += v;
        sums[c * 3 + 1] += v * ((i / channels) % 97) as f64;
        sums[c * 3 + 2] += v * v;
      }
      (dim.w, dim.h, channels, sums)
    }

    #[rustfmt::skip]
    const GOLDEN: &[(&str, usize, usize, usize, &[f64])] = &[
      ("rggb", 24, 16, 3, &[380.56808137893677, 18104.740788161755, 377.6961634597931, 232.66582250595093, 11266.001190900803, 147.76858769621805, 353.2642803788185, 17010.45430147648, 328.1159281710242]),
      ("rggb_wb_after_demosaic", 24, 16, 3, &[380.7199194431305, 18114.272230386734, 377.9928879594697, 232.36832988262177, 11259.613187074661, 147.3181136455201, 354.98246800899506, 17047.02375769615, 330.89957972745174]),
      ("rggb_preview", 12, 8, 3, &[95.1130530834198, 4535.024439275265, 94.3125455442176, 57.73037052154541, 2806.694089382887, 36.49492572569757, 89.82149887084961, 4330.648076474667, 84.67189949810677]),
      ("rggb_camera", 24, 16, 3, &[202.01232363283634, 9857.277190387249, 117.52662926537228, 204.2414404898882, 9990.435259357095, 119.04132786895539, 201.11399140954018, 9900.458577096462, 114.79045569113153]),
      ("rggb_no_wb", 24, 16, 3, &[281.4802607446909, 13564.986759513617, 211.7243223425459, 287.3429910540581, 13848.008021116257, 219.29237275252382, 283.6068353652954, 13712.447558701038, 213.1742812932963]),
      ("rggb_linear", 24, 16, 3, &[377.01246961951256, 17960.735143005848, 372.1423593051457, 133.64919804036617, 6578.678369447589, 58.98300084691425, 322.27758035063744, 15752.828521579504, 282.1283962510165]),
      ("rggb_full_frame", 24, 16, 3, &[381.69512236118317, 18150.39451789856, 379.595433615728, 232.32091468572617, 11240.07896360755, 147.64509285845165, 355.1079213619232, 17057.55524724722, 331.2803854717914]),
      ("rggb_profile", 24, 16, 3, &[383.4662112593651, 18215.506040215492, 383.0383059067018, 297.52433839440346, 14476.317115783691, 242.40614491711693, 373.5126699209213, 17833.258824765682, 364.2168519828674]),
      ("rggb_negative_exposure", 24, 16, 3, &[372.9707576036453, 17801.475763201714, 364.28282762169994, 148.17328909039497, 7266.1948882341385, 69.80508398590429, 337.22546261548996, 16365.803864657879, 303.55807226827676]),
      ("cygm", 24, 16, 3, &[330.2392064332962, 15989.270647585392, 294.36408670208885, 242.15305441617966, 11660.862674564123, 160.30449729856224, 362.35906931757927, 17345.425794780254, 344.5220274683217]),
      ("cygm_preview", 12, 8, 3, &[89.73201927542686, 4332.205256462097, 85.09224837936573, 54.80821684002876, 2643.0370524823666, 33.73017430069746, 90.4724417924881, 4360.6897903084755, 85.8589423764323]),
      ("xtrans", 28, 16, 3, &[441.8567228913307, 20118.23803973198, 436.9002191243954, 270.7740049660206, 12374.535669326782, 170.7231797951337, 409.30331003665924, 18666.734287142754, 378.8607051136843]),
      ("mono", 24, 16, 1, &[286.27965730428696, 13782.818696141243, 217.83812832726255]),
      ("linear_raw_profile", 16, 12, 3, &[185.23571345210075, 8951.324946820736, 181.01863179486003, 145.2774385213852, 7214.587505862117, 118.69475065947488, 188.05537682771683, 8992.080503284931, 184.6161640864077]),
    ];

    /// Largest absolute difference of the samples of two images of the same size
    fn max_difference(a: &RenderedImage, b: &RenderedImage) -> f32 {
      assert_eq!((a.dim(), a.channels), (b.dim(), b.channels));
      a.data.iter().zip(&b.data).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max)
    }

    fn render_with(rawimage: &RawImage, white_balance: WhiteBalancePlacement, demosaic: DemosaicQuality) -> RenderedImage {
      let options = RenderOptions {
        white_balance,
        demosaic,
        ..Default::default()
      };
      render(rawimage.clone(), &options).expect("render")
    }

    #[test]
    fn white_balance_placement_matters_for_color_differences() {
      // Values that don't clip when white balanced: bilinear demosaicing works on
      // each color plane, so it doesn't matter if the planes are balanced before.
      // PPG interpolates by the differences of the planes, Markesteijn chooses
      // the direction of the interpolation by the colors.
      let cygm_matrix = [[0.2, 0.6, 0.5], [0.7, 0.8, 0.0], [0.1, 0.9, 0.1], [0.8, 0.2, 0.6]];
      let rgb = [2.0, 1.0, 1.5, f32::NAN];
      for (pattern, colors, wb, matrix, demosaic, bilinear) in [
        ("RGGB", "RGB", rgb, transform_2d(&COLOR_MATRIX), DemosaicQuality::Quality, false),
        (XTRANS, "RGB", rgb, transform_2d(&COLOR_MATRIX), DemosaicQuality::Quality, false),
        (XTRANS, "RGB", rgb, transform_2d(&COLOR_MATRIX), DemosaicQuality::Fast, true),
        ("CYGM", "CYGM", [1.5, 1.2, 1.0, 1.4], transform_2d(&cygm_matrix), DemosaicQuality::Quality, true),
      ] {
        let mut rawimage = cfa_rawimage(pattern, colors, 96, 72, wb, matrix);
        rawimage.data = RawImageData::Integer(samples(96, 72, 1, 64, 1300));
        let difference = max_difference(
          &render_with(&rawimage, WhiteBalancePlacement::BeforeDemosaic, demosaic),
          &render_with(&rawimage, WhiteBalancePlacement::AfterDemosaic, demosaic),
        );
        assert_eq!(difference < 1e-5, bilinear, "{} {:?}: {}", pattern, demosaic, difference);
      }
    }

    /// RMSE of PPG demosaicing white balanced camera values of `scene`, without
    /// a border of 4 pixels, for both placements of the white balance
    fn ppg_error(scene: impl Fn(f32, f32) -> [f32; 3]) -> [f64; 2] {
      use ProcessingStep::*;
      let (width, height) = (64, 48);
      let wb = [2.0, 1.0, 1.5, f32::NAN];
      let mut rawimage = cfa_rawimage("RGGB", "RGB", width, height, wb, transform_2d(&COLOR_MATRIX));
      let cfa = CFA::new("RGGB");
      rawimage.data = RawImageData::Integer(
        (0..width * height)
          .map(|i| {
            let (x, y) = (i % width, i / width);
            let plane = cfa.color_at(y, x);
            (64.0 + scene(x as f32, y as f32)[plane] / wb[plane] * 4031.0).round() as u16
          })
          .collect(),
      );
      [WhiteBalancePlacement::BeforeDemosaic, WhiteBalancePlacement::AfterDemosaic].map(|white_balance| {
        let options = RenderOptions {
          steps: [Rescale, Demosaic, CropActiveArea, WhiteBalance].into_iter().collect(),
          white_balance,
          ..Default::default()
        };
        let image = render(rawimage.clone(), &options).expect("render");
        // The image is the ActiveArea at (2, 2)
        let (mut sum, mut count) = (0.0, 0);
        for y in 4..image.height - 4 {
          for x in 4..image.width - 4 {
            let truth = scene((x + 2) as f32, (y + 2) as f32);
            for (c, truth) in truth.iter().enumerate() {
              sum += f64::from(image.data[(y * image.width + x) * 3 + c] - truth).powi(2);
              count += 1;
            }
          }
        }
        (sum / f64::from(count)).sqrt()
      })
    }

    #[test]
    fn white_balance_before_demosaic_reduces_ppg_error() {
      // A neutral edge is reconstructed exactly from white balanced data
      let [before, after] = ppg_error(|x, _| [if x < 32.0 { 0.45 } else { 0.1 }; 3]);
      assert!(before < 1e-3 && after > 10.0 * before, "edge: {} {}", before, after);
      // Texture with slowly changing colors
      let [before, after] = ppg_error(|x, y| {
        let l = 0.3 + 0.15 * (x / 2.5).sin() * (y / 3.5).cos();
        [l * (1.0 + 0.4 * (y / 15.0).sin()), l, l * (1.0 + 0.4 * (x / 13.0).cos())]
      });
      assert!(before < after, "texture: {} {}", before, after);
      // Zone plate with fine detail
      let [before, after] = ppg_error(|x, y| [0.3 + 0.25 * (((x - 32.0).powi(2) + (y - 24.0).powi(2)) / 60.0).sin(); 3]);
      assert!(before < after, "zone plate: {} {}", before, after);
    }

    #[test]
    fn clipped_highlights_are_white() {
      // White balanced values are limited to the camera white, so clipped highlights stay neutral
      let mut rawimage = rggb();
      rawimage.data = RawImageData::Integer(vec![4095; 32 * 24]);
      for placement in [WhiteBalancePlacement::BeforeDemosaic, WhiteBalancePlacement::AfterDemosaic] {
        let image = render_with(&rawimage, placement, DemosaicQuality::Quality);
        assert!(image.data.iter().all(|v| (v - 1.0).abs() < 1e-4), "{:?}", placement);
      }
    }

    #[test]
    fn crop_in_demosaic_equals_crop_after_rendering() {
      // A DefaultCrop far from the borders of the ActiveArea, the gain table map
      // depends on the position of the pixels in the ActiveArea
      let crop = Rect::new(Point::new(40, 30), Dim2::new(24, 18));
      let cygm_matrix = [[0.2, 0.6, 0.5], [0.7, 0.8, 0.0], [0.1, 0.9, 0.1], [0.8, 0.2, 0.6]];
      for (pattern, colors, wb, matrix) in [
        ("RGGB", "RGB", [2.0, 1.0, 1.5, f32::NAN], transform_2d(&COLOR_MATRIX)),
        (XTRANS, "RGB", [2.0, 1.0, 1.5, f32::NAN], transform_2d(&COLOR_MATRIX)),
        ("CYGM", "CYGM", [1.5, 1.2, 1.0, 1.4], transform_2d(&cygm_matrix)),
      ] {
        let mut rawimage = cfa_rawimage(pattern, colors, 96, 72, wb, matrix);
        add_profile_tables(&mut rawimage);
        rawimage.crop_area = Some(crop);
        let active_area = rawimage.active_area.expect("active area");
        for demosaic in [DemosaicQuality::Quality, DemosaicQuality::Preview] {
          let options = RenderOptions {
            demosaic,
            ..Default::default()
          };
          let cropped = render(rawimage.clone(), &options).expect("render");
          let full = RenderOptions {
            steps: StepSet::all().without(ProcessingStep::CropDefault),
            ..options
          };
          let mut expected = render(rawimage.clone(), &full).expect("render");
          // Bayer data is demosaiced to half size for previews
          let mut area = crop.rebase(&active_area).expect("crop inside of the active area");
          if demosaic == DemosaicQuality::Preview && pattern != XTRANS {
            area.scale(0.5);
          }
          expected.crop(area).expect("crop");
          assert_eq!(cropped.dim(), expected.dim(), "{} {:?}", pattern, demosaic);
          assert!(cropped.data == expected.data, "{} {:?}", pattern, demosaic);
        }
      }
    }

    #[test]
    fn crop_modes() {
      let size = |rawimage: &RawImage, crop| {
        let image = RawDevelop::with_crop(crop).develop_image(rawimage.clone()).expect("develop");
        (image.width, image.height)
      };
      // CFA data is cropped while it is demosaiced
      let rggb = rggb();
      assert_eq!(size(&rggb, CropMode::Best), (24, 16));
      assert_eq!(size(&rggb, CropMode::ActiveArea), (28, 20));
      assert_eq!(size(&rggb, CropMode::None), (32, 24));
      // Other data before the color operations, the gain table map keeps its positions
      let mut linear_raw = linear_raw();
      linear_raw.active_area = Some(Rect::new(Point::new(1, 1), Dim2::new(14, 10)));
      linear_raw.crop_area = Some(Rect::new(Point::new(2, 2), Dim2::new(12, 8)));
      assert_eq!(size(&linear_raw, CropMode::Best), (12, 8));
      let active_area = RawDevelop::with_crop(CropMode::ActiveArea).develop_image(linear_raw.clone()).expect("develop");
      let mut expected = RawDevelop::with_crop(CropMode::None).develop_image(linear_raw).expect("develop");
      assert_eq!((expected.width, expected.height), (16, 12));
      expected.crop(Rect::new(Point::new(1, 1), Dim2::new(14, 10))).expect("crop");
      assert!(active_area.dim() == expected.dim() && active_area.data == expected.data);
    }

    #[test]
    fn preview_of_flat_field() {
      // Each pixel of the preview is a block of 2x2 pixels, a flat field keeps its color
      let mut rawimage = rggb();
      rawimage.data = RawImageData::Integer(vec![1500; 32 * 24]);
      let full = render(rawimage.clone(), &RenderOptions::default()).expect("render");
      let options = RenderOptions {
        demosaic: DemosaicQuality::Preview,
        ..Default::default()
      };
      let preview = render(rawimage, &options).expect("render");
      assert_eq!((preview.width * 2, preview.height * 2), (full.width, full.height));
      let pixel = rgb_pixels(&full)[0];
      assert!(
        rgb_pixels(&preview).iter().all(|p| p.iter().zip(pixel).all(|(a, b)| (a - b).abs() < 1e-6)),
        "{:?}",
        pixel
      );
    }

    #[test]
    fn develop_output_is_unchanged() {
      let update = std::env::var_os("RAWLER_UPDATE_GOLDEN").is_some();
      for (name, rawimage, options) in cases() {
        let (width, height, channels, sums) = fingerprint(&rawimage, &options);
        if update {
          println!("      (\"{}\", {}, {}, {}, &{:?}),", name, width, height, channels, sums);
          continue;
        }
        let (_, ew, eh, ech, expected) = GOLDEN.iter().find(|golden| golden.0 == name).expect("no golden values");
        assert_eq!((width, height, channels), (*ew, *eh, *ech), "{}", name);
        for (value, expected_value) in sums.iter().zip(expected.iter()) {
          assert!(
            (value - expected_value).abs() <= TOLERANCE * expected_value.abs().max(1.0),
            "{}: {:?} != {:?}",
            name,
            sums,
            expected
          );
        }
      }
    }
  }
}
