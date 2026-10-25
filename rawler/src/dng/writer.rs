use std::{
  borrow::Cow,
  io::{self, Seek, Write},
  mem::size_of,
  time::Instant,
};

use image::{DynamicImage, ExtendedColorType, codecs::jpeg::JpegEncoder, imageops::FilterType};
use log::debug;
use rayon::prelude::*;

use crate::{
  CFA, RawImage, RawImageData,
  compressors::jpegxl::{self, LossyTransfer, TileSamples, TileSettings},
  decoders::{Camera, RawMetadata},
  dng::rect_to_dng_area,
  envparams::{rawler_dng_multistrip_threshold, rawler_dng_rows_per_strip},
  formats::tiff::{
    CompressionMethod, PhotometricInterpretation, PreviewColorSpace, Rational, TiffError, Value,
    writer::{DirectoryWriter, TiffWriter, transfer_entry},
  },
  imgop::{Dim2, Point, Rect},
  ljpeg92::LjpegCompressor,
  pixarray::{Pix2D, PixU16, interleave_cols, interleave_rows},
  rawimage::{BlackLevel, RawPhotometricInterpretation, WhiteLevel},
  tags::ExifTag,
  tiles::ImageTiler,
};
use crate::{
  formats::tiff::SRational,
  imgop::xyz::Illuminant,
  tags::{DngTag, TiffCommonTag},
};

use super::{
  CompressionParams, CropMode, DNG_VERSION_V1_0, DNG_VERSION_V1_1, DNG_VERSION_V1_4, DNG_VERSION_V1_7, DNG_VERSION_V1_7_1, DngCompression,
  DngPhotometricConversion, DngVersionNumber, JxlParams, opcodes::OpcodeStage, original::OriginalCompressed,
};

pub type DngError = TiffError;

pub type Result<T> = std::result::Result<T, DngError>;

pub struct DngWriter<B>
where
  B: Write + Seek,
{
  pub dng: TiffWriter<B>,
  root_ifd: DirectoryWriter,
  //raw_ifd: DirectoryWriter,
  //preview_ifd: DirectoryWriter,
  exif_ifd: DirectoryWriter,
  subs: Vec<u32>,
  /// Highest DNGBackwardVersion the output is allowed to declare
  max_backward_version: DngVersionNumber,
  /// DNGBackwardVersion required by the features used so far
  dng_backward_version: DngVersionNumber,
}

pub struct SubFrameWriter<'w, B>
where
  B: Write + Seek,
{
  writer: &'w mut DngWriter<B>,
  ifd: Option<DirectoryWriter>,
}

