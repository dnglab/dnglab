use std::{
  ffi::OsStr,
  io::{Cursor, Seek, Write},
  path::Path,
  sync::Arc,
  thread::JoinHandle,
  time::SystemTime,
};

use image::DynamicImage;

use crate::{
  RawImage, RawImageData, RawlerError,
  decoders::{Decoder, RawDecodeParams, RawPhotometricInterpretation, WellKnownIFD, WhiteLevel, validate_image_index},
  dng::{
    DEFAULT_LOSSY_JPEG_QUALITY, PREVIEW_JPEG_QUALITY,
    opcodes::{OpcodeSelector, OpcodeStage},
    original::OriginalCompressed,
    writer::DngWriter,
  },
  formats::tiff::Entry,
  imgop::{
    Point, Rect,
    develop::RawDevelop,
    fuji_rotate::fuji_normalize_rotation,
    matrix::{multiply, transform_1d, transform_2d},
    raw::map_4ch_to_3ch,
    sensor::{
      Demosaic, SensorType,
      bayer::{bilinear::Bilinear4Channel, ppg::PPGDemosaic},
      xtrans::markesteijn::XTransMarkesteijnDemosaic,
    },
  },
  pixarray::{Color2D, PixF32},
  rawimage::{BlackLevel, d65_preference},
  rawsource::RawSource,
  tags::{DngTag, ExifTag, TiffCommonTag},
};

use super::{ApplyDngOpcodes, CropMode, DngCompression, DngPhotometricConversion, DngVersion};

/// Opcode list stages in processing order
const STAGES: [OpcodeStage; 3] = [OpcodeStage::List1, OpcodeStage::List2, OpcodeStage::List3];

/// Tags with values for each color plane, copied from DNG input.
/// They don't match the image data after the color planes are reduced.
const COLOR_PLANE_TAGS: [DngTag; 9] = [
  DngTag::CameraCalibration1,
  DngTag::CameraCalibration2,
  DngTag::CameraCalibration3,
  DngTag::ForwardMatrix1,
  DngTag::ForwardMatrix2,
  DngTag::ForwardMatrix3,
  DngTag::AnalogBalance,
  DngTag::AsShotPreProfileMatrix,
  DngTag::CurrentPreProfileMatrix,
];

/// Parameters for DNG conversion
#[derive(Clone, Debug)]
pub struct ConvertParams {
  pub embedded: bool,
  pub compression: DngCompression,
  pub photometric_conversion: DngPhotometricConversion,
  pub apply_scaling: bool,
  pub crop: CropMode,
  pub predictor: u8,
  /// JPEG quality (1-100) for lossy compression
  pub lossy_jpeg_quality: u8,
  pub preview: bool,
  pub thumbnail: bool,
  /// Develop the preview from raw data instead of using an embedded preview
  pub force_preview: bool,
  /// Develop the thumbnail from raw data instead of using an embedded preview
  pub force_thumbnail: bool,
  pub artist: Option<String>,
  pub copyright: Option<String>,
  pub software: String,
  pub index: usize,
  pub keep_mtime: bool,
  /// Value for DNGVersion tag
  pub dng_version: DngVersion,
  /// Highest allowed value for DNGBackwardVersion tag
  pub max_dng_backward_version: DngVersion,
  /// DNG opcode lists to apply to the image data, in addition to the lists
  /// that can't be stored with `max_dng_backward_version`
  pub apply_dng_opcodes: ApplyDngOpcodes,
  /// DNG opcodes to remove, they are neither applied nor written to the DNG
  pub remove_dng_opcodes: Vec<OpcodeSelector>,
}

/// Information surfaced from a completed conversion.
///
/// Lets callers reuse work the converter already performed (such as the
/// metadata pass) instead of re-running the decoder.
#[derive(Clone, Debug, Default)]
pub struct ConvertInfo {
  /// Embedded "last modified" timestamp recovered from the input's metadata,
  /// if the decoder was able to find one.
  pub last_modified: Option<SystemTime>,
}

impl Default for ConvertParams {
  fn default() -> Self {
    Self {
      embedded: true,
      compression: DngCompression::Lossless,
      photometric_conversion: DngPhotometricConversion::Original,
      apply_scaling: false,
      crop: CropMode::Best,
      predictor: 1,
      lossy_jpeg_quality: DEFAULT_LOSSY_JPEG_QUALITY,
      preview: true,
      thumbnail: true,
      force_preview: false,
      force_thumbnail: false,
      artist: None,
      copyright: None,
      software: "DNGLab".into(),
      index: 0,
      keep_mtime: false,
      dng_version: DngVersion::V1_6,
      max_dng_backward_version: DngVersion::V1_4,
      apply_dng_opcodes: ApplyDngOpcodes::Auto,
      remove_dng_opcodes: Vec::new(),
    }
  }
}

