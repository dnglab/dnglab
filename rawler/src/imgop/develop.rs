use std::{
  ffi::OsStr,
  io::{self, Cursor, Seek, Write},
  path::Path,
};

use byteorder::{BigEndian, WriteBytesExt};
use image::{ImageEncoder, codecs::jpeg::JpegEncoder};

use crate::{
  RawImage,
  decoders::{Orientation, RawDecodeParams, RawMetadata, validate_image_index},
  dng::{
    CropMode,
    dcp::CameraProfileFile,
    opcodes::{Opcode, OpcodeSelector, OpcodeStage},
  },
  formats::tiff::{DirectoryWriter, TiffError, TiffWriter},
  imgop::{
    Dim2, Point, Rect,
    color::{OutputSpace, TransferFunction},
    denoise::DenoiseStrength,
    highlights::HighlightMode,
    render::{BayerDemosaic, RenderOptions, RenderedImage, XTransDemosaic, render},
    sharpen::DEFAULT_SHARPEN,
  },
  rawsource::RawSource,
  tags::{ExifTag, TiffCommonTag},
};

pub use super::render::ProcessingStep;

/// File format of a processed raw image
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProcessedImageFormat {
  /// 16-bit TIFF, linear 32-bit floating point for High Dynamic Range camera
  /// profiles, LZW compressed
  #[default]
  Tiff,
  /// 8-bit JPEG
  Jpeg,
  /// 16-bit PGM for one color plane, 16-bit PPM for three
  Pnm,
}

impl ProcessedImageFormat {
  /// Default file extension of the format
  pub fn extension(&self) -> &'static str {
    match self {
      Self::Tiff => "tif",
      Self::Jpeg => "jpg",
      Self::Pnm => "pnm",
    }
  }

  /// Format of a file extension, ignoring case
  pub fn from_extension(extension: &OsStr) -> Option<Self> {
    match extension.to_str()?.to_ascii_lowercase().as_str() {
      "tif" | "tiff" => Some(Self::Tiff),
      "jpg" | "jpeg" => Some(Self::Jpeg),
      "pgm" | "ppm" | "pnm" => Some(Self::Pnm),
      _ => None,
    }
  }
}

#[cfg(feature = "clap")]
impl clap::ValueEnum for ProcessedImageFormat {
  fn value_variants<'a>() -> &'a [Self] {
    &[Self::Tiff, Self::Jpeg, Self::Pnm]
  }

  fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
    Some(match self {
      Self::Tiff => clap::builder::PossibleValue::new("tif").alias("tiff"),
      Self::Jpeg => clap::builder::PossibleValue::new("jpeg").alias("jpg"),
      Self::Pnm => clap::builder::PossibleValue::new("pnm").aliases(["pgm", "ppm"]),
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
  /// DNG opcodes to insert after `remove_dng_opcodes`, each replaces the
  /// opcodes with its ID in the list of its stage
  pub replace_dng_opcodes: Vec<(OpcodeStage, Opcode)>,
  pub format: ProcessedImageFormat,
  /// JPEG quality (1-100)
  pub jpeg_quality: u8,
  /// Color space of the image
  pub color_space: OutputSpace,
  /// Demosaic algorithm for Bayer RGB data
  pub bayer_demosaic: BayerDemosaic,
  /// Demosaic algorithm for X-Trans data
  pub xtrans_demosaic: XTransDemosaic,
  /// Strength of the noise reduction
  pub denoise: DenoiseStrength,
  /// Strength of the sharpening
  pub sharpen: f32,
  /// Exposure in EV, added to the estimate of `auto_exposure`
  pub exposure: f32,
  /// Correct the exposure by an estimate from the image data
  pub auto_exposure: bool,
  /// Treatment of the values above white after a positive BaselineExposure
  pub highlights: HighlightMode,
  /// Crop to the largest rectangle without transparent pixels of the DNG
  /// transparency mask, within the crop of `crop`
  pub crop_transparent: bool,
  /// Processing steps of the rendering to apply, `None` for the default steps
  pub steps: Option<Vec<ProcessingStep>>,
  /// Processing steps of the rendering to leave out, before `steps`
  pub skip_steps: Vec<ProcessingStep>,
  /// Camera profile to render the image with, it replaces the primary camera profile
  pub dcp: Option<CameraProfileFile>,
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
      replace_dng_opcodes: Vec::new(),
      format: ProcessedImageFormat::Tiff,
      jpeg_quality: 94,
      color_space: OutputSpace::Srgb,
      bayer_demosaic: BayerDemosaic::default(),
      xtrans_demosaic: XTransDemosaic::default(),
      denoise: DenoiseStrength::default(),
      sharpen: DEFAULT_SHARPEN,
      exposure: 0.0,
      auto_exposure: false,
      highlights: HighlightMode::default(),
      crop_transparent: false,
      steps: None,
      skip_steps: Vec::new(),
      dcp: None,
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
  rawimage.replace_opcodes(&params.replace_dng_opcodes);
  if let Some(dcp) = &params.dcp {
    dcp.check_camera(&rawimage);
    rawimage.set_primary_camera_profile(dcp.profile.clone())?;
  }
  let metadata = decoder.raw_metadata(&rawfile, &raw_params)?;
  let mut develop = RawDevelop {
    color_space: params.color_space,
    bayer_demosaic: params.bayer_demosaic,
    xtrans_demosaic: params.xtrans_demosaic,
    denoise: params.denoise,
    sharpen: params.sharpen,
    exposure: params.exposure,
    auto_exposure: params.auto_exposure,
    highlights: params.highlights,
    ..RawDevelop::with_crop(params.crop)
  };
  if params.crop_transparent {
    crop_to_opaque_area(&mut rawimage, &mut develop, params.crop);
  }
  if let Some(steps) = &params.steps {
    develop.steps.retain(|step| steps.contains(step));
  }
  develop.steps.retain(|step| !params.skip_steps.contains(step));
  match params.format {
    ProcessedImageFormat::Tiff => develop.develop(rawimage, &metadata, image_file)?,
    ProcessedImageFormat::Jpeg => develop.develop_jpeg(rawimage, &metadata, image_file, params.jpeg_quality)?,
    ProcessedImageFormat::Pnm => develop.develop_pnm(rawimage, image_file)?,
  }
  Ok(())
}

/// Crop the development of `rawimage` to the largest rectangle of opaque pixels
/// of its transparency mask, within the area that `crop` keeps: the rectangle
/// replaces the DefaultCrop and `develop` crops to it. Without a transparency
/// mask or opaque pixel, nothing is changed.
fn crop_to_opaque_area(rawimage: &mut RawImage, develop: &mut RawDevelop, crop: CropMode) {
  let Some(mask) = &rawimage.transparency_mask else {
    log::debug!("Image has no transparency mask, it is not cropped to the opaque area");
    return;
  };
  let full = Rect::new(Point::zero(), Dim2::new(rawimage.width, rawimage.height));
  let within = match crop {
    CropMode::Best => rawimage.crop_area.or(rawimage.active_area),
    CropMode::ActiveArea => rawimage.active_area,
    CropMode::None => None,
  }
  .unwrap_or(full);
  match mask.largest_opaque_rect(within) {
    Some(area) => {
      log::debug!("Crop to the opaque area {:?} within {:?}", area, within);
      rawimage.crop_area = Some(area);
      if !develop.steps.contains(&ProcessingStep::CropDefault) {
        develop.steps.push(ProcessingStep::CropDefault);
      }
    }
    None => log::warn!(
      "The transparency mask has no opaque pixels within {:?}, the image is not cropped to them",
      within
    ),
  }
}

/// Dump raw pixel data as PGM
pub fn raw_as_pgm(width: usize, height: usize, buf: &[u16], writer: &mut dyn Write) -> std::io::Result<()> {
  let header = format!("P5 {} {} {}\n", width, height, 65535);
  writer.write_all(header.as_bytes())?;
  for px in buf {
    writer.write_u16::<BigEndian>(*px)?;
  }
  Ok(())
}

/// Dump raw pixel data as PPM
pub fn raw_as_ppm16(width: usize, height: usize, buf: &[u16], writer: &mut dyn Write) -> std::io::Result<()> {
  let header = format!("P6 {} {} {}\n", width, height, 65535);
  writer.write_all(header.as_bytes())?;
  for px in buf {
    writer.write_u16::<BigEndian>(*px)?;
  }
  Ok(())
}

/// Samples of `channels` per pixel with `alpha` as an extra sample, mapped by
/// `sample`. Without alpha, `data` is returned unchanged.
fn with_alpha<T: Copy>(data: Vec<T>, channels: usize, alpha: Option<&[f32]>, sample: impl Fn(f32) -> T) -> Vec<T> {
  match alpha {
    Some(alpha) => data
      .chunks_exact(channels)
      .zip(alpha)
      .flat_map(|(pixel, alpha)| pixel.iter().copied().chain(std::iter::once(sample(*alpha))))
      .collect(),
    None => data,
  }
}

/// Development of raw images with a set of processing steps
///
/// The order of the steps is defined by the rendering, see
/// [`plan`](crate::imgop::render::plan).
#[derive(Clone)]
pub struct RawDevelop {
  pub steps: Vec<ProcessingStep>,
  /// Color space of the developed image
  pub color_space: OutputSpace,
  /// Algorithm of the Demosaic step for Bayer RGB data
  pub bayer_demosaic: BayerDemosaic,
  /// Algorithm of the Demosaic step for X-Trans data
  pub xtrans_demosaic: XTransDemosaic,
  /// Strength of the Denoise step
  pub denoise: DenoiseStrength,
  /// Strength of the Sharpen step
  pub sharpen: f32,
  /// Exposure of the Exposure step in EV
  pub exposure: f32,
  /// Apply the AutoExposure step
  pub auto_exposure: bool,
  /// Treatment of the values above white after a positive BaselineExposure
  pub highlights: HighlightMode,
}

impl Default for RawDevelop {
  fn default() -> Self {
    Self::new_with(&ProcessingStep::ALL)
  }
}

impl RawDevelop {
  pub fn new_with(steps: &[ProcessingStep]) -> Self {
    Self {
      steps: Vec::from(steps),
      color_space: OutputSpace::default(),
      bayer_demosaic: BayerDemosaic::default(),
      xtrans_demosaic: XTransDemosaic::default(),
      denoise: DenoiseStrength::default(),
      sharpen: DEFAULT_SHARPEN,
      exposure: 0.0,
      auto_exposure: false,
      highlights: HighlightMode::default(),
    }
  }

  /// Development with all steps, the image is cropped by `crop`: to the
  /// DefaultCrop, the ActiveArea or not at all
  pub fn with_crop(crop: CropMode) -> Self {
    let excluded: &[ProcessingStep] = match crop {
      CropMode::Best => &[],
      CropMode::ActiveArea => &[ProcessingStep::CropDefault],
      CropMode::None => &[ProcessingStep::CropActiveArea, ProcessingStep::CropDefault],
    };
    let steps: Vec<ProcessingStep> = ProcessingStep::ALL.into_iter().filter(|step| !excluded.contains(step)).collect();
    Self::new_with(&steps)
  }

  /// The development without `step`
  pub fn without(mut self, step: ProcessingStep) -> Self {
    self.steps.retain(|other| *other != step);
    self
  }

  /// Options of the rendering with the steps of this development
  pub fn render_options(&self) -> RenderOptions {
    RenderOptions {
      steps: self.steps.iter().copied().collect(),
      color_space: self.color_space,
      bayer_demosaic: self.bayer_demosaic,
      xtrans_demosaic: self.xtrans_demosaic,
      denoise: self.denoise,
      sharpen: self.sharpen,
      exposure: self.exposure,
      auto_exposure: self.auto_exposure,
      highlights: self.highlights,
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
  ///
  /// The TIFF has 16-bit samples. A render with a High Dynamic Range camera
  /// profile has linear 32-bit floating point samples, they keep the values
  /// above 1.0.
  pub fn develop<W>(&self, rawimage: RawImage, md: &RawMetadata, writer: W) -> crate::Result<()>
  where
    W: io::Write + io::Seek,
  {
    let (make, model, orientation) = (rawimage.clean_make.clone(), rawimage.clean_model.clone(), rawimage.orientation);
    let options = RenderOptions {
      extended_range: true,
      ..self.render_options()
    };
    let mut image = render(rawimage, &options)?;

    let write_error = |err: TiffError| general_error!("Failed to write the TIFF file").context(err);
    let mut tiff = TiffWriter::new(writer).map_err(write_error)?;
    let mut root_ifd = Self::metadata_ifd(&mut tiff, &make, &model, orientation, md, &image)?;
    if let Some(icc) = image.icc_profile() {
      root_ifd.add_tag_undefined(ExifTag::IccProfile, icc);
    }

    let mut samples = image.channels;
    // The transparency mask is an extra sample of unassociated alpha, the
    // color values are not matted
    if image.alpha.is_some() {
      samples += 1;
      let color_samples = if image.channels == 1 { 1 } else { 3 };
      let extra: Vec<u16> = (color_samples..image.channels).map(|_| 0).chain(std::iter::once(2)).collect();
      root_ifd.add_tag(ExifTag::ExtraSamples, &extra);
    }
    let dim = image.dim();
    let (bits, (strip_rows, strips)) = if image.extended_range {
      let data = with_alpha(std::mem::take(&mut image.data), image.channels, image.alpha.as_deref(), |alpha| alpha);
      (32_u16, tiff.write_strips_lzw_f32(&data, samples, dim, 0).map_err(write_error)?)
    } else {
      let data = image.to_u16();
      let data = with_alpha(data, image.channels, image.alpha.as_deref(), |alpha| (alpha * u16::MAX as f32).round() as u16);
      (16_u16, tiff.write_strips_lzw(&data, samples, dim, 0).map_err(write_error)?)
    };
    let strip_offsets: Vec<u32> = strips.iter().map(|(offset, _)| *offset).collect();
    let strip_bytes: Vec<u32> = strips.iter().map(|(_, bytes)| *bytes).collect();
    root_ifd.add_tag(TiffCommonTag::Compression, 5);
    // Floating point samples have the floating point predictor
    root_ifd.add_tag(TiffCommonTag::Predictor, if image.extended_range { 3 } else { 1 });
    if image.extended_range {
      root_ifd.add_tag(TiffCommonTag::SampleFormat, &vec![3_u16; samples]);
    }
    root_ifd.add_tag(TiffCommonTag::StripOffsets, &strip_offsets);
    root_ifd.add_tag(TiffCommonTag::StripByteCounts, &strip_bytes);
    // Four channels are written as RGB with an extra channel
    root_ifd.add_tag(TiffCommonTag::BitsPerSample, &vec![bits; samples]);
    root_ifd.add_tag(TiffCommonTag::SamplesPerPixel, [samples as u16]);
    root_ifd.add_tag(TiffCommonTag::PhotometricInt, [if image.channels == 1 { 1_u16 } else { 2 }]);
    root_ifd.add_tag(TiffCommonTag::RowsPerStrip, strip_rows);
    root_ifd.add_tag(TiffCommonTag::ImageWidth, image.width as u16);
    root_ifd.add_tag(TiffCommonTag::ImageLength, image.height as u16);

    tiff.build(root_ifd).map_err(write_error)?;

    Ok(())
  }

  /// Develop raw image and write result as 8-bit JPEG with EXIF data
  pub fn develop_jpeg<W>(&self, rawimage: RawImage, md: &RawMetadata, writer: W, quality: u8) -> crate::Result<()>
  where
    W: io::Write,
  {
    let (make, model, orientation) = (rawimage.clean_make.clone(), rawimage.clean_model.clone(), rawimage.orientation);
    let mut image = self.develop_image(rawimage)?;
    if image.raw_values {
      return Err(input_error!("Raw values need 16 bits, they can't be written to an 8-bit JPEG"));
    }
    // JPEG has no alpha channel, transparent pixels are matted to black
    if let Some(alpha) = image.alpha.take() {
      let channels = image.channels;
      for (pixel, alpha) in image.data.chunks_exact_mut(channels).zip(alpha) {
        pixel.iter_mut().for_each(|v| *v *= alpha);
      }
    }

    // EXIF data is a TIFF structure without image data
    let mut exif = Cursor::new(Vec::new());
    let exif_error = |err: TiffError| general_error!("Failed to write the EXIF data").context(err);
    let mut tiff = TiffWriter::new(&mut exif).map_err(exif_error)?;
    let root_ifd = Self::metadata_ifd(&mut tiff, &make, &model, orientation, md, &image)?;
    tiff.build(root_ifd).map_err(exif_error)?;
    let exif = exif.into_inner();

    let icc = image.icc_profile();
    let image = image
      .to_dynamic_image()
      .ok_or_else(|| general_error!("Failed to convert the image for the JPEG encoder"))?;

    let mut encoder = JpegEncoder::new_with_quality(writer, quality);
    if let Some(icc) = icc {
      encoder
        .set_icc_profile(icc)
        .map_err(|err| general_error!("Failed to set JPEG ICC profile").context(err))?;
    }
    // The APP1 segment holds 65533 bytes, including the "Exif\0\0" header
    if exif.len() <= 65527 {
      encoder
        .set_exif_metadata(exif)
        .map_err(|err| general_error!("Failed to set JPEG EXIF data").context(err))?;
    } else {
      log::warn!("EXIF data of {} bytes doesn't fit into JPEG, it is not written", exif.len());
    }
    image
      .write_with_encoder(encoder)
      .map_err(|err| general_error!("Failed to write JPEG").context(err))?;
    Ok(())
  }

  /// Develop raw image and write the result as 16-bit PGM (one channel) or
  /// PPM (three channels), without metadata
  ///
  /// The transparency mask is dropped, PGM and PPM have no alpha channel.
  pub fn develop_pnm<W>(&self, rawimage: RawImage, mut writer: W) -> crate::Result<()>
  where
    W: io::Write,
  {
    let image = self.develop_image(rawimage)?;
    if image.alpha.is_some() {
      log::warn!("PGM and PPM have no alpha channel, the transparency mask is not written");
    }
    let data = image.to_u16();
    match image.channels {
      1 => raw_as_pgm(image.width, image.height, &data, &mut writer),
      3 => raw_as_ppm16(image.width, image.height, &data, &mut writer),
      channels => return Err(input_error!("PGM and PPM hold one or three channels, the image has {}", channels)),
    }
    .map_err(|err| general_error!("Failed to write the PGM/PPM file").context(err))
  }

  /// IFD 0 with the EXIF data of `image`, developed from a raw image of a
  /// camera with `make` and `model`
  fn metadata_ifd<W>(
    tiff: &mut TiffWriter<W>,
    make: &str,
    model: &str,
    orientation: Option<Orientation>,
    md: &RawMetadata,
    image: &RenderedImage,
  ) -> crate::Result<DirectoryWriter>
  where
    W: io::Write + io::Seek,
  {
    let mut root_ifd = DirectoryWriter::new();
    let mut exif_ifd = DirectoryWriter::new();

    // Add EXIF version 0220
    exif_ifd.add_tag_undefined(ExifTag::ExifVersion, vec![48, 50, 50, 48]);

    let exif_error = |err: TiffError| general_error!("Failed to write the EXIF data").context(err);
    md.write_exif_tags(tiff, &mut root_ifd, &mut exif_ifd).map_err(exif_error)?;
    root_ifd.add_tag(TiffCommonTag::Make, make);
    root_ifd.add_tag(TiffCommonTag::Model, model);
    // The color space of the image, not of the camera: sRGB or uncalibrated
    let srgb = image.color_space == Some(OutputSpace::Srgb) && image.transfer_function == TransferFunction::Srgb;
    exif_ifd.add_tag(ExifTag::ColorSpace, if srgb { 1_u16 } else { 0xffff });

    let exif_offset = exif_ifd.build(tiff).map_err(exif_error)?;

    root_ifd.add_tag(TiffCommonTag::ExifIFDPointer, exif_offset);
    if let Some(orientation) = orientation {
      root_ifd.add_tag(TiffCommonTag::Orientation, orientation.to_u16());
    }
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
    formats::tiff::SRational,
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
    let with_render = RawDevelop::default().without(ProcessingStep::ToneCurve).without(ProcessingStep::Denoise);
    let without_render = with_render.clone().without(ProcessingStep::ProfileRender).steps;
    let develop = |rawimage: &RawImage, steps: &[ProcessingStep]| -> crate::Result<[f32; 3]> {
      Ok(rgb_pixels(&RawDevelop::new_with(steps).develop_image(rawimage.clone())?)[0])
    };
    // Without tables, the step has no effect
    assert_eq!(develop(&rawimage, &with_render.steps)?, develop(&rawimage, &without_render)?);
    // The tone curve brightens the image
    rawimage.camera_profiles[0].tone_curve = Some(vec![0.0, 0.0, 0.5, 0.8, 1.0, 1.0]);
    let rendered = develop(&rawimage, &with_render.steps)?;
    let plain = develop(&rawimage, &without_render)?;
    assert!(rendered.iter().zip(plain).all(|(a, b)| *a > b), "{:?} {:?}", rendered, plain);
    Ok(())
  }

  /// Linear RGB ramp of 16x8 pixels with an identity color matrix, and its metadata
  fn ramp_rawimage() -> (RawImage, RawMetadata) {
    use crate::{camera_profile::CameraProfile, exif::Exif};

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
    (rawimage, RawMetadata::new(&camera, Exif::default()))
  }

  #[test]
  fn develop_jpeg_with_exif() -> std::result::Result<(), Box<dyn std::error::Error>> {
    use image::{ColorType, DynamicImage, ImageDecoder, codecs::jpeg::JpegDecoder};

    let (rawimage, metadata) = ramp_rawimage();
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
  fn develop_writes_orientation_of_raw_image() -> std::result::Result<(), Box<dyn std::error::Error>> {
    use crate::formats::tiff::{GenericTiffReader, reader::TiffReader};
    use image::{ImageDecoder, codecs::jpeg::JpegDecoder};

    let orientation_tag = |tiff: &[u8]| {
      let reader = GenericTiffReader::new_with_buffer(tiff, 0, 0, None).expect("TIFF");
      reader.root_ifd().get_entry(TiffCommonTag::Orientation).map(|entry| entry.force_u16(0))
    };
    for (orientation, tag) in [(Some(Orientation::Rotate90), Some(6)), (Some(Orientation::Normal), Some(1)), (None, None)] {
      let (mut rawimage, mut metadata) = ramp_rawimage();
      rawimage.orientation = orientation;
      // Only the raw image counts, not the reported EXIF data
      metadata.exif.orientation = Some(8);

      let mut tiff = Cursor::new(Vec::new());
      RawDevelop::default().develop(rawimage.clone(), &metadata, &mut tiff)?;
      assert_eq!(orientation_tag(tiff.get_ref()), tag);

      let mut jpeg = Vec::new();
      RawDevelop::default().develop_jpeg(rawimage, &metadata, &mut jpeg, 90)?;
      let exif = JpegDecoder::new(Cursor::new(&jpeg))?.exif_metadata()?.ok_or("no EXIF data")?;
      assert_eq!(orientation_tag(&exif), tag);
    }
    Ok(())
  }

  /// sRGB encoded `codes` with the table and the color matrix of an ICC profile
  /// render back to the same values
  #[test]
  fn develop_linearizes_output_referred_values() -> std::result::Result<(), Box<dyn std::error::Error>> {
    use crate::{
      camera_profile::CameraProfile,
      imgop::icc::{self, MatrixShaperProfile},
      rawimage::ColorimetricReference,
    };

    let profile = MatrixShaperProfile::parse(&icc::rgb_profile(OutputSpace::Srgb, TransferFunction::Srgb))?;
    for (bits, tolerance) in [(16, 0.5e-3), (8, 0.6 / 255.0)] {
      let max = (1_u32 << bits) - 1;
      let codes: Vec<u16> = (0..=max).flat_map(|code| [code as u16; 3]).collect();
      let width = codes.len() / 3;
      let mut rawimage = RawImage::new_with_data(
        Camera::default(),
        RawImageData::Integer(codes.clone()),
        width * 3,
        1,
        3,
        [1.0, 1.0, 1.0, f32::NAN],
        RawPhotometricInterpretation::LinearRaw,
        Some(BlackLevel::zero(1, 1, 3)),
        Some(WhiteLevel::new(vec![u16::MAX as u32; 3])),
        false,
      );
      rawimage.linearization_table = Some(profile.linearization_table(1 << bits)?);
      rawimage.camera_profiles = vec![CameraProfile::from_forward_matrix_d50(&profile.rgb_to_xyz, &[1.0; 3]).ok_or("no profile")?];
      rawimage.colorimetric_reference = ColorimetricReference::OutputReferred;

      let rendered = RawDevelop::default().develop_image(rawimage)?;
      let worst = codes
        .iter()
        .zip(&rendered.data)
        .map(|(code, value)| (f32::from(*code) / max as f32 - value).abs())
        .fold(0.0, f32::max);
      assert!(worst <= tolerance, "{} bits: largest difference is {}", bits, worst);
    }
    Ok(())
  }

  #[test]
  fn develop_tags_color_space() -> std::result::Result<(), Box<dyn std::error::Error>> {
    use crate::imgop::icc;
    use image::{ImageDecoder, codecs::jpeg::JpegDecoder};

    let contains = |data: &[u8], part: &[u8]| data.windows(part.len()).any(|window| window == part);
    // IFD entry of the EXIF ColorSpace (SHORT, one value) in the byte order of the TIFF structure
    let color_space_entry = |tiff: &[u8], value: u16| -> Vec<u8> {
      let little = &tiff[..2] == b"II";
      let bytes = |v: u16| if little { v.to_le_bytes() } else { v.to_be_bytes() };
      [
        bytes(0xa001),
        bytes(3),
        bytes(if little { 1 } else { 0 }),
        bytes(if little { 0 } else { 1 }),
        bytes(value),
        [0, 0],
      ]
      .concat()
    };
    for (space, exif_color_space) in [(OutputSpace::Srgb, 1), (OutputSpace::AdobeRgb, 0xffff), (OutputSpace::ProPhotoRgb, 0xffff)] {
      let develop = RawDevelop {
        color_space: space,
        ..Default::default()
      };
      let profile = icc::rgb_profile(space, space.transfer_function());
      let (rawimage, metadata) = ramp_rawimage();

      let mut jpeg = Vec::new();
      develop.develop_jpeg(rawimage.clone(), &metadata, &mut jpeg, 90)?;
      let mut decoder = JpegDecoder::new(Cursor::new(&jpeg))?;
      assert_eq!(decoder.icc_profile()?.as_ref(), Some(&profile), "{:?}", space);
      let exif = decoder.exif_metadata()?.ok_or("no EXIF data")?;
      assert!(contains(&exif, &color_space_entry(&exif, exif_color_space)), "{:?}", space);

      let mut tiff = Cursor::new(Vec::new());
      develop.develop(rawimage, &metadata, &mut tiff)?;
      let tiff = tiff.into_inner();
      assert!(contains(&tiff, &profile), "{:?}", space);
      assert!(contains(&tiff, &color_space_entry(&tiff, exif_color_space)), "{:?}", space);
    }
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
    rawimage.wb_coeffs = [1.0, 1.0, 1.0, f32::NAN];
    rawimage.baseline_exposure = Some(SRational::new(-1, 1));
    rawimage
  }

  #[test]
  fn develop_applies_negative_baseline_exposure() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let rawimage = negative_exposure_rawimage();
    // White stays white, dark values are darkened by 1 EV
    let develop = RawDevelop::default()
      .without(ProcessingStep::TransferFunction)
      .without(ProcessingStep::ToneCurve)
      .without(ProcessingStep::Denoise)
      .without(ProcessingStep::Sharpen);
    let image = develop.develop_image(rawimage.clone())?;
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

  /// Linear values of an image with the Exposure step as the only color operation
  fn develop_exposure(rawimage: RawImage, exposure: f32) -> crate::Result<RenderedImage> {
    let develop = RawDevelop {
      exposure,
      ..RawDevelop::new_with(&[ProcessingStep::Rescale, ProcessingStep::Demosaic, ProcessingStep::Exposure])
    };
    develop.develop_image(rawimage)
  }

  #[test]
  fn develop_applies_exposure() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Linear output of a neutral ramp is exactly scaled by a positive exposure
    let (rawimage, _) = ramp_rawimage();
    let before = develop_exposure(rawimage.clone(), 0.0)?;
    let brighter = develop_exposure(rawimage.clone(), 1.5)?;
    let factor = 1.5_f32.exp2();
    assert!(before.data.iter().zip(&brighter.data).all(|(a, b)| (a * factor - b).abs() < 1e-5));
    // By a negative exposure up to 0.25, values above are darkened less
    let darker = develop_exposure(rawimage, -1.2)?;
    let factor = (-1.2_f32).exp2();
    for (a, b) in before.data.iter().zip(&darker.data) {
      if *a <= 0.25 {
        assert!((a * factor - b).abs() < 1e-5, "{} {}", a, b);
      } else {
        assert!(a * factor < *b && b < a, "{} {}", a, b);
      }
    }
    Ok(())
  }

  #[test]
  fn develop_keeps_white_at_negative_exposure() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // White stays white in the linear values and through the color map
    let rawimage = {
      let mut rawimage = negative_exposure_rawimage();
      rawimage.baseline_exposure = None;
      rawimage
    };
    let image = develop_exposure(rawimage.clone(), -1.2)?;
    let pixels = rgb_pixels(&image);
    assert_eq!(pixels[0], [1.0; 3]);
    assert!(pixels[1].iter().all(|v| (v - 0.2 * (-1.2_f32).exp2()).abs() < 1e-5), "{:?}", pixels[1]);
    let develop = RawDevelop {
      exposure: -1.2,
      ..RawDevelop::default()
    };
    let image = develop.develop_image(rawimage)?;
    let pixels = rgb_pixels(&image);
    assert!(pixels[0].iter().all(|v| (v - 1.0).abs() < 1e-5), "{:?}", pixels[0]);
    assert!(pixels[1].iter().all(|v| *v < 0.9), "{:?}", pixels[1]);
    Ok(())
  }

  #[test]
  fn develop_has_no_exposure_by_default() {
    for develop in [RawDevelop::default(), RawDevelop::with_crop(CropMode::None)] {
      let options = develop.render_options();
      assert_eq!((options.exposure, options.auto_exposure), (0.0, false));
    }
    let params = RawProcessingParams::default();
    assert_eq!((params.exposure, params.auto_exposure), (0.0, false));
    // The options reach the rendering
    let develop = RawDevelop {
      exposure: 0.5,
      auto_exposure: true,
      ..Default::default()
    };
    let options = develop.render_options();
    assert_eq!((options.exposure, options.auto_exposure), (0.5, true));
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
    rawimage.baseline_exposure = Some(SRational::new(1, 2));
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

  /// Ramp of a saturated color `k * (1.0, 0.4, 0.2)` with a D65 profile and a
  /// BaselineExposure of +1 EV. No camera value is clipped, but red is above
  /// 1.0 after the color matrix for the brighter pixels.
  fn saturated_ramp_rawimage() -> (RawImage, Vec<f32>) {
    let ramp: Vec<f32> = (0..61).map(|i| 0.3 + i as f32 / 100.0).collect();
    let mut rawimage = d65_rawimage();
    rawimage.width = ramp.len();
    rawimage.data = RawImageData::Float(ramp.iter().flat_map(|k| [*k, 0.4 * k, 0.2 * k]).collect());
    rawimage.wb_coeffs = [1.0, 1.0, 1.0, f32::NAN];
    rawimage.baseline_exposure = Some(SRational::new(1, 1));
    (rawimage, ramp)
  }

  #[test]
  fn develop_keeps_linear_values_above_white() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Issue #884: values above 1.0 were mapped to a less saturated color, with
    // a jump at 1.0
    let (rawimage, ramp) = saturated_ramp_rawimage();
    let develop = RawDevelop::default()
      .without(ProcessingStep::TransferFunction)
      .without(ProcessingStep::ToneCurve)
      .without(ProcessingStep::Denoise)
      .without(ProcessingStep::Sharpen);
    let image = develop.develop_image(rawimage)?;
    let pixels = rgb_pixels(&image);
    assert!(pixels[0][0] < 1.0 && pixels[pixels.len() - 1][0] > 1.0, "{:?}", pixels);
    // All values, also those above 1.0 or negative, are proportional to the camera values
    let color = pixels[0].map(|v| v / ramp[0]);
    for (pixel, k) in pixels.iter().zip(&ramp) {
      for (v, c) in pixel.iter().zip(color) {
        assert!((v - c * k).abs() < 1e-5, "{:?} at {}", pixel, k);
      }
    }
    Ok(())
  }

  #[test]
  fn develop_clips_encoded_values_per_channel() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let (rawimage, _) = saturated_ramp_rawimage();
    let steps: Vec<ProcessingStep> = RawDevelop::default()
      .steps
      .into_iter()
      .filter(|step| *step != ProcessingStep::TransferFunction)
      .collect();
    let linear = RawDevelop::new_with(&steps).develop_image(rawimage.clone())?;
    let encoded = RawDevelop::default().develop_image(rawimage)?;
    // Each channel is clipped to 0..1 on its own, so the ramp stays continuous
    for (linear, encoded) in rgb_pixels(&linear).iter().zip(rgb_pixels(&encoded)) {
      let expected = linear.map(|v| TransferFunction::Srgb.encode(v.clamp(0.0, 1.0)));
      assert!(expected.iter().zip(encoded).all(|(a, b)| (a - b).abs() < 1e-5), "{:?} {:?}", linear, encoded);
    }
    Ok(())
  }

  #[test]
  fn develop_renders_clipped_highlights_white() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // White balance of a camera with a coefficient below 1.0
    let wb = [1.0, 0.76, 1.05, f32::NAN];
    let is_white = |image: &RenderedImage| rgb_pixels(image)[0].iter().all(|v| (v - 1.0).abs() < 1e-5);
    // Clipped mosaic data, white balanced before the demosaic
    let mut camera = Camera::new();
    camera.cfa = CFA::new("RGGB");
    let photometric = RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&camera));
    let mut mosaic = RawImage::new(
      camera,
      PixU16::new_with(vec![4095; 16 * 16], 16, 16),
      1,
      wb,
      photometric,
      Some(BlackLevel::new(&[0_u32, 0, 0, 0], 2, 2, 1)),
      Some(WhiteLevel::new_bits(12, 1)),
      false,
    );
    mosaic.camera_profiles = d65_rawimage().camera_profiles;
    let develop = RawDevelop::default().without(ProcessingStep::Denoise);
    let image = develop.develop_image(mosaic)?;
    assert!(is_white(&image), "{:?}", rgb_pixels(&image)[0]);
    // A clipped and a gray pixel of LinearRaw data, white balanced by the color map
    let mut rawimage = d65_rawimage();
    rawimage.width = 2;
    rawimage.data = RawImageData::Float(vec![1.0, 1.0, 1.0, 0.2, 0.15, 0.21]);
    rawimage.wb_coeffs = wb;
    let image = develop.develop_image(rawimage.clone())?;
    assert!(is_white(&image), "{:?}", rgb_pixels(&image)[0]);
    // The scale of the coefficients has no effect
    rawimage.wb_coeffs = wb.map(|c| c * 1.7);
    let scaled = develop.develop_image(rawimage)?.data;
    assert!(image.data.iter().zip(&scaled).all(|(a, b)| (a - b).abs() < 1e-5), "{:?}", scaled);
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
      formats::tiff::{GenericTiffReader, reader::TiffReader},
      imgop::render::{DemosaicQuality, StepSet, WhiteBalancePlacement},
      mask::MaskImage,
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
      rawimage.baseline_exposure = Some(SRational::new(1, 2));
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
      // The cases pin the colors without the default tone curve, the noise
      // reduction and the sharpening, except for the cases of these steps
      let steps = StepSet::all().without(ToneCurve).without(Denoise).without(Sharpen);
      let with_tone_curve = RenderOptions {
        steps: steps.with(ToneCurve),
        ..Default::default()
      };
      let with_denoise = RenderOptions {
        steps: steps.with(Denoise),
        ..Default::default()
      };
      let with_sharpen = RenderOptions {
        steps: steps.with(Sharpen),
        ..Default::default()
      };
      let with_exposure = |exposure| RenderOptions {
        steps,
        exposure,
        ..Default::default()
      };
      let with_auto_exposure = RenderOptions {
        steps,
        auto_exposure: true,
        ..Default::default()
      };
      let all = RenderOptions { steps, ..Default::default() };
      let without = |step: ProcessingStep| RenderOptions {
        steps: steps.without(step),
        ..Default::default()
      };
      let wb_after_demosaic = RenderOptions {
        steps,
        white_balance: WhiteBalancePlacement::AfterDemosaic,
        ..Default::default()
      };
      let camera = RenderOptions {
        steps: [Rescale, Demosaic, CropActiveArea, CropDefault].into_iter().collect(),
        ..Default::default()
      };
      let preview = RenderOptions {
        steps,
        demosaic: DemosaicQuality::Preview,
        ..Default::default()
      };
      let adobe_rgb = RenderOptions {
        steps,
        color_space: OutputSpace::AdobeRgb,
        ..Default::default()
      };
      let prophoto = RenderOptions {
        steps,
        color_space: OutputSpace::ProPhotoRgb,
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
      rggb_negative_exposure.baseline_exposure = Some(SRational::new(-1, 1));
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
        ("rggb_adobe_rgb", rggb(), adobe_rgb),
        ("linear_raw_prophoto", linear_raw(), prophoto),
        ("rggb_tone_curve", rggb(), with_tone_curve.clone()),
        ("mono_tone_curve", mono(), with_tone_curve),
        ("rggb_denoise", rggb(), with_denoise.clone()),
        ("mono_denoise", mono(), with_denoise),
        ("rggb_sharpen", rggb(), with_sharpen.clone()),
        ("mono_sharpen", mono(), with_sharpen),
        ("rggb_exposure_up", rggb(), with_exposure(1.0)),
        ("rggb_exposure_down", rggb(), with_exposure(-1.2)),
        ("mono_exposure_down", mono(), with_exposure(-1.2)),
        ("rggb_auto_exposure", rggb(), with_auto_exposure),
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
      ("rggb_linear", 24, 16, 3, &[485.7438862025738, 23320.44913226366, 631.5755190633486, 134.4012443870306, 6599.922600671649, 60.6319791428197, 336.55111595988274, 16570.51391789317, 312.51550925905866]),
      ("rggb_full_frame", 24, 16, 3, &[381.69512236118317, 18150.39451789856, 379.595433615728, 232.32091468572617, 11240.07896360755, 147.64509285845165, 355.1079213619232, 17057.55524724722, 331.2803854717914]),
      ("rggb_profile", 24, 16, 3, &[383.2990419268608, 18208.48621380329, 382.7840235605359, 298.3092967271805, 14479.034941881895, 241.79218848726833, 362.78131806850433, 17361.05045902729, 343.8535267408927]),
      ("rggb_negative_exposure", 24, 16, 3, &[372.9707576036453, 17801.475763201714, 364.28282762169994, 148.17328909039497, 7266.1948882341385, 69.80508398590429, 337.22546261548996, 16365.803864657879, 303.55807226827676]),
      ("cygm", 24, 16, 3, &[333.887368619442, 16094.266443014145, 297.7676277348556, 244.9334465265274, 11778.500518351793, 161.74481508232535, 363.30793583393097, 17384.043117403984, 345.91098645419413]),
      ("cygm_preview", 12, 8, 3, &[89.73201927542686, 4332.205256462097, 85.09224837936573, 54.80821684002876, 2643.0370524823666, 33.73017430069746, 90.4724417924881, 4360.6897903084755, 85.8589423764323]),
      ("xtrans", 28, 16, 3, &[441.8567228913307, 20118.23803973198, 436.9002191243954, 270.7740049660206, 12374.535669326782, 170.7231797951337, 409.30331003665924, 18666.734287142754, 378.8607051136843]),
      ("mono", 24, 16, 1, &[286.27965730428696, 13782.818696141243, 217.83812832726255]),
      ("linear_raw_profile", 16, 12, 3, &[185.16603311896324, 8949.49612993002, 180.88419019787497, 145.02567222714424, 7189.386500045657, 117.71279589253793, 187.39759808778763, 8956.11008054018, 183.31439043260133]),
      ("rggb_adobe_rgb", 24, 16, 3, &[371.56912833452225, 17734.80420601368, 360.9035039896841, 230.95152899622917, 11181.354400068521, 145.65351766270155, 349.8610346317291, 16862.70594650507, 322.1781369714192]),
      ("linear_raw_prophoto", 16, 12, 3, &[177.1249699294567, 8622.046199917793, 166.81404954801548, 146.0337336063385, 7240.736013621092, 118.7245470189572, 181.93863874673843, 8731.905935764313, 173.29570451430013]),
      ("rggb_tone_curve", 24, 16, 3, &[382.7776269316673, 18193.989409446716, 381.7496863167044, 259.79086369276047, 12595.321324795485, 183.11937804395097, 360.4882082939148, 17300.99929469824, 340.23592304348347]),
      ("mono_tone_curve", 24, 16, 1, &[306.6658738851547, 14753.879509568214, 248.869998376688]),
      ("rggb_denoise", 24, 16, 3, &[381.8545647263527, 18156.606020390987, 379.93434239532274, 235.6433406174183, 11340.5538893044, 149.3080918155735, 355.65923941135406, 17085.819627046585, 332.07478443911083]),
      ("mono_denoise", 24, 16, 1, &[286.96862787008286, 13775.304181218147, 218.071894642691]),
      ("rggb_sharpen", 24, 16, 3, &[380.3560812473297, 18098.973595499992, 377.30827813224204, 232.59430253505707, 11267.810094922781, 147.7265395562739, 353.0541972517967, 17005.989935040474, 327.8119022570862]),
      ("mono_sharpen", 24, 16, 1, &[286.0752182006836, 13778.213288664818, 217.66035739673464]),
      ("rggb_exposure_up", 24, 16, 3, &[383.96843552589417, 18243.841208696365, 383.937458310667, 350.1493704319, 16878.810312092304, 325.87266324883933, 383.643371284008, 18234.11838966608, 383.3066028926243]),
      ("rggb_exposure_down", 24, 16, 3, &[354.61059987545013, 17034.09955960512, 332.3316358791035, 201.39680242538452, 9835.41731902957, 115.59887993295101, 309.8609471321106, 15108.08115285635, 258.89411118142544]),
      ("mono_exposure_down", 24, 16, 1, &[228.4502983391285, 11078.504926145077, 144.1481925731507]),
      ("rggb_auto_exposure", 24, 16, 3, &[320.57339230179787, 15592.564796447754, 280.72790856261537, 179.13224990665913, 8831.046201482415, 97.00345894004089, 274.5888995230198, 13529.250810801983, 211.50433309477341]),
    ];