impl<'w, B> SubFrameWriter<'w, B>
where
  B: Write + Seek,
{
  pub fn new(writer: &'w mut DngWriter<B>, subtype: u32, use_root: bool) -> Self {
    let ifd = if use_root {
      writer.root_ifd_mut().add_tag(TiffCommonTag::NewSubFileType, subtype);
      None
    } else {
      let mut ifd = DirectoryWriter::new();
      ifd.add_tag(TiffCommonTag::NewSubFileType, subtype);
      Some(ifd)
    };
    Self { ifd, writer }
  }

  pub fn ifd(&mut self) -> &DirectoryWriter {
    self.ifd.as_ref().unwrap_or(self.writer.root_ifd())
  }

  pub fn ifd_mut(&mut self) -> &mut DirectoryWriter {
    self.ifd.as_mut().unwrap_or(self.writer.root_ifd_mut())
  }

  pub fn rgb_image_u8(&mut self, data: &[u8], width: usize, height: usize, compression: DngCompression, params: &CompressionParams) -> Result<()> {
    let cpp = 3;
    let rawimagedata = PixU16::new_with(data.iter().copied().map(u16::from).collect(), width * cpp, height);
    let mut cam = Camera::new();
    cam.cfa = CFA::new("RGGB");

    let wb_coeffs = [1.0, 1.0, 1.0, 1.0];
    let blacklevel = Some(BlackLevel::new(&[0_u32, 0, 0], 1, 1, 3));
    let whitelevel = Some(WhiteLevel::new_bits(8, cpp));
    let photometric = RawPhotometricInterpretation::LinearRaw;
    let rawimage = RawImage::new(cam, rawimagedata, cpp, wb_coeffs, photometric, blacklevel, whitelevel, false);
    self.raw_image(&rawimage, CropMode::None, compression, DngPhotometricConversion::Original, params)
  }

  pub fn rgb_image_u16(&mut self, data: &[u16], width: usize, height: usize, compression: DngCompression, params: &CompressionParams) -> Result<()> {
    let cpp = 3;
    let rawimagedata = PixU16::new_with(data.to_vec(), width * cpp, height);
    let mut cam = Camera::new();
    cam.cfa = CFA::new("RGGB");

    let wb_coeffs = [1.0, 1.0, 1.0, 1.0];
    let blacklevel = Some(BlackLevel::new(&[0_u32, 0, 0], 1, 1, 3));
    let whitelevel = Some(WhiteLevel::new_bits(16, cpp));
    let photometric = RawPhotometricInterpretation::LinearRaw;
    let rawimage = RawImage::new(cam, rawimagedata, cpp, wb_coeffs, photometric, blacklevel, whitelevel, false);
    self.raw_image(&rawimage, CropMode::None, compression, DngPhotometricConversion::Original, params)
  }

  pub fn image(&mut self, _image: &RawImageData, _width: u16, _height: u16) -> Result<()> {
    todo!()
  }

  pub fn raw_image(
    &mut self,
    rawimage: &RawImage,
    cropmode: CropMode,
    compression: DngCompression,
    photometric_conversion: DngPhotometricConversion,
    params: &CompressionParams,
  ) -> Result<()> {
    match photometric_conversion {
      DngPhotometricConversion::Original => self.write_rawimage(Cow::Borrowed(rawimage), cropmode, compression, params)?,

      DngPhotometricConversion::Linear => {
        if rawimage.cpp == 3 {
          self.write_rawimage(Cow::Borrowed(rawimage), cropmode, compression, params)?;
        } else {
          let rawimage = rawimage.linearize().map_err(|e| DngError::General(format!("linearization failed: {}", e)))?;
          self.write_rawimage(Cow::Borrowed(&rawimage), cropmode, compression, params)?;
        }
      }
    }

    /*
    for (tag, value) in rawimage.dng_tags.iter() {
      self.ifd.add_untyped_tag(*tag, value.clone())?;
    }
     */

    Ok(())
  }

  fn write_rawimage(&mut self, mut rawimage: Cow<RawImage>, cropmode: CropMode, compression: DngCompression, params: &CompressionParams) -> Result<()> {
    // Readers process OpcodeList2 and OpcodeList3 on 16 bit data for 16 bit integer
    // raw data, otherwise on floats (as in the DNG SDK).
    let source_integer = match rawimage.data {
      RawImageData::Integer(_) => true,
      RawImageData::Float(_) => rawimage.scaled_from_integer,
    };
    // Area of the image data, without padding added for compression
    let image_area = Rect::new(Point::new(0, 0), Dim2::new(rawimage.width, rawimage.height));
    // The table maps the stored integer values, it must be applied by apply_scaling()
    // before the data is converted to floats.
    if rawimage.linearization_table.is_some() && matches!(rawimage.data, RawImageData::Float(_)) {
      return Err(DngError::General("LinearizationTable requires integer image data".into()));
    }
    if compression == DngCompression::Lossy {
      self.writer.update_backward_version(DNG_VERSION_V1_4, "Lossy JPEG compression")?;
      to_lossy_8bit(rawimage.to_mut())?;
    }
    if compression == DngCompression::JpegXl {
      // Fail before any processing if libjxl is missing
      jpegxl::libjxl_version().map_err(DngError::General)?;
      self.writer.update_backward_version(DNG_VERSION_V1_7, "JPEG XL compression")?;
      if jxl_interleave(&rawimage).is_some() {
        self.writer.update_backward_version(DNG_VERSION_V1_7_1, "ColumnInterleaveFactor")?;
      }
      if matches!(rawimage.data, RawImageData::Float(_)) && !jxl_stores_float(&rawimage) {
        float_to_integer(rawimage.to_mut());
      }
      if !params.jxl.is_lossless() {
        if matches!(rawimage.data, RawImageData::Integer(_)) {
          to_jxl_lossy_16bit(rawimage.to_mut());
        }
        if jxl_interleave(&rawimage).is_some() {
          pad_jxl_fields(rawimage.to_mut());
        }
      }
    }
    if matches!(rawimage.data, RawImageData::Float(_))
      && (compression == DngCompression::Lossless || !self.writer.backward_version_allows(DNG_VERSION_V1_4, "Floating point image data"))
    {
      // Lossless (LJPEG92) can only be used for 16 bit integer data and
      // floating point data requires DNG 1.4. If we have floats, convert them.
      float_to_integer(rawimage.to_mut());
    }
    // Deltas of DeltaPerRow and DeltaPerColumn are in pixel units of that data,
    // so they must be converted if the stored sample format differs.
    let delta_factor = match (source_integer, matches!(rawimage.data, RawImageData::Integer(_))) {
      (true, false) => 1.0 / u16::MAX as f32,
      (false, true) => u16::MAX as f32,
      _ => 1.0,
    };

    if rawimage.cpp > 1 || matches!(rawimage.photometric, RawPhotometricInterpretation::Cfa(_)) {
      self.writer.as_shot_neutral(wbcoeff_to_tiff_value(&rawimage));
      // ReductionMatrix is required for more than three color planes
      let color_planes = match &rawimage.photometric {
        RawPhotometricInterpretation::Cfa(config) => config.colors.plane_count(),
        _ => rawimage.cpp,
      };
      let reduction_matrices = if color_planes > 3 {
        rawimage.reduction_matrices()
      } else {
        Default::default()
      };
      // Add matrix and illumninant
      let mut available_matrices = rawimage.color_matrix.clone();
      if let Some(first_key) = available_matrices.keys().next().cloned() {
        let first_matrix = available_matrices
          .remove_entry(&Illuminant::A)
          .or_else(|| available_matrices.remove_entry(&Illuminant::A))
          .or_else(|| available_matrices.remove_entry(&first_key))
          .expect("No matrix found");
        self
          .writer
          .color_matrix(1, first_matrix.0, matrix_to_tiff_value(&first_matrix.1, 10_000).as_slice());
        if let Some(reduction) = reduction_matrices.get(&first_matrix.0) {
          self.writer.reduction_matrix(1, matrix_to_tiff_value(reduction, 10_000).as_slice());
        }
        if let Some(second_matrix) = available_matrices
          .remove_entry(&Illuminant::D65)
          .or_else(|| available_matrices.remove_entry(&Illuminant::D50))
        {
          self
            .writer
            .color_matrix(2, second_matrix.0, matrix_to_tiff_value(&second_matrix.1, 10_000).as_slice());
          if let Some(reduction) = reduction_matrices.get(&second_matrix.0) {
            self.writer.reduction_matrix(2, matrix_to_tiff_value(reduction, 10_000).as_slice());
          }
        }
      }
    }

    let full_size = Rect::new(Point::new(0, 0), Dim2::new(rawimage.width, rawimage.height));

    // Active area or uncropped, padding is never part of the active area
    let mut active_area: Rect = match cropmode {
      CropMode::ActiveArea | CropMode::Best => rawimage.active_area.unwrap_or(image_area),
      CropMode::None => image_area,
    };

    assert!(active_area.p.x + active_area.d.w <= rawimage.width);
    assert!(active_area.p.y + active_area.d.h <= rawimage.height);

    // Default crop, in full image coordinates
    let crop: Option<Rect> = match cropmode {
      CropMode::ActiveArea => Some(active_area),
      CropMode::Best => Some(rawimage.crop_area.unwrap_or(active_area)),
      CropMode::None => None,
    };

    //self.ifd.add_tag(TiffCommonTag::NewSubFileType, 0_u16)?; // Raw
    self.ifd_mut().add_tag(TiffCommonTag::ImageWidth, rawimage.width as u32);
    self.ifd_mut().add_tag(TiffCommonTag::ImageLength, rawimage.height as u32);

    // ActiveArea defaults to the full image, so the tag is only required
    // if masked pixels are stored.
    if active_area != full_size {
      if self.writer.backward_version_allows(DNG_VERSION_V1_1, "ActiveArea (masked pixels stored)") {
        self.writer.update_backward_version(DNG_VERSION_V1_1, "ActiveArea (masked pixels stored)")?;
        self.ifd_mut().add_tag(DngTag::ActiveArea, rect_to_dng_area(&active_area));
      } else {
        // ActiveArea is not available, so the full image becomes the active area.
        // The masked pixels are still hidden by the default crop.
        active_area = full_size;
      }
    }

    if let Some(crop) = crop {
      assert!(crop.p.x >= active_area.p.x);
      assert!(crop.p.y >= active_area.p.y);
      self.ifd_mut().add_tag(
        DngTag::DefaultCropOrigin,
        [(crop.p.x - active_area.p.x) as u16, (crop.p.y - active_area.p.y) as u16],
      );
      self.ifd_mut().add_tag(DngTag::DefaultCropSize, [crop.d.w as u16, crop.d.h as u16]);
    }

    self.ifd_mut().add_tag(ExifTag::PlanarConfiguration, 1_u16);

    self.ifd_mut().add_tag(
      DngTag::DefaultScale,
      [
        Rational::new(rawimage.camera.default_scale.0[0][0], rawimage.camera.default_scale.0[0][1]),
        Rational::new(rawimage.camera.default_scale.0[1][0], rawimage.camera.default_scale.0[1][1]),
      ],
    );
    self.ifd_mut().add_tag(
      DngTag::BestQualityScale,
      Rational::new(rawimage.camera.best_quality_scale.0[0], rawimage.camera.best_quality_scale.0[1]),
    );

    // Whitelevel
    assert_eq!(rawimage.whitelevel.0.len(), rawimage.cpp, "Whitelevel sample count must match cpp");

    if rawimage.whitelevel.0.iter().all(|x| *x <= (u16::MAX as u32)) {
      // Add as u16
      self
        .ifd_mut()
        .add_tag(DngTag::WhiteLevel, &rawimage.whitelevel.0.iter().map(|x| *x as u16).collect::<Vec<u16>>());
    } else {
      self.ifd_mut().add_tag(DngTag::WhiteLevel, &rawimage.whitelevel.0);
    }

    // Blacklevel
    let blacklevel = rawimage.blacklevel.shift(active_area.p.x, active_area.p.y);

    self
      .ifd_mut()
      .add_tag(DngTag::BlackLevelRepeatDim, [blacklevel.height as u16, blacklevel.width as u16]);

    if blacklevel.levels.iter().all(|x| x.d == 1) {
      let payload: Vec<u32> = blacklevel.levels.iter().map(|x| x.n as u32).collect();
      if payload.iter().all(|x| *x <= (u16::MAX as u32)) {
        // Add as u16
        self
          .ifd_mut()
          .add_tag(DngTag::BlackLevel, &payload.into_iter().map(|x| x as u16).collect::<Vec<u16>>());
      } else {
        // Add as u32
        self.ifd_mut().add_tag(DngTag::BlackLevel, &payload);
      }
    } else {
      // Add as RATIONAL
      self.ifd_mut().add_tag(DngTag::BlackLevel, blacklevel.levels.as_slice());
    }

    if !rawimage.blackareas.is_empty() {
      let data: Vec<u16> = rawimage.blackareas.iter().flat_map(rect_to_dng_area).collect();
      self.ifd_mut().add_tag(DngTag::MaskedAreas, &data);
    }

    self.ifd_mut().add_tag(TiffCommonTag::SamplesPerPixel, rawimage.cpp as u16);

    match &rawimage.photometric {
      RawPhotometricInterpretation::BlackIsZero => {
        assert_eq!(rawimage.cpp, 1);
        self.ifd_mut().add_tag(TiffCommonTag::PhotometricInt, PhotometricInterpretation::BlackIsZero);
      }
      RawPhotometricInterpretation::Cfa(config) => {
        assert!(config.cfa.is_valid());
        assert_eq!(rawimage.cpp, 1);
        let cfa = config.cfa.shift(active_area.p.x, active_area.p.y);
        self
          .ifd_mut()
          .add_tag(TiffCommonTag::CFARepeatPatternDim, [cfa.width as u16, cfa.height as u16]);
        self.ifd_mut().add_tag(TiffCommonTag::CFAPattern, &cfa.flat_pattern()[..]);
        self.ifd_mut().add_tag(TiffCommonTag::PhotometricInt, PhotometricInterpretation::CFA);
        self.ifd_mut().add_tag(DngTag::CFAPlaneColor, &config.colors);
        self.ifd_mut().add_tag(DngTag::CFALayout, 1_u16); // Square layout
      }
      RawPhotometricInterpretation::LinearRaw => {
        self.ifd_mut().add_tag(TiffCommonTag::PhotometricInt, PhotometricInterpretation::LinearRaw);
      }
    }

    // LinearizationTable not applied to the image data
    if let Some(table) = &rawimage.linearization_table {
      self.ifd_mut().add_tag(DngTag::LinearizationTable, &table[..]);
    }

    // Opcode lists not applied to the image data
    for (stage, tag) in [
      (OpcodeStage::List1, DngTag::OpcodeList1),
      (OpcodeStage::List2, DngTag::OpcodeList2),
      (OpcodeStage::List3, DngTag::OpcodeList3),
    ] {
      let Some(mut list) = rawimage.opcode_list(stage).filter(|list| !list.is_empty()).map(Cow::Borrowed) else {
        continue;
      };
      if stage != OpcodeStage::List1 && delta_factor != 1.0 {
        debug!("Scale deltas of {} by {} for the stored sample format", stage, delta_factor);
        list.to_mut().scale_deltas(delta_factor);
      }
      // DNGBackwardVersion must not be less than the version of any
      // required opcode (DNG spec, Appendix A, issue 7 and 16).
      for opcode in list.opcodes().iter().filter(|opcode| !opcode.is_optional()) {
        self
          .writer
          .update_backward_version(opcode.version(), &format!("required opcode {} in {}", opcode.name(), stage))?;
      }
      if stage != OpcodeStage::List1 && rawimage.active_area.unwrap_or(image_area) != active_area {
        log::warn!(
          "{} is relative to ActiveArea {:?}, but ActiveArea {:?} is written, opcodes may affect the wrong image area",
          stage,
          rawimage.active_area.unwrap_or(image_area),
          active_area
        );
      }
      let data = list
        .to_bytes()
        .map_err(|err| DngError::General(format!("Failed to write {}: {}", stage, err)))?;
      self.ifd_mut().add_tag(tag, Value::Undefined(data));
    }

    match compression {
      DngCompression::Uncompressed => {
        self.ifd_mut().add_tag(TiffCommonTag::Compression, CompressionMethod::None);
        dng_put_raw_uncompressed(self, &rawimage)?;
      }
      DngCompression::Lossless => {
        self.ifd_mut().add_tag(TiffCommonTag::Compression, CompressionMethod::ModernJPEG);
        dng_put_raw_ljpeg(self, &rawimage, params.ljpeg92_predictor)?;
      }
      DngCompression::Lossy => {
        self.ifd_mut().add_tag(TiffCommonTag::Compression, CompressionMethod::LossyJPEG);
        dng_put_raw_lossy_jpeg(self, &rawimage, params.lossy_jpeg_quality)?;
      }
      DngCompression::JpegXl => {
        self.ifd_mut().add_tag(TiffCommonTag::Compression, CompressionMethod::JPEGXL);
        dng_put_raw_jxl(self, &rawimage, &params.jxl)?;
      }
    }

    /*
    for (tag, value) in rawimage.dng_tags.iter() {
      self.ifd.add_untyped_tag(*tag, value.clone())?;
    }
     */

    Ok(())
  }

  pub fn preview(&mut self, img: &DynamicImage, quality: f32) -> Result<()> {
    let now = Instant::now();
    let preview_img = if img.width() > 1024 {
      DynamicImage::ImageRgb8(img.resize(1024, 768, FilterType::Nearest).to_rgb8())
    } else {
      DynamicImage::ImageRgb8(img.to_rgb8())
    };
    debug!("preview downscale: {} s", now.elapsed().as_secs_f32());

    self.ifd_mut().add_tag(TiffCommonTag::ImageWidth, Value::long(preview_img.width()));
    self.ifd_mut().add_tag(TiffCommonTag::ImageLength, Value::long(preview_img.height()));
    self.ifd_mut().add_tag(TiffCommonTag::Compression, CompressionMethod::ModernJPEG);
    self.ifd_mut().add_tag(TiffCommonTag::BitsPerSample, [8_u16, 8, 8]);
    self.ifd_mut().add_tag(TiffCommonTag::SampleFormat, [1_u16, 1, 1]);
    self.ifd_mut().add_tag(TiffCommonTag::PhotometricInt, PhotometricInterpretation::YCbCr);
    self.ifd_mut().add_tag(TiffCommonTag::RowsPerStrip, Value::long(preview_img.height()));
    self.ifd_mut().add_tag(TiffCommonTag::SamplesPerPixel, 3_u16);
    self.ifd_mut().add_tag(DngTag::PreviewColorSpace, PreviewColorSpace::SRgb); // Default for color previews, no DNGBackwardVersion 1.2 required

    //ifd.add_tag(TiffRootTag::XResolution, Rational { n: 1, d: 1 })?;
    //ifd.add_tag(TiffRootTag::YResolution, Rational { n: 1, d: 1 })?;
    //ifd.add_tag(TiffRootTag::ResolutionUnit, ResolutionUnit::None.to_u16())?;

    let now = Instant::now();
    let offset = self.writer.dng.position()?;
    // TODO: improve offsets?
    let jpeg_encoder = JpegEncoder::new_with_quality(&mut self.writer.dng.writer, (quality * 100.0).clamp(0.0, 100.0) as u8);
    preview_img
      .write_with_encoder(jpeg_encoder)
      .map_err(|err| io::Error::new(io::ErrorKind::Other, format!("Failed to write jpeg preview: {:?}", err)))?;
    let data_len = self.writer.dng.position()? - offset;
    debug!("writing preview: {} s", now.elapsed().as_secs_f32());

    self.ifd_mut().add_value(TiffCommonTag::StripOffsets, Value::Long(vec![offset]));
    self.ifd_mut().add_tag(TiffCommonTag::StripByteCounts, Value::Long(vec![data_len]));

    Ok(())
  }

  pub fn finalize(self) -> Result<()> {
    if let Some(ifd) = self.ifd {
      let offset = ifd.build(&mut self.writer.dng)?;
      self.writer.subs.push(offset);
    }
    Ok(())
  }
}