impl ConvertParams {
  /// Set whether the original raw file is embedded into the DNG.
  pub fn with_embedded(mut self, embedded: bool) -> Self {
    self.embedded = embedded;
    self
  }

  /// Set the DNG compression method.
  pub fn with_compression(mut self, compression: DngCompression) -> Self {
    self.compression = compression;
    self
  }

  /// Set the photometric conversion mode.
  pub fn with_photometric_conversion(mut self, photometric_conversion: DngPhotometricConversion) -> Self {
    self.photometric_conversion = photometric_conversion;
    self
  }

  /// Set whether black/white level scaling is applied.
  pub fn with_apply_scaling(mut self, apply_scaling: bool) -> Self {
    self.apply_scaling = apply_scaling;
    self
  }

  /// Set the crop mode.
  pub fn with_crop(mut self, crop: CropMode) -> Self {
    self.crop = crop;
    self
  }

  /// Set the compression predictor.
  pub fn with_predictor(mut self, predictor: u8) -> Self {
    self.predictor = predictor;
    self
  }

  /// Set the JPEG quality (1-100) for lossy compression.
  pub fn with_lossy_jpeg_quality(mut self, lossy_jpeg_quality: u8) -> Self {
    self.lossy_jpeg_quality = lossy_jpeg_quality;
    self
  }

  /// Set whether a preview image is generated.
  pub fn with_preview(mut self, preview: bool) -> Self {
    self.preview = preview;
    self
  }

  /// Set whether a thumbnail image is generated.
  pub fn with_thumbnail(mut self, thumbnail: bool) -> Self {
    self.thumbnail = thumbnail;
    self
  }

  /// Set whether the preview image is developed from raw data.
  pub fn with_force_preview(mut self, force_preview: bool) -> Self {
    self.force_preview = force_preview;
    self
  }

  /// Set whether the thumbnail image is developed from raw data.
  pub fn with_force_thumbnail(mut self, force_thumbnail: bool) -> Self {
    self.force_thumbnail = force_thumbnail;
    self
  }

  /// Set the artist metadata field.
  pub fn with_artist(mut self, artist: Option<String>) -> Self {
    self.artist = artist;
    self
  }

  /// Set the copyright metadata field.
  pub fn with_copyright(mut self, copyright: Option<String>) -> Self {
    self.copyright = copyright;
    self
  }

  /// Set the software metadata field.
  pub fn with_software(mut self, software: String) -> Self {
    self.software = software;
    self
  }

  /// Set the image index to convert.
  pub fn with_index(mut self, index: usize) -> Self {
    self.index = index;
    self
  }

  /// Set whether the source modification time is preserved.
  pub fn with_keep_mtime(mut self, keep_mtime: bool) -> Self {
    self.keep_mtime = keep_mtime;
    self
  }

  /// Set the DNGVersion.
  pub fn with_dng_version(mut self, dng_version: DngVersion) -> Self {
    self.dng_version = dng_version;
    self
  }

  /// Set the highest allowed DNGBackwardVersion.
  pub fn with_max_dng_backward_version(mut self, max_dng_backward_version: DngVersion) -> Self {
    self.max_dng_backward_version = max_dng_backward_version;
    self
  }

  /// Set the DNG opcode lists to apply to the image data.
  pub fn with_apply_dng_opcodes(mut self, apply_dng_opcodes: ApplyDngOpcodes) -> Self {
    self.apply_dng_opcodes = apply_dng_opcodes;
    self
  }

  /// Set the DNG opcodes to remove.
  pub fn with_remove_dng_opcodes(mut self, remove_dng_opcodes: Vec<OpcodeSelector>) -> Self {
    self.remove_dng_opcodes = remove_dng_opcodes;
    self
  }
}

/// Convert a raw input file into DNG
///
/// We don't accept a DNG file path here, because we don't know
/// how to handle existing target files, buffering, etc.
/// This is up to the caller.
pub fn convert_raw_file<W: Write + Seek + Send>(raw: &Path, dng: &mut W, params: &ConvertParams) -> crate::Result<ConvertInfo> {
  let original_filename = raw.file_name().and_then(OsStr::to_str).unwrap_or_default();
  //let raw_stream = BufReader::new(File::open(raw)?); // TODO: add path hint to error?
  //let rawfile = RawFile::new(PathBuf::from(raw), raw_stream);

  let rawfile = Arc::new(RawSource::new(raw)?);

  let original_compress_thread = if params.embedded {
    let orig_source = rawfile.clone();
    Some(std::thread::spawn(move || OriginalCompressed::compress(&mut orig_source.reader())))
  } else {
    None
  };

  internal_convert(&rawfile, dng, original_filename, original_compress_thread, params)
}

