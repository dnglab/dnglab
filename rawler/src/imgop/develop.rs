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
  let develop = RawDevelop::default();
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
      imgop::render::{StepSet, WhiteBalancePlacement},
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
        ("rggb_camera", rggb(), camera),
        ("rggb_no_wb", rggb(), without(WhiteBalance)),
        ("rggb_linear", rggb(), without(TransferFunction)),
        ("rggb_full_frame", rggb(), without(CropActiveArea)),
        ("rggb_profile", rggb_profile, all.clone()),
        ("rggb_negative_exposure", rggb_negative_exposure, all.clone()),
        ("cygm", cygm, all.clone()),
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
      ("rggb", 24, 16, 3, &[380.48748272657394, 18095.475613176823, 377.5045028641137, 232.10450096428394, 11251.507007345557, 147.3522950384351, 353.6434442996979, 17019.469807982445, 328.740341114098]),
      ("rggb_wb_after_demosaic", 24, 16, 3, &[381.2625027894974, 18131.435772717, 378.8702401502903, 231.30694465339184, 11229.669638305902, 146.84756262907865, 355.5833469033241, 17063.666516900063, 332.00312421469033]),
      ("rggb_camera", 24, 16, 3, &[201.33198672533035, 9818.438517972827, 115.96816916500968, 204.2414404898882, 9990.435259357095, 119.04132786895539, 201.55772396922112, 9910.428465634584, 115.02212716218857]),
      ("rggb_no_wb", 24, 16, 3, &[281.41804756224155, 13544.18863427639, 211.0733201286009, 287.0341155230999, 13839.717891156673, 218.97909127970485, 284.187255859375, 13729.439485132694, 214.02814418573718]),
      ("rggb_linear", 24, 16, 3, &[376.7854283154011, 17939.470737874508, 371.60039229691654, 133.27989721298218, 6566.758106201887, 58.836271068143546, 322.9456180334091, 15769.889257758856, 283.0030790269616]),
      ("rggb_full_frame", 24, 16, 3, &[381.32452404499054, 18132.763287246227, 378.87638290411275, 231.77392564713955, 11225.565983697772, 147.23389698237116, 354.80902659893036, 17045.520306646824, 330.62366710491506]),
      ("rggb_profile", 24, 16, 3, &[383.4662112593651, 18215.506040215492, 383.0383059067018, 296.44845950603485, 14440.323270201683, 241.9604888002999, 373.6859050989151, 17835.488741457462, 364.526467670008]),
      ("rggb_negative_exposure", 24, 16, 3, &[372.73057693243027, 17782.682923793793, 363.7473612794026, 147.27665086835623, 7239.980796560645, 69.52375685313699, 337.6899732351303, 16378.00525778532, 304.22489976021023]),
      ("cygm", 24, 16, 3, &[330.2392064332962, 15989.270647585392, 294.36408670208885, 242.15305441617966, 11660.862674564123, 160.30449729856224, 362.35906931757927, 17345.425794780254, 344.5220274683217]),
      ("xtrans", 28, 16, 3, &[441.1662734746933, 20135.024219989777, 435.23239948117185, 272.69089791178703, 12451.46948003769, 171.7381630839859, 408.4678649902344, 18654.983914613724, 376.8882214632959]),
      ("mono", 24, 16, 1, &[286.27965730428696, 13782.818696141243, 217.83812832726255]),
      ("linear_raw_profile", 16, 12, 3, &[185.23571345210075, 8951.324946820736, 181.01863179486003, 145.2774385213852, 7214.587505862117, 118.69475065947488, 188.05537682771683, 8992.080503284931, 184.6161640864077]),
    ];

    /// Largest absolute difference of the samples of two images of the same size
    fn max_difference(a: &RenderedImage, b: &RenderedImage) -> f32 {
      assert_eq!((a.dim(), a.channels), (b.dim(), b.channels));
      a.data.iter().zip(&b.data).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max)
    }

    fn render_with(rawimage: &RawImage, white_balance: WhiteBalancePlacement) -> RenderedImage {
      let options = RenderOptions {
        white_balance,
        ..Default::default()
      };
      render(rawimage.clone(), &options).expect("render")
    }

    #[test]
    fn white_balance_placement_matters_for_ppg_only() {
      // Values that don't clip when white balanced: bilinear demosaicing works on
      // each color plane, so it doesn't matter if the planes are balanced before.
      // PPG interpolates by the differences of the planes.
      let cygm_matrix = [[0.2, 0.6, 0.5], [0.7, 0.8, 0.0], [0.1, 0.9, 0.1], [0.8, 0.2, 0.6]];
      for (pattern, colors, wb, matrix, bilinear) in [
        ("RGGB", "RGB", [2.0, 1.0, 1.5, f32::NAN], transform_2d(&COLOR_MATRIX), false),
        (XTRANS, "RGB", [2.0, 1.0, 1.5, f32::NAN], transform_2d(&COLOR_MATRIX), true),
        ("CYGM", "CYGM", [1.5, 1.2, 1.0, 1.4], transform_2d(&cygm_matrix), true),
      ] {
        let mut rawimage = cfa_rawimage(pattern, colors, 36, 24, wb, matrix);
        rawimage.data = RawImageData::Integer(samples(36, 24, 1, 64, 1300));
        let difference = max_difference(
          &render_with(&rawimage, WhiteBalancePlacement::BeforeDemosaic),
          &render_with(&rawimage, WhiteBalancePlacement::AfterDemosaic),
        );
        assert_eq!(difference < 1e-5, bilinear, "{}: {}", pattern, difference);
      }
    }

    #[test]
    fn clipped_highlights_are_white() {
      // White balanced values are limited to the camera white, so clipped highlights stay neutral
      let mut rawimage = rggb();
      rawimage.data = RawImageData::Integer(vec![4095; 32 * 24]);
      for placement in [WhiteBalancePlacement::BeforeDemosaic, WhiteBalancePlacement::AfterDemosaic] {
        let image = render_with(&rawimage, placement);
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
        let cropped = RawDevelop::default().develop_image(rawimage.clone()).expect("develop");
        let steps: Vec<ProcessingStep> = RawDevelop::default()
          .steps
          .into_iter()
          .filter(|step| *step != ProcessingStep::CropDefault)
          .collect();
        let mut expected = RawDevelop::new_with(&steps).develop_image(rawimage).expect("develop");
        expected.crop(crop.rebase(&active_area).expect("crop inside of the active area")).expect("crop");
        assert_eq!(cropped.dim(), expected.dim(), "{}", pattern);
        assert!(cropped.data == expected.data, "{}", pattern);
      }
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