impl<B> DngWriter<B>
where
  B: Write + Seek,
{
  pub fn new(buf: B, backward_version: DngVersionNumber) -> Result<Self> {
    Self::new_with_version(buf, DNG_VERSION_V1_7_1, backward_version)
  }

  /// Create a new DNG writer with explicit DNGVersion.
  ///
  /// `max_backward_version` is the highest DNGBackwardVersion the output is
  /// allowed to declare.
  pub fn new_with_version(buf: B, version: DngVersionNumber, max_backward_version: DngVersionNumber) -> Result<Self> {
    let max_backward_version = if max_backward_version > version {
      log::warn!("DNG backward version {} is higher than DNG version {}, clamping", max_backward_version, version);
      version
    } else {
      max_backward_version
    };

    let dng = TiffWriter::new(buf)?;

    let mut root_ifd = DirectoryWriter::new();
    let mut exif_ifd = DirectoryWriter::new();
    // DNGBackwardVersion is derived from the used features and written on close().
    root_ifd.add_tag(DngTag::DNGVersion, version);
    // Add EXIF version 0220
    exif_ifd.add_tag_undefined(ExifTag::ExifVersion, vec![48, 50, 50, 48]);

    Ok(Self {
      dng,
      root_ifd,
      exif_ifd,
      subs: Vec::new(),
      max_backward_version,
      dng_backward_version: DNG_VERSION_V1_0,
    })
  }

  /// Raise the tracked DNGBackwardVersion to `version` because `feature` is used.
  /// Fails if `version` exceeds the allowed maximum backward version.
  fn update_backward_version(&mut self, version: DngVersionNumber, feature: &str) -> Result<()> {
    if version <= self.dng_backward_version {
      return Ok(());
    }
    if version > self.max_backward_version {
      return Err(DngError::General(format!(
        "DNG feature '{}' requires DNGBackwardVersion {}, but maximum allowed is {}",
        feature, version, self.max_backward_version
      )));
    }
    debug!(
      "DNGBackwardVersion raised from {} to {} by feature: {}",
      self.dng_backward_version, version, feature
    );
    self.dng_backward_version = version;
    Ok(())
  }

  /// Check if `feature`, which requires DNGBackwardVersion `version`, can be used
  /// within the allowed maximum backward version. The tracked version is not changed.
  fn backward_version_allows(&self, version: DngVersionNumber, feature: &str) -> bool {
    let allowed = version <= self.max_backward_version;
    if !allowed {
      debug!(
        "DNG feature '{}' requires DNGBackwardVersion {}, but maximum allowed is {}",
        feature, version, self.max_backward_version
      );
    }
    allowed
  }

  pub fn as_shot_neutral(&mut self, wb: impl AsRef<[Rational]>) {
    // Only write tag if wb is valid
    if wb.as_ref()[0].n != 0 {
      self.root_ifd.add_tag(DngTag::AsShotNeutral, wb.as_ref());
    }
  }

  pub fn color_matrix(&mut self, slot: usize, illu: Illuminant, matrix: impl AsRef<[SRational]>) {
    match slot {
      1 => {
        self.root_ifd.add_tag(DngTag::CalibrationIlluminant1, u16::from(illu));
        self.root_ifd.add_tag(DngTag::ColorMatrix1, matrix.as_ref());
      }
      2 => {
        self.root_ifd.add_tag(DngTag::CalibrationIlluminant2, u16::from(illu));
        self.root_ifd.add_tag(DngTag::ColorMatrix2, matrix.as_ref());
      }
      _ => todo!(),
    }
  }

  /// Add the ReductionMatrix for the illuminant of color matrix `slot`
  pub fn reduction_matrix(&mut self, slot: usize, matrix: impl AsRef<[SRational]>) {
    match slot {
      1 => self.root_ifd.add_tag(DngTag::ReductionMatrix1, matrix.as_ref()),
      2 => self.root_ifd.add_tag(DngTag::ReductionMatrix2, matrix.as_ref()),
      _ => todo!(),
    }
  }

  pub fn load_metadata(&mut self, metadata: &RawMetadata) -> Result<()> {
    metadata.write_exif_tags(&mut self.dng, &mut self.root_ifd, &mut self.exif_ifd)?;

    // DNG has a lens info tag that is identical to the LensSpec tag in EXIF IFD
    transfer_entry(&mut self.root_ifd, DngTag::LensInfo, &metadata.exif.lens_spec)?;

    if let Some(id) = &metadata.unique_image_id {
      self.root_ifd.add_tag(DngTag::RawDataUniqueID, id.to_le_bytes());
    }
    Ok(())
  }

  pub fn xpacket(&mut self, xpacket: impl AsRef<[u8]>) -> Result<()> {
    self.root_ifd.add_tag(ExifTag::ApplicationNotes, xpacket.as_ref());

    Ok(())
  }

  pub fn load_base_tags(&mut self, rawimage: &RawImage) -> Result<()> {
    self.root_ifd.add_tag(TiffCommonTag::Make, rawimage.clean_make.as_str());
    self.root_ifd.add_tag(TiffCommonTag::Model, rawimage.clean_model.as_str());
    let uq_model = format!("{} {}", rawimage.clean_make, rawimage.clean_model);
    self.root_ifd.add_tag(DngTag::UniqueCameraModel, uq_model.as_str());
    Ok(())
  }

  pub fn close(mut self) -> Result<()> {
    if !self.exif_ifd.is_empty() {
      let exif_ifd_offset = self.exif_ifd.build(&mut self.dng)?;
      self.root_ifd.add_tag(TiffCommonTag::ExifIFDPointer, exif_ifd_offset);
    }

    // Add SubIFDs
    if !self.subs.is_empty() {
      self.root_ifd.add_tag(TiffCommonTag::SubIFDs, &self.subs);
    }

    self.root_ifd.add_tag(DngTag::DNGBackwardVersion, self.dng_backward_version);

    self.dng.build(self.root_ifd)?;
    Ok(())
  }

  pub fn original_file(&mut self, original: &OriginalCompressed, filename: impl AsRef<str>) -> Result<()> {
    let mut buf = std::io::Cursor::new(Vec::new());
    let digest = original.write_to_stream(&mut buf)?;

    self.root_ifd.add_tag_undefined(DngTag::OriginalRawFileData, buf.into_inner());
    self.root_ifd.add_tag(DngTag::OriginalRawFileName, filename.as_ref());
    self.root_ifd.add_tag(DngTag::OriginalRawFileDigest, <[u8; 16]>::from(digest));
    Ok(())
  }

  pub fn subframe(&mut self, id: u32) -> SubFrameWriter<'_, B> {
    SubFrameWriter::new(self, id, false)
  }

  pub fn subframe_on_root(&mut self, id: u32) -> SubFrameWriter<'_, B> {
    SubFrameWriter::new(self, id, true)
  }

  /// Write thumbnail image into DNG
  pub fn thumbnail(&mut self, img: &DynamicImage) -> Result<()> {
    let thumb_img = img.resize(240, 120, FilterType::Nearest).to_rgb8();
    self.root_ifd.add_tag(TiffCommonTag::NewSubFileType, 1_u32);
    self.root_ifd.add_tag(TiffCommonTag::ImageWidth, thumb_img.width() as u32);
    self.root_ifd.add_tag(TiffCommonTag::ImageLength, thumb_img.height() as u32);
    self.root_ifd.add_tag(TiffCommonTag::Compression, CompressionMethod::None);
    self.root_ifd.add_tag(TiffCommonTag::BitsPerSample, [8_u16, 8, 8]);
    self.root_ifd.add_tag(TiffCommonTag::SampleFormat, [1_u16, 1, 1]);
    self.root_ifd.add_tag(TiffCommonTag::PhotometricInt, PhotometricInterpretation::RGB);
    self.root_ifd.add_tag(TiffCommonTag::SamplesPerPixel, 3_u16);
    //ifd.add_tag(TiffRootTag::XResolution, Rational { n: 1, d: 1 })?;
    //ifd.add_tag(TiffRootTag::YResolution, Rational { n: 1, d: 1 })?;
    //ifd.add_tag(TiffRootTag::ResolutionUnit, ResolutionUnit::None.to_u16())?;

    let offset = self.dng.write_data(&thumb_img)?;

    self.root_ifd.add_tag(TiffCommonTag::StripOffsets, offset);
    self.root_ifd.add_tag(TiffCommonTag::StripByteCounts, thumb_img.len() as u32);
    self.root_ifd.add_tag(TiffCommonTag::RowsPerStrip, thumb_img.height() as u32);

    Ok(())
  }

  pub fn root_ifd(&mut self) -> &DirectoryWriter {
    &self.root_ifd
  }

  pub fn root_ifd_mut(&mut self) -> &mut DirectoryWriter {
    &mut self.root_ifd
  }

  pub fn exif_ifd(&mut self) -> &DirectoryWriter {
    &self.exif_ifd
  }

  pub fn exif_ifd_mut(&mut self) -> &mut DirectoryWriter {
    &mut self.exif_ifd
  }
}

/// Convert floating point image data to 16 bit integer data
///
/// The highest white level is scaled to the full u16 range, sample values,
/// white levels and black levels are scaled by the same factor.
fn float_to_integer(rawimage: &mut RawImage) {
  let max_whitelevel = rawimage.whitelevel.0.iter().copied().max().unwrap_or(1).max(1) as f32;
  let scale = |v: f32| v / max_whitelevel * u16::MAX as f32;
  if let RawImageData::Float(data) = &rawimage.data {
    rawimage.data = RawImageData::Integer(data.iter().map(|p| scale(*p) as u16).collect());
  }
  rawimage.whitelevel.0.iter_mut().for_each(|x| *x = scale(*x as f32).round() as u32);
  rawimage.blacklevel.levels.iter_mut().for_each(|x| {
    let level = scale(x.as_f32());
    *x = if level.fract() == 0.0 {
      Rational::new(level as u32, 1)
    } else {
      Rational::new_f32(level, 1000)
    };
  });
  rawimage.bps = 16; // Reset bps as integers are scaled to u16 range.
}

/// Slope at the origin of the encoding curve for 8 bit lossy compressed data
const LOSSY_CURVE_SLOPE: f64 = 1.0 / 128.0;

/// LinearizationTable for 8 bit lossy compressed data
///
/// Maps code y (scaled to 0.0 .. 1.0) to the linear value x = s·y + (1−s)·y³,
/// scaled to the 16 bit range.
fn lossy_linearization_table() -> Vec<u16> {
  (0..=u8::MAX)
    .map(|code| {
      let y = code as f64 / u8::MAX as f64;
      let x = LOSSY_CURVE_SLOPE * y + (1.0 - LOSSY_CURVE_SLOPE) * y.powi(3);
      (x * u16::MAX as f64).round() as u16
    })
    .collect()
}

/// Convert image data to 8 bit codes for lossy JPEG compression
///
/// Lossy JPEG requires 8 bit LinearRaw data. The data is scaled to linear
/// 0.0 .. 1.0 and each sample is encoded to the nearest code of
/// [`lossy_linearization_table`], which is stored as LinearizationTable.
fn to_lossy_8bit(rawimage: &mut RawImage) -> Result<()> {
  match rawimage.photometric {
    RawPhotometricInterpretation::Cfa(_) => {
      return Err(DngError::General("Lossy JPEG compression requires demosaiced (LinearRaw) image data".into()));
    }
    // A single plane LinearRaw image is the same as BlackIsZero
    RawPhotometricInterpretation::BlackIsZero => rawimage.photometric = RawPhotometricInterpretation::LinearRaw,
    RawPhotometricInterpretation::LinearRaw => {}
  }
  if rawimage.cpp != 1 && rawimage.cpp != 3 {
    return Err(DngError::General(format!(
      "Lossy JPEG compression requires 1 or 3 samples per pixel, got {}",
      rawimage.cpp
    )));
  }
  let normalized =
    matches!(rawimage.data, RawImageData::Float(_)) && rawimage.blacklevel.levels.iter().all(|x| x.n == 0) && rawimage.whitelevel.0.iter().all(|x| *x == 1);
  if !normalized {
    rawimage
      .apply_scaling()
      .map_err(|err| DngError::General(format!("Scaling for lossy compression failed: {}", err)))?;
  }
  let table = lossy_linearization_table();
  // Decision levels between adjacent codes, so each sample gets the nearest code
  let levels: Vec<f32> = table.windows(2).map(|pair| (pair[0] as f32 + pair[1] as f32) / 2.0 / u16::MAX as f32).collect();
  let codes: Vec<u16> = rawimage
    .data
    .as_f32()
    .par_iter()
    .map(|v| levels.partition_point(|level| level < v) as u16)
    .collect();
  rawimage.data = RawImageData::Integer(codes);
  rawimage.linearization_table = Some(table);
  rawimage.blacklevel = BlackLevel::zero(1, 1, rawimage.cpp);
  rawimage.whitelevel = WhiteLevel::new_bits(16, rawimage.cpp);
  rawimage.bps = 8;
  Ok(())
}

/// Slope at the origin of the encoding curve for 16 bit lossy JPEG XL data
const JXL_LOSSY_CURVE_SLOPE: f64 = 1.0 / 16.0;