/// Convert a raw input file into DNG
pub fn convert_raw_source<W>(raw_source: &RawSource, dng: &mut W, original_filename: impl AsRef<str>, params: &ConvertParams) -> crate::Result<ConvertInfo>
where
  W: Write + Seek + Send,
{
  let original_compress_thread = if params.embedded {
    let mut original_stream = Cursor::new(raw_source.as_vec()?);
    Some(std::thread::spawn(move || OriginalCompressed::compress(&mut original_stream)))
  } else {
    None
  };

  internal_convert(raw_source, dng, original_filename, original_compress_thread, params)
}

fn internal_convert<W>(
  rawfile: &RawSource,
  dng: &mut W,
  original_filename: impl AsRef<str>,
  original_compress_thread: Option<JoinHandle<Result<OriginalCompressed, std::io::Error>>>,
  params: &ConvertParams,
) -> crate::Result<ConvertInfo>
where
  W: Write + Seek + Send,
{
  let decoder = crate::get_decoder(rawfile)?;
  let raw_params = RawDecodeParams { image_index: params.index };
  validate_image_index(decoder.as_ref(), &raw_params)?;
  let mut rawimage = decoder.raw_image(rawfile, &raw_params, false)?;
  rawimage.remove_opcodes(&params.remove_dng_opcodes);
  let metadata = decoder.raw_metadata(rawfile, &raw_params)?;

  log::info!(
    "DNG conversion: '{}', make: {}, model: {}, raw-image-count: {}",
    original_filename.as_ref(),
    rawimage.clean_make,
    rawimage.clean_model,
    decoder.raw_image_count()?
  );
  log::debug!("Raw image WB coeff: {:?}", rawimage.wb_coeffs);

  // If the raw image needs to be rotated, we do this before
  // writing the image to DNG. This requires scaling and debayer
  // to be applied.
  let fuji_rotation = rawimage.camera.find_hint("fuji_rotation") || rawimage.camera.find_hint("fuji_rotation_alt");
  let max_backward_version = params.max_dng_backward_version.as_dng_value();
  let present = STAGES.map(|stage| rawimage.opcode_list(stage).is_some_and(|list| !list.is_empty()));
  let needs_processing = STAGES.map(|stage| rawimage.opcode_list(stage).is_some_and(|list| list.needs_processing(max_backward_version)));
  let required_stage = required_stage(fuji_rotation, params.apply_scaling, params.compression);
  let (data_stage, apply) = processing_plan(present, needs_processing, params.apply_dng_opcodes, required_stage);
  for (stage, apply) in STAGES.into_iter().zip(apply) {
    // Bring the image data into the state the stage operates on
    match stage {
      OpcodeStage::List2 if data_stage >= stage => rawimage.apply_scaling()?,
      OpcodeStage::List3 if data_stage >= stage => {
        if fuji_rotation {
          log::debug!("Raw image requires fuji_rotation before writing to DNG");
        }
        demosaic_to_linear_raw(&mut rawimage, fuji_rotation)?;
      }
      _ => {}
    }
    if apply {
      log::debug!("Apply {} to the image data", stage);
      rawimage.apply_opcode_list(stage).map_err(|err| {
        RawlerError::DecoderFailed(format!(
          "Can't apply {} to the image data: {}",
          stage,
          match err {
            RawlerError::DecoderFailed(msg) => msg,
            err => err.to_string(),
          }
        ))
      })?;
    }
  }
  // Lossy JPEG supports one or three color planes
  let reduced_planes = params.compression == DngCompression::Lossy && rawimage.cpp == 4;
  if reduced_planes {
    // OpcodeList3 addresses the four color planes
    rawimage.apply_opcode_list(OpcodeStage::List3)?;
    reduce_to_three_planes(&mut rawimage)?;
  }
  let copy_tag = |tag: &u16| {
    let keep = !reduced_planes || !COLOR_PLANE_TAGS.iter().any(|t| *t as u16 == *tag);
    if !keep {
      log::debug!("Color planes are reduced, tag 0x{:04x} is not copied", tag);
    }
    keep
  };

  let mut dng = DngWriter::new_with_version(dng, params.dng_version.as_dng_value(), max_backward_version)?;

  // Write RAW image for subframe type 0
  // If no thumbnail should be written to root IFD, we need to put the raw image into
  // root IFD instead.
  let mut raw = if params.thumbnail { dng.subframe(0) } else { dng.subframe_on_root(0) };
  raw.raw_image(
    &rawimage,
    params.crop,
    params.compression,
    params.photometric_conversion,
    params.predictor,
    params.lossy_jpeg_quality,
  )?;
  // Check for DNG raw IFD related tags
  if let Some(dng_raw_ifd) = decoder.ifd(WellKnownIFD::VirtualDngRawTags)? {
    // Opcode lists are written by raw_image(), they may have been applied to the image data
    let opcode_lists = [DngTag::OpcodeList1 as u16, DngTag::OpcodeList2 as u16, DngTag::OpcodeList3 as u16];
    raw
      .ifd_mut()
      .copy(dng_raw_ifd.value_iter().filter(|(tag, _)| !opcode_lists.contains(tag) && copy_tag(tag)));
  }
  raw.finalize()?;

  // Write preview and thumbnail if requested
  if params.preview || params.thumbnail {
    // Each source image is generated at most once, and only if required.
    let need_embedded = (params.preview && !params.force_preview) || (params.thumbnail && !params.force_thumbnail);
    let need_developed = (params.preview && params.force_preview) || (params.thumbnail && params.force_thumbnail);
    let load = |force_develop: bool| match generate_preview(rawfile, decoder.as_ref(), &rawimage, &raw_params, force_develop) {
      Ok(image) => Some(image),
      Err(err) => {
        log::warn!("Failed to get review image, continue anyway: {:?}", err);
        None
      }
    };
    let embedded = if need_embedded { load(false) } else { None };
    let developed = if need_developed { load(true) } else { None };
    let select = |force: bool| if force { developed.as_ref() } else { embedded.as_ref() };

    if params.preview {
      if let Some(image) = select(params.force_preview) {
        let mut preview = dng.subframe(1);
        preview.preview(image, PREVIEW_JPEG_QUALITY)?;
        preview.finalize()?;
      }
    }
    if params.thumbnail {
      if let Some(image) = select(params.force_thumbnail) {
        dng.thumbnail(image)?;
      }
    }
  }
  // Write metadata
  dng.load_base_tags(&rawimage)?;
  dng.load_metadata(&metadata)?;
  if !dng.root_ifd().contains(ExifTag::Orientation) {
    dng.root_ifd_mut().add_tag(ExifTag::Orientation, rawimage.orientation.to_u16());
  }

  // Check for DNG root IFD related tags
  if let Some(dng_root_ifd) = decoder.ifd(WellKnownIFD::VirtualDngRootTags)? {
    dng.root_ifd_mut().copy(dng_root_ifd.value_iter().filter(|(tag, _)| copy_tag(tag)));
  }

  // Check for TIFF root IFD related tags
  if let Some(tiff_root) = decoder.ifd(WellKnownIFD::Root)? {
    dng.root_ifd_mut().copy(tiff_root.value_iter().filter(|(tag, _)| {
      [
        // Tags from CinemaDNG files
        TiffCommonTag::TimeCodes as u16,
        TiffCommonTag::FrameFrate as u16,
        TiffCommonTag::TStop as u16,
      ]
      .contains(tag)
    }));
  }

  // Remove makernotes from EXIF if MakerNoteSafety is not 1 (safe)
  if let Some(Entry {
    value: crate::formats::tiff::Value::Short(v),
    ..
  }) = decoder
    .ifd(WellKnownIFD::VirtualDngRootTags)?
    .and_then(|ifd| ifd.get_entry(DngTag::MakerNoteSafety).cloned())
  {
    if v.get(0).copied().unwrap_or(0) == 0 {
      dng.exif_ifd_mut().remove_tag(ExifTag::MakerNotes);
    }
  }

  if let Some(xpacket) = decoder.xpacket(rawfile, &raw_params)? {
    dng.xpacket(&xpacket)?;
  }

  if let Some(handle) = original_compress_thread {
    let original = handle
      .join()
      .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, format!("Failed to join compression thread: {:?}", err)))??;
    dng.original_file(&original, original_filename)?;
  }

  if let Some(artist) = &params.artist {
    dng.root_ifd_mut().add_tag(TiffCommonTag::Artist, artist);
  }
  if let Some(copyright) = &params.copyright {
    dng.root_ifd_mut().add_tag(ExifTag::Copyright, copyright);
  }
  dng.root_ifd_mut().add_tag(TiffCommonTag::Software, &params.software);

  dng
    .root_ifd_mut()
    .add_tag(ExifTag::ModifyDate, chrono::Local::now().format("%Y:%m:%d %H:%M:%S").to_string());

  dng.close()?;

  let last_modified = match metadata.last_modified() {
    Ok(Some(last_modified)) => Some(last_modified),
    Err(err) => {
      log::warn!("Failed to get last-modified: {:?}", err);
      None
    }
    _ => None,
  };
  Ok(ConvertInfo { last_modified })
}