    /// Largest absolute difference of the samples of two images of the same size
    fn max_difference(a: &RenderedImage, b: &RenderedImage) -> f32 {
      assert_eq!((a.dim(), a.channels), (b.dim(), b.channels));
      a.data.iter().zip(&b.data).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max)
    }

    fn render_with(rawimage: &RawImage, white_balance: WhiteBalancePlacement, demosaic: DemosaicQuality) -> RenderedImage {
      let options = RenderOptions {
        steps: StepSet::all().without(ProcessingStep::Denoise).without(ProcessingStep::Sharpen),
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
      // PPG and VNG interpolate by the differences of the planes, Markesteijn
      // chooses the direction of the interpolation by the colors.
      let cygm_matrix = [[0.2, 0.6, 0.5], [0.7, 0.8, 0.0], [0.1, 0.9, 0.1], [0.8, 0.2, 0.6]];
      let rgb = [2.0, 1.0, 1.5, f32::NAN];
      for (pattern, colors, wb, matrix, demosaic, bilinear) in [
        ("RGGB", "RGB", rgb, transform_2d(&COLOR_MATRIX), DemosaicQuality::Quality, false),
        (XTRANS, "RGB", rgb, transform_2d(&COLOR_MATRIX), DemosaicQuality::Quality, false),
        (XTRANS, "RGB", rgb, transform_2d(&COLOR_MATRIX), DemosaicQuality::Fast, true),
        (
          "CYGM",
          "CYGM",
          [1.5, 1.2, 1.0, 1.4],
          transform_2d(&cygm_matrix),
          DemosaicQuality::Quality,
          false,
        ),
        ("CYGM", "CYGM", [1.5, 1.2, 1.0, 1.4], transform_2d(&cygm_matrix), DemosaicQuality::Fast, true),
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
          // The noise is estimated from the image data that is left after the crop,
          // and the sharpening has no neighbors outside of it
          let steps = StepSet::all().without(ProcessingStep::Denoise).without(ProcessingStep::Sharpen);
          let options = RenderOptions {
            steps,
            demosaic,
            ..Default::default()
          };
          let cropped = render(rawimage.clone(), &options).expect("render");
          let full = RenderOptions {
            steps: steps.without(ProcessingStep::CropDefault),
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
      // The noise is estimated from the image data that is left after the crop,
      // and the sharpening has no neighbors outside of it
      let with_crop = |crop| RawDevelop::with_crop(crop).without(ProcessingStep::Denoise).without(ProcessingStep::Sharpen);
      let active_area = with_crop(CropMode::ActiveArea).develop_image(linear_raw.clone()).expect("develop");
      let mut expected = with_crop(CropMode::None).develop_image(linear_raw).expect("develop");
      assert_eq!((expected.width, expected.height), (16, 12));
      expected.crop(Rect::new(Point::new(1, 1), Dim2::new(14, 10))).expect("crop");
      assert!(active_area.dim() == expected.dim() && active_area.data == expected.data);
    }

    /// Transparency mask of `rawimage` with the position y * width + x as value
    fn position_mask(rawimage: &RawImage) -> MaskImage {
      let (width, height) = (rawimage.width, rawimage.height);
      MaskImage::new(width, height, 16, RawImageData::Integer((0..(width * height) as u16).collect()))
    }

    /// Alpha of the positions of `area` in a mask of `width` pixels, see [`position_mask`]
    fn position_alpha(width: usize, area: Rect) -> Vec<f32> {
      (area.p.y..area.p.y + area.d.h)
        .flat_map(|y| (area.p.x..area.p.x + area.d.w).map(move |x| (y * width + x) as f32 / 65535.0))
        .collect()
    }

    #[test]
    fn transparency_mask_is_cropped_with_the_image() {
      let mut linear_raw = linear_raw();
      linear_raw.active_area = Some(Rect::new(Point::new(1, 1), Dim2::new(14, 10)));
      linear_raw.crop_area = Some(Rect::new(Point::new(2, 2), Dim2::new(12, 8)));
      for (name, mut rawimage) in [("rggb", rggb()), ("mono", mono()), ("linear_raw", linear_raw)] {
        rawimage.transparency_mask = Some(position_mask(&rawimage));
        let full = Rect::new(Point::zero(), Dim2::new(rawimage.width, rawimage.height));
        let crop_area = rawimage.crop_area.expect("DefaultCrop");
        for (crop, area) in [
          (CropMode::Best, crop_area),
          (CropMode::ActiveArea, rawimage.active_area.unwrap_or(full)),
          (CropMode::None, full),
        ] {
          let image = RawDevelop::with_crop(crop).develop_image(rawimage.clone()).expect("develop");
          assert_eq!(image.alpha, Some(position_alpha(rawimage.width, area)), "{} {:?}", name, crop);
        }
      }
    }

    #[test]
    fn transparency_mask_of_preview_has_half_size() {
      let mut rawimage = rggb();
      rawimage.transparency_mask = Some(position_mask(&rawimage));
      let crop = rawimage.crop_area.expect("DefaultCrop");
      let options = RenderOptions {
        demosaic: DemosaicQuality::Preview,
        ..Default::default()
      };
      let image = render(rawimage.clone(), &options).expect("render");
      assert_eq!(image.dim(), Dim2::new(crop.d.w / 2, crop.d.h / 2));
      // Each pixel is the rounded mean of a 2x2 block of the DefaultCrop
      let width = rawimage.width;
      let expected: Vec<f32> = (0..image.height)
        .flat_map(|y| {
          (0..image.width).map(move |x| {
            let (bx, by) = (crop.p.x + 2 * x, crop.p.y + 2 * y);
            let sum: usize = [(0, 0), (1, 0), (0, 1), (1, 1)].iter().map(|(dx, dy)| (by + dy) * width + bx + dx).sum();
            ((sum + 2) / 4) as f32 / 65535.0
          })
        })
        .collect();
      assert_eq!(image.alpha, Some(expected));
    }

    /// Mask of `rawimage` with an opaque rectangle `opaque`, other pixels are transparent
    fn rect_mask(rawimage: &RawImage, opaque: Rect) -> MaskImage {
      let samples = (0..rawimage.height)
        .flat_map(|y| {
          (0..rawimage.width).map(move |x| {
            if opaque.intersect(&Rect::new(Point::new(x, y), Dim2::new(1, 1))).is_some() {
              255
            } else {
              0
            }
          })
        })
        .collect();
      MaskImage::new(rawimage.width, rawimage.height, 8, RawImageData::Integer(samples))
    }

    #[test]
    fn crop_to_the_opaque_area() {
      let mut rawimage = rggb();
      rawimage.transparency_mask = Some(rect_mask(&rawimage, Rect::new(Point::new(6, 2), Dim2::new(20, 18))));
      // Within the DefaultCrop (4, 4, 24, 16), or the whole image
      for (crop, expected) in [
        (CropMode::Best, Rect::new(Point::new(6, 4), Dim2::new(20, 16))),
        (CropMode::None, Rect::new(Point::new(6, 2), Dim2::new(20, 18))),
      ] {
        let mut rawimage = rawimage.clone();
        let mut develop = RawDevelop::with_crop(crop);
        crop_to_opaque_area(&mut rawimage, &mut develop, crop);
        assert_eq!(rawimage.crop_area, Some(expected), "{:?}", crop);
        let image = develop.develop_image(rawimage).expect("develop");
        assert_eq!(image.dim(), expected.d, "{:?}", crop);
        assert!(image.alpha.is_some_and(|alpha| alpha.iter().all(|a| *a == 1.0)), "{:?}", crop);
      }
      // Without opaque pixels, the image isn't cropped
      let mut rawimage = rggb();
      rawimage.transparency_mask = Some(rect_mask(&rawimage, Rect::new(Point::zero(), Dim2::new(0, 0))));
      let mut develop = RawDevelop::with_crop(CropMode::None);
      crop_to_opaque_area(&mut rawimage, &mut develop, CropMode::None);
      assert_eq!(develop.develop_image(rawimage).expect("develop").dim(), Dim2::new(32, 24));
    }

    #[test]
    fn transparency_mask_in_output_files() {
      // The left half is transparent
      let mut rawimage = rggb();
      rawimage.transparency_mask = Some(rect_mask(&rawimage, Rect::new(Point::new(16, 0), Dim2::new(16, 24))));
      let develop = RawDevelop::with_crop(CropMode::None);
      let metadata = RawMetadata::default();

      // TIFF has an extra sample of unassociated alpha
      let mut tiff = Cursor::new(Vec::new());
      develop.develop(rawimage.clone(), &metadata, &mut tiff).expect("develop TIFF");
      let tiff = tiff.into_inner();
      let reader = GenericTiffReader::new_with_buffer(&tiff, 0, 0, None).expect("TIFF");
      let root = reader.root_ifd();
      assert_eq!(root.get_entry(TiffCommonTag::SamplesPerPixel).map(|entry| entry.force_u16(0)), Some(4));
      assert_eq!(root.get_entry(ExifTag::ExtraSamples).map(|entry| entry.force_u16(0)), Some(2));

      // JPEG is matted to black
      let mut jpeg = Vec::new();
      develop.develop_jpeg(rawimage, &metadata, &mut jpeg, 95).expect("develop JPEG");
      let jpeg = image::load_from_memory(&jpeg).expect("JPEG").into_rgb8();
      assert!(jpeg.get_pixel(3, 12).0.iter().all(|v| *v <= 2), "{:?}", jpeg.get_pixel(3, 12));
      assert!(jpeg.get_pixel(28, 12).0.iter().any(|v| *v > 20), "{:?}", jpeg.get_pixel(28, 12));
    }

    #[test]
    fn hdr_profile_in_float_tiff() {
      use crate::camera_profile::ProfileDynamicRange;
      use crate::formats::tiff::writer::float_predictor_decode;

      /// BitsPerSample, SampleFormat and Predictor of a TIFF and its decoded
      /// floating point samples
      fn read_tiff(tiff: &[u8]) -> (u16, u16, u16, Vec<f32>) {
        let reader = GenericTiffReader::new_with_buffer(tiff, 0, 0, None).expect("TIFF");
        let root = reader.root_ifd();
        let tag = |tag: TiffCommonTag| root.get_entry(tag).map_or(0, |entry| entry.force_u16(0));
        let (bits, format, predictor) = (
          tag(TiffCommonTag::BitsPerSample),
          tag(TiffCommonTag::SampleFormat),
          tag(TiffCommonTag::Predictor),
        );
        if format != 3 {
          return (bits, format, predictor, Vec::new());
        }
        let row_bytes = tag(TiffCommonTag::ImageWidth) as usize * tag(TiffCommonTag::SamplesPerPixel) as usize * 4;
        let (offsets, counts) = (
          root.get_entry(TiffCommonTag::StripOffsets).expect("offsets"),
          root.get_entry(TiffCommonTag::StripByteCounts).expect("counts"),
        );
        let mut values = Vec::new();
        for strip in 0..offsets.count() as usize {
          let start = offsets.force_u32(strip) as usize;
          let data = &tiff[start..start + counts.force_u32(strip) as usize];
          let decoded = weezl::decode::Decoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8)
            .decode(data)
            .expect("LZW");
          for row in decoded.chunks(row_bytes) {
            values.extend(float_predictor_decode(row.to_vec(), tag(TiffCommonTag::SamplesPerPixel) as usize));
          }
        }
        (bits, format, predictor, values)
      }

      // +2 EV brighten the highlights above white
      let mut rawimage = rggb();
      rawimage.baseline_exposure = Some(SRational::new(2, 1));
      let mut hdr = rawimage.clone();
      hdr.camera_profiles[0].dynamic_range = Some(ProfileDynamicRange {
        version: 1,
        dynamic_range: 1,
        hint_max_output_value: 0.0,
      });
      let develop = RawDevelop::with_crop(CropMode::None);
      let metadata = RawMetadata::default();
      let tiff_of = |rawimage: &RawImage| {
        let mut tiff = Cursor::new(Vec::new());
        develop.develop(rawimage.clone(), &metadata, &mut tiff).expect("develop TIFF");
        read_tiff(&tiff.into_inner())
      };

      // The HDR profile gives linear floating point samples above 1.0
      let (bits, format, predictor, values) = tiff_of(&hdr);
      assert_eq!((bits, format, predictor), (32, 3, 3));
      assert_eq!(values.len(), 32 * 24 * 3);
      assert!(values.iter().any(|v| *v > 1.5), "{:?}", values.iter().copied().fold(0.0, f32::max));
      // The SDR profile gives 16-bit samples
      assert_eq!(tiff_of(&rawimage).0, 16);
      // JPEG of the HDR profile is clipped
      let mut jpeg = Vec::new();
      develop.develop_jpeg(hdr, &metadata, &mut jpeg, 95).expect("develop JPEG");
      assert!(image::load_from_memory(&jpeg).is_ok());
    }

    #[test]
    fn preview_of_flat_field() {
      // Each pixel of the preview is a block of 2x2 pixels, a flat field keeps its color
      let mut rawimage = rggb();
      rawimage.data = RawImageData::Integer(vec![1500; 32 * 24]);
      let steps = StepSet::all().without(ProcessingStep::Denoise).without(ProcessingStep::Sharpen);
      let full = render(rawimage.clone(), &RenderOptions { steps, ..Default::default() }).expect("render");
      let options = RenderOptions {
        steps,
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