/// Interleave factors (rows, columns) for JPEG XL compression of CFA data
///
/// CFA data is stored as sub-images of the single colours of the pattern,
/// see DNG RowInterleaveFactor and ColumnInterleaveFactor.
fn jxl_interleave(rawimage: &RawImage) -> Option<(usize, usize)> {
  match &rawimage.photometric {
    RawPhotometricInterpretation::Cfa(config) if config.cfa.height < rawimage.height && config.cfa.width < rawimage.width => {
      Some((config.cfa.height, config.cfa.width))
    }
    _ => None,
  }
}

/// Check if floating point data is stored as half floats with JPEG XL
///
/// Floats derived from integer raw data are stored exactly as 16 bit integers,
/// as are floats out of the half float range (e.g. integers above 16 bits).
fn jxl_stores_float(rawimage: &RawImage) -> bool {
  matches!(rawimage.data, RawImageData::Float(_)) && !rawimage.scaled_from_integer && rawimage.whitelevel.0.iter().all(|x| *x as f32 <= half::f16::MAX.to_f32())
}

/// Length of JPEG XL tiles of about 1024 pixels, which split `len` evenly
fn jxl_tile_size(len: usize) -> usize {
  let count = len.div_ceil(1024).max(1);
  len.div_ceil(count).next_multiple_of(16)
}

/// Encode integer data for lossy JPEG XL compression
///
/// The linear values (after an existing LinearizationTable) are encoded to 16 bit
/// codes by x = s·y + (1−s)·y³. The new
/// LinearizationTable maps the codes back to the linear values, so black and white
/// levels keep their units and values below the black level are kept.
fn to_jxl_lossy_16bit(rawimage: &mut RawImage) {
  let table = rawimage.linearization_table.take();
  let RawImageData::Integer(data) = &rawimage.data else {
    rawimage.linearization_table = table;
    return;
  };
  let linear = |v: u16| -> u16 {
    match &table {
      Some(table) if !table.is_empty() => table[(v as usize).min(table.len() - 1)],
      _ => v,
    }
  };
  let max_input = data.par_iter().copied().max().unwrap_or(0);
  let max_linear = (0..=max_input).map(linear).max().unwrap_or(0) as u32;
  let white = rawimage.whitelevel.0.iter().copied().max().unwrap_or(0);
  let d = white.max(max_linear).min(u16::MAX as u32) as f64;
  let s = JXL_LOSSY_CURVE_SLOPE;
  let codes: Vec<u16> = (0..=u16::MAX)
    .map(|c| {
      let y = c as f64 / u16::MAX as f64;
      (d * (s * y + (1.0 - s) * y.powi(3))).round() as u16
    })
    .collect();
  // Decision levels between adjacent codes, so each value gets the nearest code
  let levels: Vec<f64> = codes.windows(2).map(|pair| (pair[0] as f64 + pair[1] as f64) / 2.0).collect();
  let lut: Vec<u16> = (0..=max_input)
    .map(|v| levels.partition_point(|level| *level < linear(v) as f64) as u16)
    .collect();
  rawimage.data = RawImageData::Integer(data.par_iter().map(|v| lut[*v as usize]).collect());
  rawimage.linearization_table = Some(codes);
  rawimage.bps = 16;
}

/// Pad CFA data on the right and bottom, so each interleave field of lossy JPEG XL
/// data is a multiple of the tile size and tile seams don't cross colour fields.
///
/// The padding repeats the last pixels of the same CFA phase. It is outside of
/// the active area written by `write_rawimage`.
fn pad_jxl_fields(rawimage: &mut RawImage) {
  let Some((rows, cols)) = jxl_interleave(rawimage) else {
    return;
  };
  let (width, height, cpp) = (rawimage.width, rawimage.height, rawimage.cpp);
  let padded_len = |len: usize, factor: usize| {
    let field = len.div_ceil(factor);
    field.next_multiple_of(jxl_tile_size(field)) * factor
  };
  let (padded_width, padded_height) = (padded_len(width, cols), padded_len(height, rows));
  if (padded_width, padded_height) == (width, height) {
    return;
  }
  // Source indices within the image for the padded indices, with the same phase
  let sources = |padded_len: usize, len: usize, factor: usize| -> Vec<usize> {
    (0..padded_len)
      .map(|i| if i < len { i } else { i - factor * ((i - len) / factor + 1) })
      .collect()
  };
  let (xs, ys) = (sources(padded_width, width, cols), sources(padded_height, height, rows));
  fn pad<T: Copy>(data: &[T], width: usize, cpp: usize, xs: &[usize], ys: &[usize]) -> Vec<T> {
    let mut padded = Vec::with_capacity(xs.len() * ys.len() * cpp);
    for y in ys {
      let row = &data[y * width * cpp..][..width * cpp];
      for x in xs {
        padded.extend_from_slice(&row[x * cpp..][..cpp]);
      }
    }
    padded
  }
  rawimage.data = match &rawimage.data {
    RawImageData::Integer(data) => RawImageData::Integer(pad(data, width, cpp, &xs, &ys)),
    RawImageData::Float(data) => RawImageData::Float(pad(data, width, cpp, &xs, &ys)),
  };
  debug!("Pad {}x{} to {}x{} for JPEG XL fields", width, height, padded_width, padded_height);
  rawimage.width = padded_width;
  rawimage.height = padded_height;
}

/// DNG requires the WB values to be the reciprocal
fn wbcoeff_to_tiff_value(rawimage: &RawImage) -> Vec<Rational> {
  let wb = &rawimage.wb_coeffs;
  match &rawimage.photometric {
    RawPhotometricInterpretation::BlackIsZero => {
      vec![Rational::new(1, 1)] // TODO: is this useful?
    }
    RawPhotometricInterpretation::Cfa(config) => {
      assert!([1, 3, 4].contains(&config.cfa.unique_colors()));

      let mut values = Vec::with_capacity(4);

      values.push(Rational::new_f32(1.0 / wb[0], 100000));
      values.push(Rational::new_f32(1.0 / wb[1], 100000));
      values.push(Rational::new_f32(1.0 / wb[2], 100000));

      if config.cfa.unique_colors() == 4 {
        values.push(Rational::new_f32(1.0 / wb[3], 100000));
      }
      values
    }
    RawPhotometricInterpretation::LinearRaw => {
      //assert_eq!(rawimage.cpp, 3);

      match rawimage.cpp {
        1 => {
          vec![Rational::new(1, 1)]
        }
        3 | 4 => wb[..rawimage.cpp].iter().map(|x| Rational::new_f32(1.0 / x, 100000)).collect(),
        _ => todo!(),
      }
    }
  }
}

fn matrix_to_tiff_value(xyz_to_cam: &[f32], d: i32) -> Vec<SRational> {
  xyz_to_cam.iter().map(|a| SRational::new((a * d as f32) as i32, d)).collect()
}

/// Compress RAW image with LJPEG-92
///
/// Data is split into multiple tiles, each tile is compressed seperately
///
/// Predictor mode 4,5,6,7 is best for images where two images
/// lines are merged, because then the image bayer pattern is:
/// RGRGGBGB
/// RGRGGBGB
/// Instead of the default:
/// RGRG
/// GBGB
/// RGRG
/// GBGB
fn dng_put_raw_ljpeg<W>(subframe: &mut SubFrameWriter<W>, rawimage: &RawImage, predictor: u8) -> Result<()>
where
  W: Seek + Write,
{
  let tile_w = 256 & !0b111; // ensure div 16
  let tile_h = 256 & !0b111;

  let lj92_data = match rawimage.data {
    RawImageData::Integer(ref data) => {
      // Only merge two lines into one for higher predictors, if image is CFA

      let (j_width, j_height, components, realign) = match &rawimage.photometric {
        RawPhotometricInterpretation::BlackIsZero => {
          assert_eq!(rawimage.cpp, 1);
          (tile_w, tile_h, 1, 1)
        }
        RawPhotometricInterpretation::Cfa(config) => {
          assert_eq!(rawimage.cpp, 1);
          let realign = if (4..=7).contains(&predictor) && config.cfa.width == 2 && config.cfa.height == 2 {
            2
          } else {
            1
          };
          (tile_w / 2, tile_h, 2, realign)
        }
        RawPhotometricInterpretation::LinearRaw => {
          (tile_w, tile_h, rawimage.cpp, 1) /* RGB LinearRaw */
        }
      };

      debug!("LJPEG compression: bit depth: {}", rawimage.bps);

      // Build and compress each tile in one parallel pass: tile materialization
      // overlaps with LJPEG encoding instead of pre-collecting `Vec<Vec<u16>>`
      // for the whole image. Peak memory drops from O(image) uncompressed
      // staging to roughly O(threads × tile).
      let tiler = ImageTiler::new(data, rawimage.width, rawimage.height, rawimage.cpp, tile_w, tile_h);
      let n_tiles = tiler.tile_count();
      let bps = rawimage.bps as u8;
      let tiles_compr: Vec<Vec<u8>> = (0..n_tiles)
        .into_par_iter()
        .map(|idx| {
          let tile = tiler.build_tile(idx);
          let state = LjpegCompressor::new(&tile, j_width * realign, j_height / realign, components, bps, predictor, 0, 0)
            .map_err(|e| DngError::General(format!("LJPEG compressor init failed: {}", e)))?;
          state.encode().map_err(|e| DngError::General(format!("LJPEG encoding failed: {}", e)))
        })
        .collect::<Result<Vec<_>>>()?;
      tiles_compr
    }
    RawImageData::Float(ref _data) => {
      panic!("invalid format");
    }
  };

  let mut tile_offsets: Vec<u32> = Vec::new();
  let mut tile_sizes: Vec<u32> = Vec::new();

  for tile in lj92_data.iter() {
    let offs = subframe.writer.dng.write_data(tile)?;
    tile_offsets.push(offs);
    tile_sizes.push((tile.len() * size_of::<u8>()) as u32);
  }

  subframe
    .ifd_mut()
    .add_tag(TiffCommonTag::BitsPerSample, &vec![rawimage.bps as u16; rawimage.cpp]);
  subframe.ifd_mut().add_tag(TiffCommonTag::SampleFormat, &vec![1_u16; rawimage.cpp]);
  subframe.ifd_mut().add_tag(TiffCommonTag::TileOffsets, &tile_offsets);
  subframe.ifd_mut().add_tag(TiffCommonTag::TileByteCounts, &tile_sizes);
  subframe.ifd_mut().add_tag(TiffCommonTag::TileWidth, tile_w as u16);
  subframe.ifd_mut().add_tag(TiffCommonTag::TileLength, tile_h as u16);

  Ok(())
}

/// Compress image with lossy JPEG
///
/// Data is split into multiple tiles, each tile is compressed seperately.
/// The image must hold 8 bit samples with one (grayscale) or three (RGB) planes.
fn dng_put_raw_lossy_jpeg<W>(subframe: &mut SubFrameWriter<W>, rawimage: &RawImage, quality: u8) -> Result<()>
where
  W: Seek + Write,
{
  let tile_w = 256;
  let tile_h = 256;

  let color_type = match rawimage.cpp {
    1 => ExtendedColorType::L8,
    3 => ExtendedColorType::Rgb8,
    cpp => {
      return Err(DngError::General(format!(
        "Lossy JPEG compression requires 1 or 3 samples per pixel, got {}",
        cpp
      )));
    }
  };
  let RawImageData::Integer(ref data) = rawimage.data else {
    return Err(DngError::General("Lossy JPEG compression requires 8 bit integer data".into()));
  };

  let tiler = ImageTiler::new(data, rawimage.width, rawimage.height, rawimage.cpp, tile_w, tile_h);
  let jpeg_data: Vec<Vec<u8>> = (0..tiler.tile_count())
    .into_par_iter()
    .map(|idx| {
      let tile: Vec<u8> = tiler.build_tile(idx).into_iter().map(|v| v as u8).collect();
      let mut buf = Vec::new();
      JpegEncoder::new_with_quality(&mut buf, quality.clamp(1, 100))
        .encode(&tile, tile_w as u32, tile_h as u32, color_type)
        .map_err(|e| DngError::General(format!("Lossy JPEG encoding failed: {}", e)))?;
      Ok(buf)
    })
    .collect::<Result<Vec<_>>>()?;

  let mut tile_offsets: Vec<u32> = Vec::new();
  let mut tile_sizes: Vec<u32> = Vec::new();

  for tile in jpeg_data.iter() {
    let offs = subframe.writer.dng.write_data(tile)?;
    tile_offsets.push(offs);
    tile_sizes.push(tile.len() as u32);
  }

  subframe.ifd_mut().add_tag(TiffCommonTag::BitsPerSample, &vec![8_u16; rawimage.cpp]);
  subframe.ifd_mut().add_tag(TiffCommonTag::SampleFormat, &vec![1_u16; rawimage.cpp]);
  subframe.ifd_mut().add_tag(TiffCommonTag::TileOffsets, &tile_offsets);
  subframe.ifd_mut().add_tag(TiffCommonTag::TileByteCounts, &tile_sizes);
  subframe.ifd_mut().add_tag(TiffCommonTag::TileWidth, tile_w as u16);
  subframe.ifd_mut().add_tag(TiffCommonTag::TileLength, tile_h as u16);

  Ok(())
}