/// Minimum processing stage of the image data written to DNG
fn required_stage(fuji_rotation: bool, apply_scaling: bool, compression: DngCompression) -> OpcodeStage {
  if fuji_rotation || compression == DngCompression::Lossy {
    // Rotation and lossy compression require demosaiced image data
    OpcodeStage::List3
  } else if apply_scaling {
    OpcodeStage::List2
  } else {
    OpcodeStage::List1
  }
}

/// Decide which opcode lists are applied to the image data, returns the processing
/// stage of the image data written to DNG and the lists to apply.
///
/// `present` are the opcode lists of the image. A list in `needs` can't be stored with
/// the maximum DNGBackwardVersion, `apply` are the lists requested by the user and
/// `required` is the minimum stage of the image data. An opcode list is applied as a
/// whole, as the opcodes may depend on each other. Lists of earlier stages are applied
/// as well, because a DNG reader would apply them to image data of the wrong stage.
fn processing_plan(present: [bool; 3], needs: [bool; 3], apply: ApplyDngOpcodes, required: OpcodeStage) -> (OpcodeStage, [bool; 3]) {
  let processed: [bool; 3] = std::array::from_fn(|i| needs[i] || (present[i] && apply.includes(STAGES[i])));
  let data_stage = STAGES
    .into_iter()
    .zip(processed)
    .filter_map(|(stage, processed)| processed.then_some(stage))
    .fold(required, Ord::max);
  let applied = std::array::from_fn(|i| present[i] && (STAGES[i] < data_stage || (STAGES[i] == data_stage && processed[i])));
  (data_stage, applied)
}

