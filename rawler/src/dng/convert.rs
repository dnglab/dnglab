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
    PREVIEW_JPEG_QUALITY,
    opcodes::{OpcodeSelector, OpcodeStage},
    original::OriginalCompressed,
    writer::DngWriter,
  },
  formats::tiff::Entry,
  imgop::{
    Point, Rect,
    develop::RawDevelop,
    fuji_rotate::fuji_normalize_rotation,
    sensor::{Demosaic, bayer::ppg::PPGDemosaic},
  },
  pixarray::PixF32,
  rawimage::BlackLevel,
  rawsource::RawSource,
  tags::{DngTag, ExifTag, TiffCommonTag},
};

use super::{ApplyDngOpcodes, CropMode, DngCompression, DngPhotometricConversion, DngVersion};

/// Opcode list stages in processing order
const STAGES: [OpcodeStage; 3] = [OpcodeStage::List1, OpcodeStage::List2, OpcodeStage::List3];

/// Parameters for DNG conversion
#[derive(Clone, Debug)]
pub struct ConvertParams {
  pub embedded: bool,
  pub compression: DngCompression,
  pub photometric_conversion: DngPhotometricConversion,
  pub apply_scaling: bool,
  pub crop: CropMode,
  pub predictor: u8,
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
  let required_stage = if fuji_rotation {
    OpcodeStage::List3
  } else if params.apply_scaling {
    OpcodeStage::List2
  } else {
    OpcodeStage::List1
  };
  let (data_stage, apply) = processing_plan(present, needs_processing, params.apply_dng_opcodes, required_stage);
  for (stage, apply) in STAGES.into_iter().zip(apply) {
    // Bring the image data into the state the stage operates on
    match stage {
      OpcodeStage::List2 if data_stage >= stage => rawimage.apply_scaling()?,
      OpcodeStage::List3 if data_stage >= stage => {
        if fuji_rotation {
          log::debug!("Raw image requires fuji_rotation before writing to DNG");
        }
        demosaic_to_linear_raw(&mut rawimage, fuji_rotation);
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

  let mut dng = DngWriter::new_with_version(dng, params.dng_version.as_dng_value(), max_backward_version)?;

  // Write RAW image for subframe type 0
  // If no thumbnail should be written to root IFD, we need to put the raw image into
  // root IFD instead.
  let mut raw = if params.thumbnail { dng.subframe(0) } else { dng.subframe_on_root(0) };
  raw.raw_image(&rawimage, params.crop, params.compression, params.photometric_conversion, params.predictor)?;
  // Check for DNG raw IFD related tags
  if let Some(dng_raw_ifd) = decoder.ifd(WellKnownIFD::VirtualDngRawTags)? {
    // Opcode lists are written by raw_image(), they may have been applied to the image data
    let opcode_lists = [DngTag::OpcodeList1 as u16, DngTag::OpcodeList2 as u16, DngTag::OpcodeList3 as u16];
    raw.ifd_mut().copy(dng_raw_ifd.value_iter().filter(|(tag, _)| !opcode_lists.contains(tag)));
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
    dng.root_ifd_mut().copy(dng_root_ifd.value_iter());
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

/// Demosaic scaled CFA image data into three-color LinearRaw data, cropped to the
/// active area. Other image data is left unchanged, as stage 3 equals stage 2 for it.
fn demosaic_to_linear_raw(rawimage: &mut RawImage, fuji_rotation: bool) {
  if !fuji_rotation && !matches!(rawimage.photometric, RawPhotometricInterpretation::Cfa(_)) {
    return;
  }
  let pixels = PixF32::new_with(rawimage.data.as_f32().into_owned(), rawimage.width, rawimage.height);
  let roi = rawimage.active_area.unwrap_or(pixels.rect());
  let demosaic = PPGDemosaic::new();
  let mut rgb = demosaic.demosaic(&pixels, &rawimage.camera.cfa, &rawimage.camera.plane_color, roi);
  if fuji_rotation {
    let fuji_rotation_width = rawimage.fuji_rotation_width.expect("fuji_rotate: no rotation width found");
    let extra_rotate = rawimage.camera.find_hint("fuji_rotate_90cw");
    rgb = fuji_normalize_rotation(&rgb, fuji_rotation_width, extra_rotate);
  } else {
    // Crop area is relative to the full image, which is now cropped to the active area
    rawimage.crop_area = rawimage
      .crop_area
      .map(|crop| Rect::new(Point::new(crop.p.x.saturating_sub(roi.p.x), crop.p.y.saturating_sub(roi.p.y)), crop.d));
  }
  rawimage.width = rgb.width;
  rawimage.height = rgb.height;
  rawimage.active_area = None;
  rawimage.cpp = 3;
  rawimage.blacklevel = BlackLevel::zero(1, 1, 3);
  rawimage.whitelevel = WhiteLevel::new([1, 1, 1]); // Already scaled up to 0.0 .. 1.0
  rawimage.photometric = RawPhotometricInterpretation::LinearRaw;
  rawimage.data = RawImageData::Float(rgb.into_flatten());
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