/// Compress image with JPEG XL
///
/// CFA data is interleaved into sub-images of the single colours. Data is split
/// into tiles, each tile is compressed separately. Integer data is stored with
/// 16 bits per sample, floating point data as half floats.
fn dng_put_raw_jxl<W>(subframe: &mut SubFrameWriter<W>, rawimage: &RawImage, params: &JxlParams) -> Result<()>
where
  W: Seek + Write,
{
  let cpp = rawimage.cpp;
  if cpp != 1 && cpp != 3 {
    return Err(DngError::General(format!("JPEG XL compression requires 1 or 3 samples per pixel, got {}", cpp)));
  }
  let interleave = jxl_interleave(rawimage);
  let lossless = params.is_lossless();
  // Lossy CFA data is padded, so tiles of the field size cover whole fields
  let (tile_w, tile_h) = match interleave {
    Some((rows, cols)) if !lossless => (jxl_tile_size(rawimage.width / cols), jxl_tile_size(rawimage.height / rows)),
    _ => (jxl_tile_size(rawimage.width), jxl_tile_size(rawimage.height)),
  };
  let settings = TileSettings {
    distance: params.distance,
    effort: params.effort as u32,
    decode_speed: params.decode_speed as u32,
    lossy_transfer: if jxl_stores_float(rawimage) {
      LossyTransfer::Linear
    } else {
      LossyTransfer::Srgb
    },
  };
  // Samples in stored order, as integers or half float bit patterns
  let (samples, float) = match &rawimage.data {
    RawImageData::Integer(data) => (data.clone(), false),
    RawImageData::Float(data) => (data.par_iter().map(|v| half::f16::from_f32(*v).to_bits()).collect(), true),
  };
  let mut pixbuf = Pix2D::new_with(samples, rawimage.width * cpp, rawimage.height);
  if let Some((rows, cols)) = interleave {
    pixbuf = interleave_cols(&interleave_rows(&pixbuf, rows), cpp, cols);
  }
  debug!(
    "JPEG XL compression: distance {}, effort {}, {}, tiles {}x{}, interleave {:?}",
    params.distance,
    params.effort,
    if float { "half float" } else { "16 bit integer" },
    tile_w,
    tile_h,
    interleave
  );

  let tiler = ImageTiler::new(&pixbuf.data, rawimage.width, rawimage.height, cpp, tile_w, tile_h);
  let jxl_data: Vec<Vec<u8>> = (0..tiler.tile_count())
    .into_par_iter()
    .map(|idx| {
      let tile = tiler.build_tile(idx);
      let samples = if float { TileSamples::F16(&tile) } else { TileSamples::U16(&tile) };
      jpegxl::encode_tile(samples, tile_w, tile_h, cpp, &settings).map_err(DngError::General)
    })
    .collect::<Result<Vec<_>>>()?;

  let mut tile_offsets: Vec<u32> = Vec::new();
  let mut tile_sizes: Vec<u32> = Vec::new();

  for tile in jxl_data.iter() {
    let offs = subframe.writer.dng.write_data(tile)?;
    tile_offsets.push(offs);
    tile_sizes.push(tile.len() as u32);
  }

  subframe.ifd_mut().add_tag(TiffCommonTag::BitsPerSample, &vec![16_u16; cpp]);
  subframe
    .ifd_mut()
    .add_tag(TiffCommonTag::SampleFormat, &vec![if float { 3_u16 } else { 1_u16 }; cpp]);
  subframe.ifd_mut().add_tag(TiffCommonTag::TileOffsets, &tile_offsets);
  subframe.ifd_mut().add_tag(TiffCommonTag::TileByteCounts, &tile_sizes);
  subframe.ifd_mut().add_tag(TiffCommonTag::TileWidth, tile_w as u32);
  subframe.ifd_mut().add_tag(TiffCommonTag::TileLength, tile_h as u32);
  if let Some((rows, cols)) = interleave {
    subframe.ifd_mut().add_tag(DngTag::RowInterleaveFactor, rows as u16);
    subframe.ifd_mut().add_tag(DngTag::ColumnInterleaveFactor, cols as u16);
  }
  subframe.ifd_mut().add_tag(DngTag::JXLDistance, params.distance);
  subframe.ifd_mut().add_tag(DngTag::JXLEffort, params.effort as u32);
  if !lossless {
    subframe.ifd_mut().add_tag(DngTag::JXLDecodeSpeed, params.decode_speed as u32);
  }

  Ok(())
}

/// Write RAW uncompressed into DNG
///
/// This uses unsigned 16 bit values for storage
/// Data is split into multiple strips
fn dng_put_raw_uncompressed<W>(subframe: &mut SubFrameWriter<W>, rawimage: &RawImage) -> Result<()>
where
  W: Write + Seek,
{
  let mut strip_offsets: Vec<u32> = Vec::new();
  let mut strip_sizes: Vec<u32> = Vec::new();
  let mut strip_rows: Vec<u32> = Vec::new();

  let rows_per_strip = if rawimage.height > rawler_dng_multistrip_threshold().unwrap_or(100) {
    rawler_dng_rows_per_strip().unwrap_or(256)
  } else {
    rawimage.height
  };

  match rawimage.data {
    RawImageData::Integer(ref data) => {
      for strip in data.chunks(rows_per_strip * rawimage.width * rawimage.cpp) {
        let offset = subframe.writer.dng.write_data_u16_le(strip)?;
        strip_offsets.push(offset);
        strip_sizes.push(std::mem::size_of_val(strip) as u32);
        strip_rows.push((strip.len() / (rawimage.width * rawimage.cpp)) as u32);
      }
      subframe.ifd_mut().add_tag(TiffCommonTag::SampleFormat, &vec![1_u16; rawimage.cpp]); // Unsigned Integer
      subframe.ifd_mut().add_tag(TiffCommonTag::BitsPerSample, &vec![16_u16; rawimage.cpp]);
    }
    RawImageData::Float(ref data) => {
      subframe.writer.update_backward_version(DNG_VERSION_V1_4, "Floating point image data")?;
      for strip in data.chunks(rows_per_strip * rawimage.width * rawimage.cpp) {
        let offset = subframe.writer.dng.write_data_f32_le(strip)?;
        strip_offsets.push(offset);
        strip_sizes.push(std::mem::size_of_val(strip) as u32);
        strip_rows.push((strip.len() / (rawimage.width * rawimage.cpp)) as u32);
      }
      subframe.ifd_mut().add_tag(TiffCommonTag::SampleFormat, &vec![3_u16; rawimage.cpp]); // IEEE Float
      subframe.ifd_mut().add_tag(TiffCommonTag::BitsPerSample, &vec![32_u16; rawimage.cpp]);
    }
  };
  subframe.ifd_mut().add_tag(TiffCommonTag::StripOffsets, &strip_offsets);
  subframe.ifd_mut().add_tag(TiffCommonTag::StripByteCounts, &strip_sizes);
  subframe.ifd_mut().add_tag(TiffCommonTag::RowsPerStrip, &strip_rows);

  Ok(())
}

#[cfg(test)]
mod tests {

  use std::io::Cursor;

  use crate::dng::{DNG_VERSION_V1_4, DNG_VERSION_V1_6};
  use crate::dng::{DNG_VERSION_V1_7, DNG_VERSION_V1_7_1};

  use super::*;