/// Demosaic scaled CFA image data into LinearRaw data, cropped to the active area.
/// Other image data is left unchanged, as stage 3 equals stage 2 for it.
///
/// Three-color sensors result in three planes, four-color sensors in four planes.
fn demosaic_to_linear_raw(rawimage: &mut RawImage, fuji_rotation: bool) -> crate::Result<()> {
  if !fuji_rotation && !matches!(rawimage.photometric, RawPhotometricInterpretation::Cfa(_)) {
    return Ok(());
  }
  let pixels = PixF32::new_with(rawimage.data.as_f32().into_owned(), rawimage.width, rawimage.height);
  let roi = rawimage.active_area.unwrap_or(pixels.rect());
  let cfa = &rawimage.camera.cfa;
  let colors = &rawimage.camera.plane_color;
  let (width, height, cpp, data) = match SensorType::from_cfa(cfa) {
    SensorType::Bayer if cfa.is_rgb() => {
      let mut rgb = PPGDemosaic::new().demosaic(&pixels, cfa, colors, roi);
      if fuji_rotation {
        let fuji_rotation_width = rawimage.fuji_rotation_width.expect("fuji_rotate: no rotation width found");
        let extra_rotate = rawimage.camera.find_hint("fuji_rotate_90cw");
        rgb = fuji_normalize_rotation(&rgb, fuji_rotation_width, extra_rotate);
      }
      (rgb.width, rgb.height, 3, rgb.into_flatten())
    }
    SensorType::Bayer if cfa.unique_colors() == 4 => {
      let image = Bilinear4Channel::new().demosaic(&pixels, cfa, colors, roi);
      (image.width, image.height, 4, image.into_flatten())
    }
    SensorType::Xtrans if cfa.is_rgb() => {
      let rgb = XTransMarkesteijnDemosaic::new_pass_3().demosaic(&pixels, cfa, colors, roi);
      (rgb.width, rgb.height, 3, rgb.into_flatten())
    }
    _ => return Err(RawlerError::DecoderFailed(format!("Demosaic of CFA pattern '{}' is not supported", cfa))),
  };
  if !fuji_rotation {
    // Crop area is relative to the full image, which is now cropped to the active area
    rawimage.crop_area = rawimage
      .crop_area
      .map(|crop| Rect::new(Point::new(crop.p.x.saturating_sub(roi.p.x), crop.p.y.saturating_sub(roi.p.y)), crop.d));
  }
  rawimage.width = width;
  rawimage.height = height;
  rawimage.active_area = None;
  rawimage.cpp = cpp;
  rawimage.blacklevel = BlackLevel::zero(1, 1, cpp);
  rawimage.whitelevel = WhiteLevel::new(vec![1; cpp]); // Already scaled up to 0.0 .. 1.0
  rawimage.photometric = RawPhotometricInterpretation::LinearRaw;
  rawimage.data = RawImageData::Float(data);
  Ok(())
}

/// Reduce demosaiced four-color image data to three color planes.
///
/// The planes are mapped by a 3x4 reduction matrix R and the color matrices are
/// replaced by R·CM. This is what a DNG reader does with a ReductionMatrix, but
/// applied to the image data, so the DNG has three color planes and no ReductionMatrix.
/// The image data can only be reduced once, so the reduction matrix for D65 is
/// preferred, see [`RawImage::reduction_matrices`].
fn reduce_to_three_planes(rawimage: &mut RawImage) -> crate::Result<()> {
  let (illuminant, matrix) = rawimage
    .reduction_matrices()
    .into_iter()
    .min_by_key(|(illuminant, _)| d65_preference(illuminant))
    .ok_or_else(|| RawlerError::DecoderFailed("Reducing four color planes requires a color matrix".into()))?;
  let reduction = transform_1d::<3, 4>(&matrix).expect("reduction matrix has 12 values");
  log::debug!("Reduce four color planes by {:?}, the reduction matrix for {:?}", reduction, illuminant);

  let pixels = Color2D::<f32, 4>::new_with(rawimage.data.as_f32().as_chunks::<4>().0.to_vec(), rawimage.width, rawimage.height);
  rawimage.data = RawImageData::Float(map_4ch_to_3ch(&pixels, &reduction).into_flatten());

  for matrix in rawimage.color_matrix.values_mut() {
    if let Some(xyz2cam) = transform_1d::<4, 3>(matrix) {
      *matrix = transform_2d(&multiply(&reduction, &xyz2cam));
    }
  }
  rawimage.color_matrix.retain(|illuminant, matrix| {
    let valid = matrix.len() == 9;
    if !valid {
      log::warn!(
        "Color matrix for {:?} has {} values, can't reduce it to three color planes",
        illuminant,
        matrix.len()
      );
    }
    valid
  });

  // Camera neutral in the reduced color space
  let wb = rawimage.wb_coeffs;
  rawimage.wb_coeffs = [f32::NAN; 4];
  if !wb[0].is_nan() {
    let neutral = reduction.map(|row| row.iter().zip(wb).map(|(r, wb)| r / wb).sum::<f32>());
    let max = neutral.iter().copied().fold(f32::MIN, f32::max);
    if neutral.iter().all(|n| *n > 0.0) {
      rawimage.wb_coeffs = [max / neutral[0], max / neutral[1], max / neutral[2], f32::NAN];
    } else {
      log::warn!(
        "Camera neutral {:?} is invalid after reducing the color planes, white balance is dropped",
        neutral
      );
    }
  }

  rawimage.cpp = 3;
  rawimage.blacklevel = BlackLevel::zero(1, 1, 3);
  rawimage.whitelevel = WhiteLevel::new(vec![1; 3]);
  // ReductionMatrix is only allowed for more than three color planes
  rawimage.reduction_matrix.clear();
  Ok(())
}