  #[test]
  fn build_empty_dng() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut buf = Cursor::new(Vec::new());
    let mut dng = DngWriter::new(&mut buf, DNG_VERSION_V1_4)?;
    dng.root_ifd_mut().add_tag(TiffCommonTag::Artist, "Test");
    dng.close()?;
    #[cfg(target_endian = "little")]
    let expected_output = [
      73, 73, 42, 0, 36, 0, 0, 0, 1, 0, 0, 144, 7, 0, 4, 0, 0, 0, 48, 50, 50, 48, 0, 0, 0, 0, 0, 0, 84, 101, 115, 116, 0, 0, 0, 0, 4, 0, 59, 1, 2, 0, 5, 0, 0,
      0, 28, 0, 0, 0, 105, 135, 4, 0, 1, 0, 0, 0, 8, 0, 0, 0, 18, 198, 1, 0, 4, 0, 0, 0, 1, 7, 1, 0, 19, 198, 1, 0, 4, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0,
    ];
    #[cfg(not(target_endian = "little"))]
    let expected_output = [
      77, 77, 0, 42, 0, 0, 0, 36, 0, 1, 144, 0, 0, 7, 0, 0, 0, 4, 48, 50, 50, 48, 0, 0, 0, 0, 0, 0, 84, 101, 115, 116, 0, 0, 0, 0, 0, 4, 1, 59, 0, 2, 0, 0, 0,
      5, 0, 0, 0, 28, 135, 105, 0, 4, 0, 0, 0, 1, 0, 0, 0, 8, 198, 18, 0, 1, 0, 0, 0, 4, 0, 1, 7, 1, 198, 19, 0, 1, 0, 0, 0, 4, 0, 0, 0, 1, 0, 0, 0, 0,
    ];
    assert_eq!(expected_output, buf.into_inner().as_slice());
    Ok(())
  }

  /// Synthetic 16x16 CFA image with an optional active area
  fn synthetic_rawimage(active_area: Option<Rect>) -> RawImage {
    use crate::rawimage::CFAConfig;

    let mut camera = Camera::new();
    camera.cfa = CFA::new("RGGB");
    let pixels: Vec<u16> = (0..16 * 16).map(|i| (i % 4096) as u16).collect();
    let image = PixU16::new_with(pixels, 16, 16);
    let photometric = RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&camera));
    let mut rawimage = RawImage::new(
      camera,
      image,
      1,
      [1.0, 1.0, 1.0, f32::NAN],
      photometric,
      Some(BlackLevel::new(&[0_u32, 0, 0, 0], 2, 2, 1)),
      Some(WhiteLevel::new_bits(12, 1)),
      false,
    );
    rawimage.active_area = active_area;
    rawimage
  }

  /// Write `rawimage` with the given max backward version and return the DNG bytes
  fn write_synthetic_dng(rawimage: &RawImage, cropmode: CropMode, max_backward_version: DngVersionNumber) -> Result<Vec<u8>> {
    let mut buf = Cursor::new(Vec::new());
    let mut dng = DngWriter::new(&mut buf, max_backward_version)?;
    let mut frame = dng.subframe(0);
    frame.raw_image(
      rawimage,
      cropmode,
      DngCompression::Uncompressed,
      DngPhotometricConversion::Original,
      &CompressionParams::default(),
    )?;
    frame.finalize()?;
    dng.close()?;
    Ok(buf.into_inner())
  }

  /// Write `rawimage` with the given compression and return the DNG bytes
  fn write_synthetic_dng_compressed(rawimage: &RawImage, compression: DngCompression) -> Result<Vec<u8>> {
    let mut buf = Cursor::new(Vec::new());
    let mut dng = DngWriter::new(&mut buf, DNG_VERSION_V1_4)?;
    dng.load_base_tags(rawimage)?;
    let mut frame = dng.subframe(0);
    frame.raw_image(
      rawimage,
      CropMode::None,
      compression,
      DngPhotometricConversion::Original,
      &CompressionParams::default(),
    )?;
    frame.finalize()?;
    dng.close()?;
    Ok(buf.into_inner())
  }

  fn read_backward_version_and_active_area(data: &[u8]) -> std::result::Result<(DngVersionNumber, bool), Box<dyn std::error::Error>> {
    use crate::formats::tiff::{GenericTiffReader, reader::TiffReader};

    let tiff = GenericTiffReader::new_with_buffer(data, 0, 0, None)?;
    let backward_version = tiff
      .root_ifd()
      .get_entry(DngTag::DNGBackwardVersion)
      .ok_or("DNGBackwardVersion missing")?
      .value
      .get_data()
      .as_slice()
      .try_into()
      .map(DngVersionNumber)?;
    let has_active_area = !tiff.find_ifds_with_tag(DngTag::ActiveArea).is_empty();
    Ok((backward_version, has_active_area))
  }

  #[test]
  fn backward_version_active_area() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let rawimage = synthetic_rawimage(Some(Rect::new(Point::new(2, 2), Dim2::new(12, 12))));
    let data = write_synthetic_dng(&rawimage, CropMode::Best, DNG_VERSION_V1_4)?;
    let (backward_version, has_active_area) = read_backward_version_and_active_area(&data)?;
    assert_eq!(backward_version, DNG_VERSION_V1_1);
    assert!(has_active_area);
    Ok(())
  }

  #[test]
  fn backward_version_active_area_not_allowed() -> std::result::Result<(), Box<dyn std::error::Error>> {
    use crate::formats::tiff::{GenericTiffReader, reader::TiffReader};

    // ActiveArea is skipped, masked pixels are excluded by the default crop only
    let rawimage = synthetic_rawimage(Some(Rect::new(Point::new(2, 2), Dim2::new(12, 12))));
    let data = write_synthetic_dng(&rawimage, CropMode::Best, DNG_VERSION_V1_0)?;
    let (backward_version, has_active_area) = read_backward_version_and_active_area(&data)?;
    assert_eq!(backward_version, DNG_VERSION_V1_0);
    assert!(!has_active_area);

    let tiff = GenericTiffReader::new_with_buffer(&data, 0, 0, None)?;
    let raw_ifd = tiff.find_first_ifd_with_tag(DngTag::DefaultCropOrigin).ok_or("DefaultCropOrigin missing")?;
    let origin = raw_ifd.get_entry(DngTag::DefaultCropOrigin).ok_or("DefaultCropOrigin missing")?;
    let size = raw_ifd.get_entry(DngTag::DefaultCropSize).ok_or("DefaultCropSize missing")?;
    assert_eq!((origin.value.force_u16(0), origin.value.force_u16(1)), (2, 2));
    assert_eq!((size.value.force_u16(0), size.value.force_u16(1)), (12, 12));
    Ok(())
  }

  #[test]
  fn backward_version_full_size_active_area() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Active area covers the full image, so no masked pixels are stored
    let rawimage = synthetic_rawimage(Some(Rect::new(Point::new(0, 0), Dim2::new(16, 16))));
    let data = write_synthetic_dng(&rawimage, CropMode::Best, DNG_VERSION_V1_0)?;
    let (backward_version, has_active_area) = read_backward_version_and_active_area(&data)?;
    assert_eq!(backward_version, DNG_VERSION_V1_0);
    assert!(!has_active_area);

    // CropMode::None ignores the active area
    let rawimage = synthetic_rawimage(Some(Rect::new(Point::new(2, 2), Dim2::new(12, 12))));
    let data = write_synthetic_dng(&rawimage, CropMode::None, DNG_VERSION_V1_0)?;
    let (backward_version, has_active_area) = read_backward_version_and_active_area(&data)?;
    assert_eq!(backward_version, DNG_VERSION_V1_0);
    assert!(!has_active_area);
    Ok(())
  }

  /// Synthetic 16x16 CFA image with floating point data
  fn synthetic_rawimage_f32() -> RawImage {
    let mut rawimage = synthetic_rawimage(None);
    rawimage.data = RawImageData::Float((0..16 * 16).map(|i| i as f32 / 256.0).collect());
    rawimage.whitelevel = WhiteLevel::new(vec![1]);
    rawimage
  }

  /// Write `rawimage` uncompressed or lossless and return DNGBackwardVersion, SampleFormat and BitsPerSample
  fn write_float_dng(
    rawimage: &RawImage,
    compression: DngCompression,
    max_backward_version: DngVersionNumber,
  ) -> std::result::Result<(DngVersionNumber, u16, u16), Box<dyn std::error::Error>> {
    use crate::formats::tiff::{GenericTiffReader, reader::TiffReader};

    let mut buf = Cursor::new(Vec::new());
    let mut dng = DngWriter::new(&mut buf, max_backward_version)?;
    let mut frame = dng.subframe(0);
    frame.raw_image(
      rawimage,
      CropMode::None,
      compression,
      DngPhotometricConversion::Original,
      &CompressionParams::default(),
    )?;
    frame.finalize()?;
    dng.close()?;
    let data = buf.into_inner();

    let (backward_version, _) = read_backward_version_and_active_area(&data)?;
    let tiff = GenericTiffReader::new_with_buffer(&data, 0, 0, None)?;
    let raw_ifd = tiff.find_first_ifd_with_tag(TiffCommonTag::BitsPerSample).ok_or("raw IFD missing")?;
    let sample_format = raw_ifd.get_entry(TiffCommonTag::SampleFormat).map(|e| e.value.force_u16(0)).unwrap_or(1);
    let bps = raw_ifd
      .get_entry(TiffCommonTag::BitsPerSample)
      .ok_or("BitsPerSample missing")?
      .value
      .force_u16(0);
    Ok((backward_version, sample_format, bps))
  }

  #[test]
  fn backward_version_float_data() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let rawimage = synthetic_rawimage_f32();
    assert_eq!(
      write_float_dng(&rawimage, DngCompression::Uncompressed, DNG_VERSION_V1_4)?,
      (DNG_VERSION_V1_4, 3, 32)
    );
    Ok(())
  }

  #[test]
  fn backward_version_float_data_not_allowed() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Floats are converted to integer data
    let rawimage = synthetic_rawimage_f32();
    assert_eq!(
      write_float_dng(&rawimage, DngCompression::Uncompressed, DNG_VERSION_V1_1)?,
      (DNG_VERSION_V1_0, 1, 16)
    );
    Ok(())
  }

  #[test]
  fn backward_version_float_data_lossless() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Lossless always converts floats to integer data
    let rawimage = synthetic_rawimage_f32();
    let (backward_version, _, bps) = write_float_dng(&rawimage, DngCompression::Lossless, DNG_VERSION_V1_4)?;
    assert_eq!((backward_version, bps), (DNG_VERSION_V1_0, 16));
    Ok(())
  }

  #[test]
  fn float_to_integer_scales_by_whitelevel() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut rawimage = synthetic_rawimage(None);
    rawimage.data = RawImageData::Float((0..16 * 16).map(|i| [0.0, 256.0, 2048.0, 4095.0][i % 4]).collect());
    rawimage.whitelevel = WhiteLevel::new(vec![4095]);
    rawimage.blacklevel = BlackLevel::new(&[256_u32], 1, 1, 1);
    let data = write_synthetic_dng_compressed(&rawimage, DngCompression::Lossless)?;

    let source = crate::rawsource::RawSource::new_from_slice(&data);
    let decoded = crate::get_decoder(&source)?.raw_image(&source, &crate::decoders::RawDecodeParams::default(), false)?;
    assert_eq!(decoded.whitelevel.0, vec![65535]);
    // 256 / 4095 * 65535 = 4096.94
    assert!((decoded.blacklevel.levels[0].as_f32() - 4096.94).abs() < 0.01);
    match decoded.data {
      RawImageData::Integer(pixels) => assert_eq!(pixels[..4], [0, 4096, 32775, 65535]),
      RawImageData::Float(_) => panic!("lossless data must be integer"),
    }
    Ok(())
  }

  /// Write `rawimage` with DeltaPerRow opcodes in all opcode lists and return the
  /// deltas read from the DNG per list and whether the data is stored as floats
  fn write_deltas(mut rawimage: RawImage, compression: DngCompression) -> std::result::Result<([Vec<f32>; 3], bool), Box<dyn std::error::Error>> {
    use crate::dng::opcodes::{AreaSpec, DeltaPerRow, Opcode, OpcodeList};

    let area = AreaSpec {
      top: 0,
      left: 0,
      bottom: 16,
      right: 16,
      plane: 0,
      planes: 1,
      row_pitch: 8,
      col_pitch: 1,
    };
    let list = OpcodeList::new(vec![Opcode::DeltaPerRow(DeltaPerRow {
      version: 0x01030000,
      flags: 0,
      area,
      deltas: vec![1000.0, -2.0],
    })]);
    rawimage.dng_opcodes_1 = Some(list.clone());
    rawimage.dng_opcodes_2 = Some(list.clone());
    rawimage.dng_opcodes_3 = Some(list);
    let data = write_synthetic_dng_compressed(&rawimage, compression)?;

    let source = crate::rawsource::RawSource::new_from_slice(&data);
    let decoded = crate::get_decoder(&source)?.raw_image(&source, &crate::decoders::RawDecodeParams::default(), false)?;
    let deltas = |list: Option<&OpcodeList>| match list.map(OpcodeList::opcodes) {
      Some([Opcode::DeltaPerRow(op)]) => Ok(op.deltas.clone()),
      other => Err(format!("unexpected opcode list: {:?}", other)),
    };
    Ok((
      [
        deltas(decoded.dng_opcodes_1.as_ref())?,
        deltas(decoded.dng_opcodes_2.as_ref())?,
        deltas(decoded.dng_opcodes_3.as_ref())?,
      ],
      matches!(decoded.data, RawImageData::Float(_)),
    ))
  }

  #[test]
  fn deltas_follow_stored_sample_format() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let deltas = vec![1000.0, -2.0];
    let to_float = vec![1000.0 * (1.0 / 65535.0), -2.0 * (1.0 / 65535.0)];
    let to_integer = vec![1000.0 * 65535.0, -2.0 * 65535.0];
    let mut scaled = synthetic_rawimage_f32();
    scaled.scaled_from_integer = true;
    // OpcodeList1 is unchanged, OpcodeList2 and OpcodeList3 are converted
    let expect = |stage23: &Vec<f32>, float: bool| ([deltas.clone(), stage23.clone(), stage23.clone()], float);
    // Data scaled from 16 bit integers
    assert_eq!(write_deltas(scaled.clone(), DngCompression::Uncompressed)?, expect(&to_float, true));
    assert_eq!(write_deltas(scaled, DngCompression::Lossless)?, expect(&deltas, false));
    // Floating point raw data
    assert_eq!(write_deltas(synthetic_rawimage_f32(), DngCompression::Uncompressed)?, expect(&deltas, true));
    assert_eq!(write_deltas(synthetic_rawimage_f32(), DngCompression::Lossless)?, expect(&to_integer, false));
    // 16 bit integer raw data
    assert_eq!(write_deltas(synthetic_rawimage(None), DngCompression::Uncompressed)?, expect(&deltas, false));
    assert_eq!(write_deltas(synthetic_rawimage(None), DngCompression::Lossless)?, expect(&deltas, false));
    Ok(())
  }

  /// Synthetic 16x16 LinearRaw image with `cpp` planes of floating point data in range 0.0 .. 1.0
  fn synthetic_linear_f32(cpp: usize) -> RawImage {
    let mut rawimage = synthetic_rawimage(None);
    let samples = 16 * 16 * cpp;
    rawimage.data = RawImageData::Float((0..samples).map(|i| i as f32 / samples as f32).collect());
    rawimage.cpp = cpp;
    rawimage.photometric = RawPhotometricInterpretation::LinearRaw;
    rawimage.blacklevel = BlackLevel::zero(1, 1, cpp);
    rawimage.whitelevel = WhiteLevel::new(vec![1; cpp]);
    rawimage
  }

  /// Write `rawimage` lossy compressed, check the raw IFD and return the decoded image data, scaled to 0.0 .. 1.0
  fn write_lossy(rawimage: &RawImage) -> std::result::Result<Vec<f32>, Box<dyn std::error::Error>> {
    use crate::formats::tiff::{GenericTiffReader, reader::TiffReader};

    let data = write_synthetic_dng_compressed(rawimage, DngCompression::Lossy)?;
    let (backward_version, _) = read_backward_version_and_active_area(&data)?;
    assert_eq!(backward_version, DNG_VERSION_V1_4);
    let tiff = GenericTiffReader::new_with_buffer(&data, 0, 0, None)?;
    let raw_ifd = tiff.find_first_ifd_with_tag(DngTag::LinearizationTable).ok_or("LinearizationTable missing")?;
    let value = |tag: TiffCommonTag| raw_ifd.get_entry(tag).map(|entry| entry.value.force_u16(0));
    assert_eq!(value(TiffCommonTag::Compression), Some(CompressionMethod::LossyJPEG as u16));
    assert_eq!(value(TiffCommonTag::PhotometricInt), Some(PhotometricInterpretation::LinearRaw as u16));
    assert_eq!(value(TiffCommonTag::BitsPerSample), Some(8));
    assert_eq!(raw_ifd.get_entry(DngTag::LinearizationTable).map(|entry| entry.value.count()), Some(256));
    assert_eq!(tiff.root_ifd().get_entry(DngTag::AsShotNeutral).is_some(), rawimage.cpp > 1);

    let source = crate::rawsource::RawSource::new_from_slice(&data);
    let mut decoded = crate::get_decoder(&source)?.raw_image(&source, &crate::decoders::RawDecodeParams::default(), false)?;
    assert_eq!(decoded.cpp, rawimage.cpp);
    decoded.apply_scaling()?;
    Ok(decoded.data.as_f32().into_owned())
  }

  /// Compare lossy compressed data, the step size of 8 bit codes is up to 1.2% of the white level
  fn assert_lossy_eq(decoded: &[f32], expected: &[f32]) {
    assert_eq!(decoded.len(), expected.len());
    for (i, (d, e)) in decoded.iter().zip(expected).enumerate() {
      assert!((d - e).abs() < 0.03, "sample {}: decoded {}, expected {}", i, d, e);
    }
  }

  #[test]
  fn lossy_rgb_roundtrip() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let rawimage = synthetic_linear_f32(3);
    assert_lossy_eq(&write_lossy(&rawimage)?, &rawimage.data.as_f32());
    Ok(())
  }

  #[test]
  fn lossy_monochrome_roundtrip() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let rawimage = synthetic_linear_f32(1);
    assert_lossy_eq(&write_lossy(&rawimage)?, &rawimage.data.as_f32());

    // BlackIsZero is written as LinearRaw
    let mut rawimage = synthetic_rawimage(None);
    rawimage.photometric = RawPhotometricInterpretation::BlackIsZero;
    rawimage.blacklevel = BlackLevel::new(&[256_u32], 1, 1, 1);
    let expected: Vec<f32> = rawimage.data.as_f32().iter().map(|v| ((v - 256.0) / (4095.0 - 256.0)).max(0.0)).collect();
    assert_lossy_eq(&write_lossy(&rawimage)?, &expected);
    Ok(())
  }

  #[test]
  fn lossy_applies_linearization_table() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut rawimage = synthetic_linear_f32(3);
    rawimage.data = RawImageData::Integer((0..16 * 16 * 3).map(|i| (i * 5) as u16).collect());
    rawimage.linearization_table = Some((0..4096_u32).map(|i| (i * i / 256) as u16).collect());
    rawimage.whitelevel = WhiteLevel::new_bits(16, 3);
    let expected: Vec<f32> = (0..16 * 16 * 3).map(|i| ((i * 5) * (i * 5) / 256) as f32 / 65535.0).collect();
    assert_lossy_eq(&write_lossy(&rawimage)?, &expected);
    Ok(())
  }

  #[test]
  fn lossy_linearization_table_is_monotonic() {
    let table = lossy_linearization_table();
    assert_eq!(table.len(), 256);
    assert_eq!((table[0], table[255]), (0, u16::MAX));
    assert!(table.windows(2).all(|pair| pair[0] < pair[1]));

    // Each table value is encoded to its own code
    let mut rawimage = synthetic_linear_f32(1);
    rawimage.data = RawImageData::Float(table.iter().map(|v| *v as f32 / u16::MAX as f32).collect());
    to_lossy_8bit(&mut rawimage).expect("lossy conversion");
    assert_eq!(
      rawimage.data.as_f32().iter().map(|v| *v as u16).collect::<Vec<_>>(),
      (0..=255).collect::<Vec<u16>>()
    );
    assert_eq!(rawimage.linearization_table, Some(table));
  }

  #[test]
  fn lossy_requires_linear_raw_and_dng_1_4() -> std::result::Result<(), Box<dyn std::error::Error>> {
    assert!(write_synthetic_dng_compressed(&synthetic_rawimage(None), DngCompression::Lossy).is_err());
    assert!(write_synthetic_dng_compressed(&synthetic_linear_f32(4), DngCompression::Lossy).is_err());

    let mut buf = Cursor::new(Vec::new());
    let mut dng = DngWriter::new(&mut buf, DNG_VERSION_V1_1)?;
    let mut frame = dng.subframe(0);
    let result = frame.raw_image(
      &synthetic_linear_f32(3),
      CropMode::None,
      DngCompression::Lossy,
      DngPhotometricConversion::Original,
      &CompressionParams::default(),
    );
    assert!(result.is_err());
    Ok(())
  }

  type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

  const XTRANS: &str = "GGRGGBGGBGGRBRGRBGGGBGGRGGRGGBRBGBRG";

  /// Synthetic `width` x `height` CFA image of `pattern` with 12 bit data, `sample` gives the value of a pixel
  fn synthetic_cfa(pattern: &str, colors: &str, width: usize, height: usize, sample: impl Fn(usize, usize) -> u16) -> RawImage {
    use crate::{cfa::PlaneColor, rawimage::CFAConfig};

    let mut camera = Camera::new();
    camera.cfa = CFA::new(pattern);
    camera.plane_color = PlaneColor::new(colors);
    let pixels: Vec<u16> = (0..width * height).map(|i| sample(i % width, i / width)).collect();
    let photometric = RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&camera));
    let mut rawimage = RawImage::new(
      camera,
      PixU16::new_with(pixels, width, height),
      1,
      [1.0, 1.0, 1.0, 1.0],
      photometric,
      Some(BlackLevel::new(&[0_u32], 1, 1, 1)),
      Some(WhiteLevel::new_bits(12, 1)),
      false,
    );
    rawimage.bps = 12;
    rawimage
  }

  /// Noisy 12 bit samples, every value is likely to differ from its neighbours
  fn noise(x: usize, y: usize) -> u16 {
    ((x * 7919 + y * 104729) % 4096) as u16
  }

  /// Synthetic `width` x `height` LinearRaw image with `cpp` planes of 16 bit data
  fn synthetic_linear_u16(cpp: usize, width: usize, height: usize) -> RawImage {
    let mut rawimage = synthetic_linear_f32(cpp);
    rawimage.width = width;
    rawimage.height = height;
    rawimage.data = RawImageData::Integer((0..width * height * cpp).map(|i| ((i * 7919) % 65536) as u16).collect());
    rawimage.whitelevel = WhiteLevel::new_bits(16, cpp);
    rawimage
  }

  /// Write `rawimage` with JPEG XL compression
  fn write_jxl(rawimage: &RawImage, jxl: JxlParams, max_backward_version: DngVersionNumber) -> Result<Vec<u8>> {
    let mut buf = Cursor::new(Vec::new());
    let mut dng = DngWriter::new(&mut buf, max_backward_version)?;
    dng.load_base_tags(rawimage)?;
    let mut frame = dng.subframe(0);
    let params = CompressionParams { jxl, ..Default::default() };
    frame.raw_image(rawimage, CropMode::None, DngCompression::JpegXl, DngPhotometricConversion::Original, &params)?;
    frame.finalize()?;
    dng.close()?;
    Ok(buf.into_inner())
  }

  fn decode_dng(data: &[u8]) -> TestResult<RawImage> {
    let source = crate::rawsource::RawSource::new_from_slice(data);
    Ok(crate::get_decoder(&source)?.raw_image(&source, &crate::decoders::RawDecodeParams::default(), false)?)
  }

  fn integers(data: &RawImageData) -> &[u16] {
    match data {
      RawImageData::Integer(samples) => samples,
      RawImageData::Float(_) => panic!("expected integer data"),
    }
  }

  /// Values of the raw IFD tags, None if the tag is missing
  fn raw_ifd_values<const N: usize>(data: &[u8], tags: [u16; N]) -> TestResult<[Option<f32>; N]> {
    use crate::formats::tiff::{GenericTiffReader, reader::TiffReader};

    let tiff = GenericTiffReader::new_with_buffer(data, 0, 0, None)?;
    let raw_ifd = tiff.find_first_ifd_with_tag(TiffCommonTag::TileOffsets).ok_or("raw IFD missing")?;
    Ok(tags.map(|tag| raw_ifd.get_entry(tag).map(|entry| entry.value.force_f32(0))))
  }

  #[test]
  fn jxl_lossless_cfa_roundtrip() -> TestResult {
    if !jpegxl::libjxl_available_for_tests() {
      return Ok(());
    }
    // Image sizes are not divisible by the pattern size, so the interleave fields differ in size
    for (pattern, colors, width, height) in [("RGGB", "RGB", 37, 23), ("CYGM", "CYGM", 36, 25), (XTRANS, "RGB", 41, 29)] {
      let rawimage = synthetic_cfa(pattern, colors, width, height, noise);
      let data = write_jxl(&rawimage, JxlParams::default(), DNG_VERSION_V1_7_1)?;
      let (backward_version, _) = read_backward_version_and_active_area(&data)?;
      assert_eq!(backward_version, DNG_VERSION_V1_7_1);
      let cfa = CFA::new(pattern);
      let tags = [
        TiffCommonTag::Compression as u16,
        DngTag::RowInterleaveFactor as u16,
        DngTag::ColumnInterleaveFactor as u16,
        TiffCommonTag::BitsPerSample as u16,
        TiffCommonTag::SampleFormat as u16,
        DngTag::JXLDistance as u16,
        DngTag::JXLEffort as u16,
        DngTag::JXLDecodeSpeed as u16,
      ];
      assert_eq!(
        raw_ifd_values(&data, tags)?,
        [
          Some(52546.0),
          Some(cfa.height as f32),
          Some(cfa.width as f32),
          Some(16.0),
          Some(1.0),
          Some(0.0),
          Some(7.0),
          None
        ],
        "{}",
        pattern
      );
      let decoded = decode_dng(&data)?;
      assert_eq!((decoded.width, decoded.height), (width, height));
      assert_eq!(integers(&decoded.data), integers(&rawimage.data), "{}", pattern);
    }
    Ok(())
  }

  #[test]
  fn jxl_lossless_linear_raw_roundtrip() -> TestResult {
    if !jpegxl::libjxl_available_for_tests() {
      return Ok(());
    }
    for cpp in [1, 3] {
      let rawimage = synthetic_linear_u16(cpp, 35, 19);
      let data = write_jxl(&rawimage, JxlParams::default(), DNG_VERSION_V1_7)?;
      let (backward_version, _) = read_backward_version_and_active_area(&data)?;
      assert_eq!(backward_version, DNG_VERSION_V1_7);
      let tags = [DngTag::RowInterleaveFactor as u16, DngTag::ColumnInterleaveFactor as u16];
      assert_eq!(raw_ifd_values(&data, tags)?, [None, None]);
      let decoded = decode_dng(&data)?;
      assert_eq!(decoded.cpp, cpp);
      assert_eq!(integers(&decoded.data), integers(&rawimage.data));
    }
    Ok(())
  }

  #[test]
  fn jxl_lossless_floats() -> TestResult {
    use crate::formats::tiff::{GenericTiffReader, reader::TiffReader};

    if !jpegxl::libjxl_available_for_tests() {
      return Ok(());
    }
    // jxl-oxide decodes half float subnormals wrong, so only normal values are used
    let mut rawimage = synthetic_linear_f32(3);
    rawimage.data = RawImageData::Float((1..=16 * 16 * 3).map(|i| i as f32 / 800.0).collect());
    let data = write_jxl(&rawimage, JxlParams::default(), DNG_VERSION_V1_7)?;
    let tags = [TiffCommonTag::BitsPerSample as u16, TiffCommonTag::SampleFormat as u16];
    assert_eq!(raw_ifd_values(&data, tags)?, [Some(16.0), Some(3.0)]);
    // rawler can't read half float JPEG XL yet, decode the single tile directly
    let tiff = GenericTiffReader::new_with_buffer(&data, 0, 0, None)?;
    let raw_ifd = tiff.find_first_ifd_with_tag(TiffCommonTag::TileOffsets).ok_or("raw IFD missing")?;
    let offset = raw_ifd.get_entry(TiffCommonTag::TileOffsets).ok_or("offsets")?.value.force_usize(0);
    let size = raw_ifd.get_entry(TiffCommonTag::TileByteCounts).ok_or("sizes")?.value.force_usize(0);
    let image = jxl_oxide::JxlImage::builder()
      .read(&data[offset..offset + size])
      .map_err(|err| err.to_string())?;
    let render = image.render_frame(0).map_err(|err| err.to_string())?;
    let mut stream = render.stream_no_alpha();
    let mut decoded = vec![0.0_f32; 16 * 16 * 3];
    stream.write_to_buffer(&mut decoded);
    let expected: Vec<f32> = rawimage.data.as_f32().iter().map(|v| half::f16::from_f32(*v).to_f32()).collect();
    assert_eq!(decoded, expected);

    // Floats derived from integer data are stored as 16 bit integers
    rawimage.scaled_from_integer = true;
    let data = write_jxl(&rawimage, JxlParams::default(), DNG_VERSION_V1_7)?;
    assert_eq!(raw_ifd_values(&data, tags)?, [Some(16.0), Some(1.0)]);
    Ok(())
  }

  #[test]
  fn jxl_lossy_cfa() -> TestResult {
    if !jpegxl::libjxl_available_for_tests() {
      return Ok(());
    }
    let params = JxlParams {
      distance: 1.0,
      ..Default::default()
    };
    // Smooth gradients with a different level for each CFA phase, so mixed up
    // phases are detected. The two left columns are below the black level.
    let mut rawimage = synthetic_cfa("GRBG", "RGB", 30, 18, |x, y| {
      if x < 2 {
        200
      } else {
        (600 + 700 * (x % 2 + 2 * (y % 2)) + 30 * x + 20 * y) as u16
      }
    });
    rawimage.blacklevel = BlackLevel::new(&[256_u32], 1, 1, 1);
    let data = write_jxl(&rawimage, params, DNG_VERSION_V1_7_1)?;
    let tags = [DngTag::JXLDistance as u16, DngTag::JXLDecodeSpeed as u16, TiffCommonTag::BitsPerSample as u16];
    assert_eq!(raw_ifd_values(&data, tags)?, [Some(1.0), Some(4.0), Some(16.0)]);

    let mut decoded = decode_dng(&data)?;
    // Each interleave field is padded to the tile size (16), the active area excludes the padding
    assert_eq!((decoded.width, decoded.height), (32, 32));
    assert_eq!(decoded.active_area, Some(Rect::new(Point::new(0, 0), Dim2::new(30, 18))));
    assert_eq!(decoded.camera.cfa.to_string(), "GRBG");
    assert_eq!(decoded.blacklevel.levels, rawimage.blacklevel.levels);
    assert_eq!(decoded.whitelevel, rawimage.whitelevel);
    let table = decoded.linearization_table.clone().ok_or("LinearizationTable missing")?;
    assert_eq!(table.len(), 65536);
    // Values below the black level are kept
    let below_black = table[integers(&decoded.data)[decoded.width] as usize];
    assert!((180..=220).contains(&below_black), "below black: {}", below_black);

    decoded.apply_scaling()?;
    let mut expected = rawimage.clone();
    expected.apply_scaling()?;
    let (decoded, expected) = (decoded.data.as_f32(), expected.data.as_f32());
    let mut sum = 0.0;
    for y in 0..18 {
      for x in 0..30 {
        let (d, e) = (decoded[y * 32 + x], expected[y * 30 + x]);
        // Errors are larger next to the edge of the left columns
        assert!((d - e).abs() < 0.06, "pixel {}x{}: decoded {}, expected {}", x, y, d, e);
        sum += (d - e).abs();
      }
    }
    let mean = sum / (30 * 18) as f32;
    assert!(mean < 0.01, "mean error {}", mean);
    Ok(())
  }

  #[test]
  fn jxl_lossy_linear_raw() -> TestResult {
    if !jpegxl::libjxl_available_for_tests() {
      return Ok(());
    }
    let params = JxlParams {
      distance: 1.0,
      ..Default::default()
    };
    // Integer data with a LinearizationTable, which is composed into the lossy table
    let mut rawimage = synthetic_linear_u16(3, 32, 16);
    rawimage.data = RawImageData::Integer((0..32 * 16 * 3).map(|i| ((i / 3) % 32 * 100 + (i / 96) * 50) as u16).collect());
    rawimage.linearization_table = Some((0..4096_u32).map(|i| (i * i / 256) as u16).collect());
    let data = write_jxl(&rawimage, params, DNG_VERSION_V1_7)?;
    let mut decoded = decode_dng(&data)?;
    assert_eq!(decoded.linearization_table.as_ref().map(Vec::len), Some(65536));
    decoded.apply_scaling()?;
    let mut expected = rawimage.clone();
    expected.apply_scaling()?;
    for (d, e) in decoded.data.as_f32().iter().zip(expected.data.as_f32().iter()) {
      assert!((d - e).abs() < 0.02, "decoded {}, expected {}", d, e);
    }

    // Floats are stored as half floats with linear transfer
    let rawimage = synthetic_linear_f32(3);
    let data = write_jxl(&rawimage, params, DNG_VERSION_V1_7)?;
    let tags = [TiffCommonTag::SampleFormat as u16, DngTag::JXLDecodeSpeed as u16];
    assert_eq!(raw_ifd_values(&data, tags)?, [Some(3.0), Some(4.0)]);
    Ok(())
  }

  #[test]
  fn jxl_requires_dng_1_7() -> TestResult {
    if !jpegxl::libjxl_available_for_tests() {
      return Ok(());
    }
    let linear = synthetic_linear_u16(1, 16, 16);
    let cfa = synthetic_cfa("RGGB", "RGB", 16, 16, noise);
    assert!(write_jxl(&linear, JxlParams::default(), DNG_VERSION_V1_6).is_err());
    assert!(write_jxl(&linear, JxlParams::default(), DNG_VERSION_V1_7).is_ok());
    // CFA data needs ColumnInterleaveFactor
    assert!(write_jxl(&cfa, JxlParams::default(), DNG_VERSION_V1_7).is_err());
    assert!(write_jxl(&cfa, JxlParams::default(), DNG_VERSION_V1_7_1).is_ok());
    Ok(())
  }

  #[test]
  fn jxl_tile_size_splits_evenly() {
    assert_eq!(jxl_tile_size(16), 16);
    assert_eq!(jxl_tile_size(1000), 1008);
    assert_eq!(jxl_tile_size(1024), 1024);
    assert_eq!(jxl_tile_size(1500), 752);
    assert_eq!(jxl_tile_size(6000), 1008);
  }

  #[test]
  fn max_backward_version_is_clamped_to_dng_version() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut buf = Cursor::new(Vec::new());
    let dng = DngWriter::new_with_version(&mut buf, DNG_VERSION_V1_4, DNG_VERSION_V1_6)?;
    assert!(dng.backward_version_allows(DNG_VERSION_V1_4, "test"));
    assert!(!dng.backward_version_allows(DNG_VERSION_V1_6, "test"));
    Ok(())
  }

  #[test]
  fn backward_version_allows() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut buf = Cursor::new(Vec::new());
    let dng = DngWriter::new(&mut buf, DNG_VERSION_V1_4)?;
    assert!(dng.backward_version_allows(DNG_VERSION_V1_1, "test"));
    assert!(dng.backward_version_allows(DNG_VERSION_V1_4, "test"));
    assert!(!dng.backward_version_allows(DNG_VERSION_V1_6, "test"));
    Ok(())
  }

  #[test]
  fn dng_version_number() {
    let version = DngVersionNumber::from([1, 4, 2, 3]);
    assert_eq!((version.major(), version.minor(), version.revision(), version.build()), (1, 4, 2, 3));
    assert_eq!(version.to_string(), "1.4.2.3");
    assert!(DNG_VERSION_V1_1 < DNG_VERSION_V1_4);
    assert!(DngVersionNumber::from([1, 1, 0, 0]) < DngVersionNumber::from([1, 1, 0, 1]));
  }

  #[cfg(feature = "rawdb")]
  #[test]
  fn convert_canon_cr3_to_dng() -> std::result::Result<(), Box<dyn std::error::Error>> {
    use crate::{
      decoders::RawDecodeParams,
      devtools::rawdb::{get_rawdb_cache, rawdb_ensure_file},
      dng::{DNG_VERSION_V1_4, PREVIEW_JPEG_QUALITY},
      rawsource::RawSource,
    };
    use std::{
      fs::File,
      io::{BufReader, BufWriter},
    };

    let rawdb_cache = get_rawdb_cache();
    let raw_file = rawdb_ensure_file(&rawdb_cache, "Canon", "EOS R6", "raw_modes/Canon EOS R6_RAW_ISO_100_nocrop_nodual.CR3")?;

    let raw_source = RawSource::new(&raw_file)?;
    let orig_file = File::open(&raw_file).expect("File missing");

    let original_thread = std::thread::spawn(move || OriginalCompressed::compress(&mut BufReader::new(orig_file)));

    let decoder = crate::get_decoder(&raw_source)?;

    let rawimage = decoder.raw_image(&raw_source, &RawDecodeParams::default(), false)?;

    let full_image = decoder.preview_image(&raw_source, &RawDecodeParams::default())?.unwrap();

    let metadata = decoder.raw_metadata(&raw_source, &RawDecodeParams::default())?;

    let predictor = 1;

    let buf = BufWriter::new(Cursor::new(Vec::new()));
    //let buf = BufWriter::new(File::create("/tmp/dng_writer_simple_test.dng")?);
    let mut dng = DngWriter::new(buf, DNG_VERSION_V1_4)?;
    let mut raw = dng.subframe(0);
    raw.raw_image(
      &rawimage,
      CropMode::Best,
      DngCompression::Lossless,
      DngPhotometricConversion::Original,
      &CompressionParams {
        ljpeg92_predictor: predictor,
        ..Default::default()
      },
    )?;
    raw.finalize()?;
    dng.thumbnail(&full_image)?;
    let mut preview = dng.subframe(1);
    preview.preview(&full_image, PREVIEW_JPEG_QUALITY)?;
    preview.finalize()?;
    dng.load_base_tags(&rawimage)?;

    dng.load_metadata(&metadata)?;

    if let Some(xpacket) = decoder.xpacket(&raw_source, &RawDecodeParams::default())? {
      dng.xpacket(xpacket)?;
    }

    let original = original_thread.join().unwrap()?;

    dng.original_file(&original, "test.CR3")?;

    dng.close()?;

    Ok(())
  }
}