/// Get a preview image from the decoder, or develop it from raw data.
///
/// If `force_develop` is set, embedded previews are ignored.
fn generate_preview(
  rawfile: &RawSource,
  decoder: &dyn Decoder,
  rawimage: &RawImage,
  params: &RawDecodeParams,
  force_develop: bool,
) -> crate::Result<DynamicImage> {
  let embedded = if force_develop { None } else { decoder.preview_image(rawfile, params)? };
  match embedded {
    Some(image) => Ok(image),
    None => {
      if force_develop {
        log::debug!("Generate sRGB preview from RAW");
      } else {
        log::warn!("Preview image not found, try to generate sRGB from RAW");
      }
      let dev = RawDevelop::default();
      let image = dev.develop_intermediate(rawimage)?;
      /*
      let params = rawimage.develop_params()?;
      let (srgbf, dim) = develop_raw_srgb(&rawimage.data, &params)?;
      let output = convert_from_f32_scaled_u16(&srgbf, 0, u16::MAX);
      let image = if srgbf.len() == dim.w * dim.h {
        DynamicImage::ImageLuma16(ImageBuffer::from_raw(dim.w as u32, dim.h as u32, output).expect("Invalid ImageBuffer size"))
      } else {
        DynamicImage::ImageRgb16(ImageBuffer::from_raw(dim.w as u32, dim.h as u32, output).expect("Invalid ImageBuffer size"))
      };
       */
      Ok(image.to_dynamic_image().ok_or("failed to convert to dynamic image")?)
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{decoders::Camera, imgop::xyz::Illuminant, pixarray::PixU16};

  #[test]
  fn required_stage_for_compression() {
    use DngCompression as C;
    use OpcodeStage as S;
    assert_eq!(required_stage(false, false, C::Lossless), S::List1);
    assert_eq!(required_stage(false, true, C::Uncompressed), S::List2);
    assert_eq!(required_stage(true, false, C::Lossless), S::List3);
    // Lossy compression requires demosaiced data
    assert_eq!(required_stage(false, false, C::Lossy), S::List3);
    assert_eq!(required_stage(false, true, C::Lossy), S::List3);
  }

  #[test]
  fn demosaic_by_sensor_type() -> crate::Result<()> {
    use crate::{
      cfa::{CFA, PlaneColor},
      rawimage::CFAConfig,
    };

    const XTRANS: &str = "GGRGGBGGBGGRBRGRBGGGBGGRGGRGGBRBGBRG";
    for (pattern, colors, cpp) in [("RGGB", "RGB", 3), (XTRANS, "RGB", 3), ("CYGM", "CYGM", 4)] {
      let mut camera = Camera::new();
      camera.cfa = CFA::new(pattern);
      camera.plane_color = PlaneColor::new(colors);
      let photometric = RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&camera));
      let mut rawimage = RawImage::new(
        camera,
        PixU16::new_with(vec![0; 24 * 24], 24, 24),
        1,
        [1.0, 1.0, 1.0, 1.0],
        photometric,
        Some(BlackLevel::zero(1, 1, 1)),
        Some(WhiteLevel::new(vec![1])),
        false,
      );
      rawimage.data = RawImageData::Float(vec![0.5; 24 * 24]);

      demosaic_to_linear_raw(&mut rawimage, false)?;

      assert_eq!(rawimage.cpp, cpp, "{}", pattern);
      assert_eq!(rawimage.photometric, RawPhotometricInterpretation::LinearRaw);
      assert_eq!(rawimage.whitelevel.0, vec![1; cpp]);
      let data = rawimage.data.as_f32();
      assert_eq!(data.len(), 24 * 24 * cpp);
      // A flat field stays flat
      let center = (12 * 24 + 12) * cpp;
      assert!(
        data[center..center + cpp].iter().all(|v| (v - 0.5).abs() < 1e-3),
        "{}: {:?}",
        pattern,
        &data[center..center + cpp]
      );
    }
    Ok(())
  }

  /// Demosaiced CYGM image with white and a saturated color, with color matrices for D65 and A
  fn four_plane_image() -> RawImage {
    let mut rawimage = RawImage::new(
      Camera::new(),
      PixU16::new_with(vec![0; 2 * 4], 2 * 4, 1),
      4,
      // Camera neutral of D65 is (1, 1, 0.5, 1)
      [1.0, 1.0, 2.0, 1.0],
      RawPhotometricInterpretation::LinearRaw,
      Some(BlackLevel::zero(1, 1, 4)),
      Some(WhiteLevel::new(vec![1; 4])),
      false,
    );
    rawimage.data = RawImageData::Float(vec![1.0, 1.0, 1.0, 1.0, 0.8, 0.2, 0.1, 0.4]);
    // cyan = G + B, yellow = R + G, green = G, magenta = R + B
    let rgb2cam = [[0.0, 1.0, 1.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0, 1.0]];
    let xyz2cam = transform_2d(&multiply(&rgb2cam, &crate::imgop::xyz::XYZ_TO_SRGB_D65));
    rawimage.color_matrix.insert(Illuminant::D65, xyz2cam.clone());
    rawimage.color_matrix.insert(Illuminant::A, xyz2cam.iter().map(|v| v * 1.1).collect());
    rawimage
  }

  #[test]
  fn reduce_four_planes_to_three() -> crate::Result<()> {
    let mut rawimage = four_plane_image();

    reduce_to_three_planes(&mut rawimage)?;

    assert_eq!(rawimage.cpp, 3);
    assert_eq!(rawimage.whitelevel.0, vec![1; 3]);
    let data = rawimage.data.as_f32();
    assert_eq!(data.len(), 2 * 3);
    // Camera white stays white, colors stay in range
    assert!(data[..3].iter().all(|v| (v - 1.0).abs() < 1e-4), "white is {:?}", &data[..3]);
    assert!(data.iter().all(|v| (0.0..=1.0 + 1e-6).contains(v)), "data is {:?}", data);
    assert!(rawimage.color_matrix.values().all(|matrix| matrix.len() == 9));
    // The neutral is mapped into the reduced planes and normalized to a maximum of 1.0
    assert!(rawimage.wb_coeffs[3].is_nan());
    let neutral: Vec<f32> = rawimage.wb_coeffs[..3].iter().map(|wb| 1.0 / wb).collect();
    assert!(neutral.iter().all(|n| *n > 0.0 && *n <= 1.0 + 1e-6), "neutral is {:?}", neutral);
    assert!(neutral.iter().any(|n| (n - 1.0).abs() < 1e-6), "neutral is {:?}", neutral);
    Ok(())
  }

  #[test]
  fn reduce_four_planes_by_camera_reduction_matrix() -> crate::Result<()> {
    let mut rawimage = four_plane_image();
    let xyz2cam_a = transform_1d::<4, 3>(&rawimage.color_matrix[&Illuminant::A]).expect("4x3 matrix");
    // The matrix for D65 is preferred to reduce the image data
    let provided = [[1.0, 0.0, 0.0, 0.0], [0.0, 0.5, 0.0, 0.5], [0.0, 0.0, 1.0, 0.0]];
    rawimage.reduction_matrix.insert(Illuminant::D65, transform_2d(&provided));
    rawimage.reduction_matrix.insert(Illuminant::A, vec![0.25; 12]);

    reduce_to_three_planes(&mut rawimage)?;

    let data = rawimage.data.as_f32();
    let expected = [1.0, 1.0, 1.0, 0.8, 0.3, 0.1];
    assert!(data.iter().zip(expected).all(|(v, e)| (v - e).abs() < 1e-6), "data is {:?}", data);
    assert_eq!(rawimage.color_matrix[&Illuminant::A], transform_2d(&multiply(&provided, &xyz2cam_a)));
    // Three color planes have no reduction matrix
    assert!(rawimage.reduction_matrix.is_empty());
    Ok(())
  }

  #[test]
  fn reduce_four_planes_requires_color_matrix() {
    let mut rawimage = RawImage::new(
      Camera::new(),
      PixU16::new_with(vec![0; 4], 4, 1),
      4,
      [1.0, 1.0, 1.0, 1.0],
      RawPhotometricInterpretation::LinearRaw,
      Some(BlackLevel::zero(1, 1, 4)),
      Some(WhiteLevel::new(vec![1; 4])),
      false,
    );
    rawimage.color_matrix.clear();
    assert!(reduce_to_three_planes(&mut rawimage).is_err());
  }

  #[test]
  fn processing_plan_stages() {
    use ApplyDngOpcodes as A;
    use OpcodeStage as S;
    const ALL: [bool; 3] = [true, true, true];
    const NONE: [bool; 3] = [false, false, false];
    let cases = [
      // Lists are written as they are
      (ALL, NONE, A::Auto, S::List1, (S::List1, NONE)),
      // Lists requested by the user
      (ALL, NONE, A::List1, S::List1, (S::List1, [true, false, false])),
      (ALL, NONE, A::List2, S::List1, (S::List2, [true, true, false])),
      (ALL, NONE, A::List3, S::List1, (S::List3, ALL)),
      // The image data is only processed up to the last present list
      ([true, false, false], NONE, A::List3, S::List1, (S::List1, [true, false, false])),
      ([false, false, true], NONE, A::List2, S::List1, (S::List1, NONE)),
      // A list that can't be stored is applied with all earlier lists
      (ALL, [false, false, true], A::List1, S::List1, (S::List3, ALL)),
      (ALL, [false, true, false], A::Auto, S::List1, (S::List2, [true, true, false])),
      // Scaling applies OpcodeList1 and keeps OpcodeList2
      (ALL, NONE, A::Auto, S::List2, (S::List2, [true, false, false])),
      // Missing lists are not applied
      ([false, true, false], [false, true, false], A::Auto, S::List1, (S::List2, [false, true, false])),
    ];
    for (present, needs, apply, required, expected) in cases {
      assert_eq!(
        processing_plan(present, needs, apply, required),
        expected,
        "present {:?}, needs {:?}, apply {:?}, required {:?}",
        present,
        needs,
        apply,
        required
      );
    }
  }
}
