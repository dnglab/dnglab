use crate::RawImage;
use crate::cfa::*;
use crate::decoders::*;
use crate::dng::{DNG_READER_VERSION, DNG_VERSION_V1_1, DngVersionNumber};
use crate::formats::tiff::Entry;
use crate::formats::tiff::ExtractFromIFD;
use crate::formats::tiff::Rational;
use crate::formats::tiff::SampleFormat;
use crate::formats::tiff::Value;
use crate::imgop::Dim2;
use crate::imgop::Point;
use crate::imgop::Rect;
use crate::imgop::matrix::*;
use crate::imgop::xyz::*;
use crate::tags::DngTag;
use crate::tags::TiffCommonTag;

#[derive(Debug, Clone)]
pub struct DngDecoder<'a> {
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
}

impl<'a> DngDecoder<'a> {
  pub fn new(_file: &RawSource, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<DngDecoder<'a>> {
    let decoder = DngDecoder { tiff, rawloader };
    if let Some(backward_version) = decoder.dng_backward_version()
      && backward_version > DNG_READER_VERSION
    {
      log::warn!(
        "DNG file requires a reader for DNG version {}, but only {} is supported. Trying to decode anyway.",
        backward_version,
        DNG_READER_VERSION
      );
    }
    Ok(decoder)
  }

  /// DNGVersion of the file
  pub fn dng_version(&self) -> Option<DngVersionNumber> {
    Self::read_version(self.tiff.root_ifd(), DngTag::DNGVersion)
  }

  /// DNGBackwardVersion of the file, defaults to DNGVersion with the last two bytes set to zero
  pub fn dng_backward_version(&self) -> Option<DngVersionNumber> {
    Self::read_version(self.tiff.root_ifd(), DngTag::DNGBackwardVersion)
      .or_else(|| self.dng_version().map(|version| DngVersionNumber([version.major(), version.minor(), 0, 0])))
  }

  fn read_version(ifd: &IFD, tag: DngTag) -> Option<DngVersionNumber> {
    ifd
      .get_entry(tag)
      .filter(|entry| entry.count() == 4)
      .map(|entry| DngVersionNumber([0, 1, 2, 3].map(|i| entry.value.force_u8(i))))
  }
}

/// DNG format encapsulation for analyzer
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DngFormat {
  tiff: GenericTiffReader,
}

impl<'a> Decoder for DngDecoder<'a> {
  fn raw_image(&self, file: &RawSource, _params: &RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let raw = self.get_raw_ifd()?;
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let cpp = fetch_tiff_tag!(raw, TiffCommonTag::SamplesPerPixel).force_usize(0);
    let bits = fetch_tiff_tag!(raw, TiffCommonTag::BitsPerSample).force_u32(0);
    let orientation = Orientation::from_tiff(self.tiff.root_ifd());

    let mut cam = self.make_camera(raw, width, height)?;
    // If we know the camera, re-use the clean names
    if let Ok(known_cam) = self
      .rawloader
      .check_supported_with_mode(self.tiff.root_ifd(), "dng")
      .or_else(|_| self.rawloader.check_supported(self.tiff.root_ifd()))
    {
      cam.clean_make = known_cam.clean_make;
      cam.clean_model = known_cam.clean_model;
      cam.hints.extend_from_slice(&known_cam.hints);
      cam.params.extend(known_cam.params.iter().map(|(k, v)| (k.clone(), v.clone())));
    } else {
      log::debug!("DNG: camera {} / {} is not in the camera catalog", cam.make, cam.model);
      // panic!("Camera {} / {} is not in the camera catalog", cam.make, cam.model);
    }

    let blacklevel = self.get_blacklevels(raw)?;
    // Default WhiteLevel is 1.0 for floating point and 2^BitsPerSample-1 for integer data
    let whitelevel = match self.get_whitelevels(raw)? {
      Some(whitelevel) => whitelevel,
      None if SampleFormat::extract(raw)? == Some(SampleFormat::IEEEFP) => WhiteLevel::new(vec![1; cpp]),
      None => WhiteLevel::new_bits(bits, cpp),
    };
    let whitelevel = Some(whitelevel);

    let photometric = match fetch_tiff_tag!(raw, TiffCommonTag::PhotometricInt).force_u32(0) {
      1 => RawPhotometricInterpretation::BlackIsZero,
      32803 => RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&cam)),
      34892 => RawPhotometricInterpretation::LinearRaw,
      _ => todo!(),
    };

    // DNG versions before 1.1.0.0 use a deviating 16-bit lossless JPEG encoding
    let ljpeg_dng_bug = self.dng_version().is_some_and(|version| version < DNG_VERSION_V1_1);
    let raw_data = plain_image_from_ifd_ext(raw, file, ljpeg_dng_bug)?;
    let wb_coeffs = self.get_wb(&cam)?;
    let mut image = RawImage::new_with_data(cam, raw_data, width * cpp, height, cpp, wb_coeffs, photometric, blacklevel, whitelevel, dummy);
    image.orientation = orientation;
    Ok(image)
  }

  fn format_dump(&self) -> FormatDump {
    FormatDump::Dng(DngFormat { tiff: self.tiff.clone() })
  }

  fn raw_metadata(&self, _file: &RawSource, _params: &RawDecodeParams) -> Result<RawMetadata> {
    let raw = self.get_raw_ifd()?;
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let mut cam = self.make_camera(raw, width, height)?;
    // If we know the camera, re-use the clean names
    if let Ok(known_cam) = self.rawloader.check_supported(self.tiff.root_ifd()) {
      cam.clean_make = known_cam.clean_make;
      cam.clean_model = known_cam.clean_model;
    }
    let exif = Exif::new(self.tiff.root_ifd())?;
    let mdata = RawMetadata::new(&cam, exif);
    Ok(mdata)
  }

  fn thumbnail_image(&self, file: &RawSource, _params: &RawDecodeParams) -> Result<Option<DynamicImage>> {
    if let Some(thumb_ifd) = Some(self.tiff.root_ifd()).filter(|ifd| ifd.get_entry(TiffCommonTag::NewSubFileType).map(|entry| entry.force_u16(0)) == Some(1)) {
      Ok(Some(dynamic_image_from_ifd(thumb_ifd, file)?))
    } else {
      Ok(None)
    }
  }

  fn preview_image(&self, file: &RawSource, params: &RawDecodeParams) -> Result<Option<DynamicImage>> {
    if params.image_index != 0 {
      return Ok(None);
    }
    if let Some(sub_ifds) = self.tiff.root_ifd().get_sub_ifd_all(TiffCommonTag::SubIFDs) {
      let first_ifd = sub_ifds
        .iter()
        .find(|ifd| ifd.get_entry(TiffCommonTag::NewSubFileType).map(|entry| entry.force_u32(0)) == Some(1));
      if let Some(preview_ifd) = first_ifd {
        return Ok(Some(dynamic_image_from_ifd(preview_ifd, file)?));
      }
    }
    Ok(None)
  }

  fn ifd(&self, wk_ifd: WellKnownIFD) -> Result<Option<Rc<IFD>>> {
    Ok(match wk_ifd {
      WellKnownIFD::Root => Some(Rc::new(self.tiff.root_ifd().clone())),
      WellKnownIFD::Raw => Some(Rc::new(self.get_raw_ifd()?.clone())),
      WellKnownIFD::Exif => self
        .tiff
        .root_ifd()
        .get_sub_ifd_all(ExifTag::ExifOffset)
        .and_then(|list| list.get(0))
        .cloned()
        .map(Rc::new),
      WellKnownIFD::ExifGps => self
        .tiff
        .root_ifd()
        .get_sub_ifd_all(ExifTag::GPSInfo)
        .and_then(|list| list.get(0))
        .cloned()
        .map(Rc::new),
      WellKnownIFD::VirtualDngRawTags => {
        let mut ifd = IFD::default();
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::OpcodeList1);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::OpcodeList2);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::OpcodeList3);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::NoiseProfile);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::BayerGreenSplit);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::ChromaBlurRadius);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::AntiAliasStrength);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::NoiseReductionApplied);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::ProfileGainTableMap);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::CameraCalibration1);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::CameraCalibration2);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::CameraCalibration3);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::ForwardMatrix1);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::ForwardMatrix2);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::ForwardMatrix3);
        Some(Rc::new(ifd))
      }
      WellKnownIFD::VirtualDngRootTags => {
        let mut ifd = IFD::default();
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileEmbedPolicy);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileHueSatMapData1);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileHueSatMapData2);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileHueSatMapData3);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileHueSatMapDims);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileHueSatMapEncoding);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileLookTableData);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileLookTableDims);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileLookTableEncoding);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileName);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileCopyright);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileToneCurve);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::DNGPrivateData);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::MakerNoteSafety);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::AnalogBalance);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::BaselineExposure);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::BaselineNoise);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::BaselineSharpness);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::LinearResponseLimit);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::CameraSerialNumber);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::AsShotICCProfile);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::AsShotPreProfileMatrix);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::CurrentICCProfile);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::CurrentPreProfileMatrix);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::AsShotProfileName);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::DefaultBlackRender);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::BaselineExposureOffset);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::DepthFormat);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::DepthNear);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::DepthFar);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::DepthUnits);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::DepthMeasureType);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::RGBTables);
        Some(Rc::new(ifd))
      }
      _ => return Ok(None),
    })
  }

  fn format_hint(&self) -> FormatHint {
    FormatHint::DNG
  }
}

impl<'a> DngDecoder<'a> {
  fn get_raw_ifd(&self) -> Result<&IFD> {
    let ifds = self
      .tiff
      .find_ifds_with_tag(TiffCommonTag::Compression)
      .into_iter()
      .filter(|ifd| {
        let compression = (**ifd)
          .get_entry(TiffCommonTag::Compression)
          .expect("This IFD must contains this tag")
          .force_u32(0);
        // The main raw image has NewSubFileType 0. Other values are previews (1),
        // transparency masks (4), depth maps (8), enhanced images (16) etc.
        let subfile_type = (**ifd).get_entry(TiffCommonTag::NewSubFileType).map(|e| e.force_u32(0)).unwrap_or(0);
        subfile_type == 0 && (compression == 7 || compression == 8 || compression == 1 || compression == 0x884c || compression == 52546)
      })
      .collect::<Vec<&IFD>>();
    if let Some(first) = ifds.first() {
      Ok(first)
    } else {
      Err(RawlerError::DecoderFailed(format!("TODO: Unsupported DNG compression")))
    }
  }

  fn make_camera(&self, raw: &IFD, width: usize, height: usize) -> Result<Camera> {
    let mode = String::from("dng");

    let make = self
      .tiff
      .root_ifd()
      .get_entry(TiffCommonTag::Make)
      .and_then(|x| x.as_string())
      .cloned()
      .unwrap_or_default();
    let model = self
      .tiff
      .root_ifd()
      .get_entry(TiffCommonTag::Model)
      .and_then(|x| x.as_string())
      .cloned()
      .unwrap_or_default();

    let active_area = self.get_active_area(raw, width, height);
    let crop_area = if let Some(crops) = self.get_crop(raw) {
      if let Some(active_area) = &active_area {
        let mut full = crops;
        full.p.x += active_area[0]; // left
        full.p.y += active_area[1]; // Top
        Some(full.as_ltrb_offsets(width, height))
      } else {
        Some(crops.as_ltrb_offsets(width, height))
      }
    } else {
      None
    };

    let linear = fetch_tiff_tag!(raw, TiffCommonTag::PhotometricInt).force_usize(0) == 34892;
    let cfa = if linear { CFA::default() } else { self.get_cfa(raw)? };
    let color_matrix = self.get_color_matrix()?;
    let real_bps = if raw.has_entry(TiffCommonTag::Linearization) {
      // If DNG contains linearization table, output is always 16 bits
      16
    } else {
      raw.get_entry(TiffCommonTag::BitsPerSample).map(|v| v.force_usize(0)).unwrap_or(16)
    };

    Ok(Camera {
      clean_make: make.clone(),
      clean_model: model.clone(),
      make,
      model,
      mode,
      whitelevel: None,
      blacklevel: None,
      blackareah: None,
      blackareav: None,
      xyz_to_cam: Default::default(),
      color_matrix,
      cfa,
      active_area,
      crop_area,
      real_bps,
      ..Default::default()
    })
  }

  fn get_wb(&self, cam: &Camera) -> Result<[f32; 4]> {
    if let Some(levels) = self.tiff.get_entry(DngTag::AsShotNeutral) {
      // One value per color plane (1, 3 or 4)
      let mut wb = [f32::NAN; 4];
      for (i, coeff) in wb.iter_mut().enumerate().take(levels.count() as usize) {
        *coeff = 1.0 / levels.force_f32(i);
      }
      Ok(wb)
    } else if let Some(levels) = self.tiff.get_entry(DngTag::AsShotWhiteXY) {
      // TODO: improve once AnalogBalance and CC is properly implemented
      if let Some(flat_colormatrix) = cam.color_matrix.get(&Illuminant::D65)
        && let Some(colormatrix) = transform_1d::<3, 3>(flat_colormatrix)
      {
        let wb_coeff = xy_whitepoint_to_wb_coeff(levels.force_f32(0), levels.force_f32(1), &colormatrix);
        Ok([wb_coeff[0], wb_coeff[1], wb_coeff[2], f32::NAN])
      } else {
        Ok([f32::NAN, f32::NAN, f32::NAN, f32::NAN])
      }
    } else {
      Ok([f32::NAN, f32::NAN, f32::NAN, f32::NAN])
    }
  }

  fn get_blacklevels(&self, raw: &IFD) -> Result<Option<BlackLevel>> {
    let cpp = raw.get_entry(TiffCommonTag::SamplesPerPixel).map(|entry| entry.force_usize(0)).unwrap_or(1);
    if let Some(entry) = raw.get_entry(TiffCommonTag::BlackLevels) {
      let levels = match &entry.value {
        Value::Short(black) => black.iter().copied().map(Rational::from).collect(),
        Value::Long(black) => black.iter().copied().map(Rational::from).collect(),
        Value::Rational(black) => black.clone(),
        _ => return Err(format!("Unsupported BlackLevel type: {}", entry.value_type_name()).into()),
      };
      let mut repeat = (1, 1);
      if let Some(Entry {
        value: Value::Short(value), ..
      }) = raw.get_entry(DngTag::BlackLevelRepeatDim)
      {
        if value.len() == 2 {
          repeat = (value[0] as usize, value[1] as usize);
        } else {
          // Pentax K-3 Mark III Monochrome is known to has invalid tag
          log::warn!("File has BlackLevelRepeatDim tag but with invalid length: {}", value.len());
        }
      }
      Ok(Some(BlackLevel::new(&levels, repeat.1, repeat.0, cpp)))
    } else {
      Ok(None)
    }
  }

  fn get_whitelevels(&self, raw: &IFD) -> Result<Option<WhiteLevel>> {
    let cpp = fetch_tiff_tag!(raw, TiffCommonTag::SamplesPerPixel).force_usize(0);
    if let Some(levels) = raw.get_entry(TiffCommonTag::WhiteLevel) {
      let mut whitelevels = WhiteLevel((0..levels.count()).map(|i| levels.force_u32(i as usize)).collect());
      // Fixes a bug where only a single whitelevel value is given.
      if whitelevels.0.len() == 1 && cpp > 1 {
        whitelevels.0 = vec![whitelevels.0[0]; cpp];
      }
      return Ok(Some(whitelevels));
    }
    Ok(None)
  }

  fn get_cfa(&self, raw: &IFD) -> Result<CFA> {
    let pattern = fetch_tiff_tag!(raw, TiffCommonTag::CFAPattern);
    let cfa = CFA::new_from_tag(pattern);
    // If DNG has active area, we need to calulate back the CFA pattern,
    // because for DNG the CFAPattern is relative to ActiveArea and we
    // use (0, 0) as starting point.
    if let Some(active_area) = self.get_active_area_borders(raw) {
      let top = active_area[0];
      let left = active_area[1];
      Ok(cfa.shift(left % cfa.width, top % cfa.height))
    } else {
      Ok(cfa)
    }
  }

  fn get_active_area_borders(&self, raw: &IFD) -> Option<[usize; 4]> {
    if let Some(crops) = raw.get_entry(DngTag::ActiveArea) {
      let rect = [crops.force_usize(0), crops.force_usize(1), crops.force_usize(2), crops.force_usize(3)];
      Some(rect)
    } else {
      // Ignore missing crops, at least some pentax DNGs don't have it
      None
    }
  }

  fn get_active_area(&self, raw: &IFD, width: usize, height: usize) -> Option<[usize; 4]> {
    if let Some(rect) = self.get_active_area_borders(raw) {
      Some(Rect::new_with_dng(&rect).as_ltrb_offsets(width, height))
    } else {
      None
    }
  }

  fn get_crop(&self, raw: &IFD) -> Option<Rect> {
    if let Some(crops) = raw.get_entry(DngTag::DefaultCropOrigin) {
      let p = Point::new(crops.force_usize(0), crops.force_usize(1));
      if let Some(size) = raw.get_entry(DngTag::DefaultCropSize) {
        let d = Dim2::new(size.force_usize(0), size.force_usize(1));
        return Some(Rect::new(p, d));
      }
    }
    None
  }

  fn _get_masked_areas(&self, raw: &IFD) -> Vec<Rect> {
    let mut areas = Vec::new();

    if let Some(masked_area) = raw.get_entry(TiffCommonTag::MaskedAreas) {
      for x in (0..masked_area.count() as usize).step_by(4) {
        areas.push(Rect::new_with_points(
          Point::new(masked_area.force_usize(x), masked_area.force_usize(x + 1)),
          Point::new(masked_area.force_usize(x + 2), masked_area.force_usize(x + 3)),
        ));
      }
    }

    areas
  }

  fn get_color_matrix(&self) -> Result<HashMap<Illuminant, FlatColorMatrix>> {
    let mut result = HashMap::new();

    let mut read_matrix = |cal: DngTag, mat: DngTag| -> Result<()> {
      if let Some(c) = self.tiff.get_entry(mat) {
        let illuminant_val = self.tiff.get_entry(cal).map(|e| e.force_u16(0)).unwrap_or(21);
        let illuminant: Illuminant = illuminant_val.try_into().unwrap_or(Illuminant::D65);

        let mut matrix = FlatColorMatrix::new();
        for i in 0..c.count() as usize {
          matrix.push(c.force_f32(i));
        }

        if !matrix.is_empty() && matrix.len() <= 12 {
          result.insert(illuminant, matrix);
        } else {
          log::warn!("Invalid ColorMatrix dimensions for illuminant {:?}: length {}", illuminant, matrix.len());
        }
      }
      Ok(())
    };

    read_matrix(DngTag::CalibrationIlluminant1, DngTag::ColorMatrix1)?;
    read_matrix(DngTag::CalibrationIlluminant2, DngTag::ColorMatrix2)?;
    // TODO: add 3

    Ok(result)
  }
}

#[cfg(test)]
mod tests {
  use std::io::Cursor;

  use super::DngDecoder;
  use crate::RawImage;
  use crate::cfa::CFA;
  use crate::decoders::{Camera, RawDecodeParams};
  use crate::dng::writer::DngWriter;
  use crate::dng::{
    CropMode, DNG_VERSION_V1_0, DNG_VERSION_V1_1, DNG_VERSION_V1_4, DNG_VERSION_V1_6, DngCompression, DngPhotometricConversion, DngVersionNumber,
  };
  use crate::formats::tiff::Rational;
  use crate::formats::tiff::SampleFormat;
  use crate::formats::tiff::{DirectoryWriter, GenericTiffReader};
  use crate::pixarray::PixU16;
  use crate::rawimage::{BlackLevel, CFAConfig, RawImageData, RawPhotometricInterpretation, WhiteLevel};
  use crate::rawsource::RawSource;
  use crate::tags::{DngTag, TiffCommonTag};

  type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

  /// Synthetic 16x16 CFA image
  fn synthetic_rawimage() -> RawImage {
    let mut camera = Camera::new();
    camera.cfa = CFA::new("RGGB");
    camera.clean_make = "DNGLab".into();
    camera.clean_model = "Synthetic".into();
    let pixels: Vec<u16> = (0..16 * 16).map(|i| (i % 4096) as u16).collect();
    let photometric = RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&camera));
    RawImage::new(
      camera,
      PixU16::new_with(pixels, 16, 16),
      1,
      [1.0, 1.0, 1.0, f32::NAN],
      photometric,
      Some(BlackLevel::new(&[0_u32, 0, 0, 0], 2, 2, 1)),
      Some(WhiteLevel::new_bits(12, 1)),
      false,
    )
  }

  /// Write `rawimage` uncompressed as DNG, `modify` can change the raw IFD before it is written
  fn write_dng(rawimage: &RawImage, modify: impl FnOnce(&mut DirectoryWriter)) -> TestResult<Vec<u8>> {
    let mut buf = Cursor::new(Vec::new());
    let mut dng = DngWriter::new(&mut buf, DNG_VERSION_V1_4)?;
    dng.load_base_tags(rawimage)?;
    let mut frame = dng.subframe(0);
    frame.raw_image(rawimage, CropMode::None, DngCompression::Uncompressed, DngPhotometricConversion::Original, 1)?;
    modify(frame.ifd_mut());
    frame.finalize()?;
    dng.close()?;
    Ok(buf.into_inner())
  }

  fn decode_dng(data: &[u8]) -> TestResult<RawImage> {
    let source = RawSource::new_from_slice(data);
    let decoder = crate::get_decoder(&source)?;
    Ok(decoder.raw_image(&source, &RawDecodeParams::default(), false)?)
  }

  #[test]
  fn float_whitelevel_defaults_to_one() -> TestResult {
    let mut rawimage = synthetic_rawimage();
    rawimage.data = RawImageData::Float((0..16 * 16).map(|i| i as f32 / 256.0).collect());
    rawimage.whitelevel = WhiteLevel::new(vec![1]);
    let data = write_dng(&rawimage, |ifd| ifd.remove_tag(TiffCommonTag::WhiteLevel))?;
    assert_eq!(decode_dng(&data)?.whitelevel.0, vec![1]);
    Ok(())
  }

  #[test]
  fn raw_ifd_requires_subfile_type_zero() -> TestResult {
    let rawimage = synthetic_rawimage();
    let mut buf = Cursor::new(Vec::new());
    let mut dng = DngWriter::new(&mut buf, DNG_VERSION_V1_4)?;
    dng.load_base_tags(&rawimage)?;
    // Transparency mask (NewSubFileType 4) with a different size, written before the raw image
    let mut mask = dng.subframe(4);
    mask.rgb_image_u8(&[0; 8 * 8 * 3], 8, 8, DngCompression::Uncompressed, 1)?;
    mask.finalize()?;
    let mut frame = dng.subframe(0);
    frame.raw_image(&rawimage, CropMode::None, DngCompression::Uncompressed, DngPhotometricConversion::Original, 1)?;
    frame.finalize()?;
    dng.close()?;

    let decoded = decode_dng(&buf.into_inner())?;
    assert_eq!((decoded.width, decoded.height), (16, 16));
    Ok(())
  }

  /// Write a DNG with the given AsShotNeutral values and return the decoded WB coefficients
  fn decode_wb(as_shot_neutral: &[Rational]) -> TestResult<[f32; 4]> {
    let rawimage = synthetic_rawimage();
    let mut buf = Cursor::new(Vec::new());
    let mut dng = DngWriter::new(&mut buf, DNG_VERSION_V1_4)?;
    dng.load_base_tags(&rawimage)?;
    let mut frame = dng.subframe(0);
    frame.raw_image(&rawimage, CropMode::None, DngCompression::Uncompressed, DngPhotometricConversion::Original, 1)?;
    frame.finalize()?;
    dng.root_ifd_mut().add_tag(DngTag::AsShotNeutral, as_shot_neutral);
    dng.close()?;
    Ok(decode_dng(&buf.into_inner())?.wb_coeffs)
  }

  #[test]
  fn as_shot_neutral_with_four_planes() -> TestResult {
    let wb = decode_wb(&[Rational::new(1, 2), Rational::new(1, 1), Rational::new(1, 4), Rational::new(1, 5)])?;
    assert_eq!(wb, [2.0, 1.0, 4.0, 5.0]);
    Ok(())
  }

  #[test]
  fn as_shot_neutral_with_three_planes() -> TestResult {
    let wb = decode_wb(&[Rational::new(1, 2), Rational::new(1, 1), Rational::new(1, 4)])?;
    assert_eq!(wb[..3], [2.0, 1.0, 4.0]);
    assert!(wb[3].is_nan());
    Ok(())
  }

  /// Find the inline DNGBackwardVersion IFD entry in `data` (written in native byte order)
  fn find_backward_version_entry(data: &[u8]) -> usize {
    let pattern: Vec<u8> = [&50707_u16.to_ne_bytes()[..], &1_u16.to_ne_bytes(), &4_u32.to_ne_bytes()].concat();
    data
      .windows(pattern.len())
      .position(|w| w == pattern)
      .expect("DNGBackwardVersion entry not found")
  }

  fn dng_decoder(data: &[u8]) -> TestResult<DngDecoder<'static>> {
    let source = RawSource::new_from_slice(data);
    let tiff = GenericTiffReader::new_with_buffer(data, 0, 0, None)?;
    Ok(DngDecoder::new(&source, tiff, crate::global_loader())?)
  }

  #[test]
  fn dng_backward_version_from_tag() -> TestResult {
    let mut data = write_dng(&synthetic_rawimage(), |_| {})?;
    let entry = find_backward_version_entry(&data);
    // Newer than supported, decoder warns but continues
    data[entry + 8..entry + 12].copy_from_slice(&[1, 8, 0, 0]);
    let decoder = dng_decoder(&data)?;
    assert_eq!(decoder.dng_backward_version(), Some(DngVersionNumber([1, 8, 0, 0])));
    assert_eq!(decoder.dng_version(), Some(DNG_VERSION_V1_6));
    Ok(())
  }

  #[test]
  fn dng_backward_version_defaults_to_dng_version() -> TestResult {
    let rawimage = synthetic_rawimage();
    let mut buf = Cursor::new(Vec::new());
    let mut dng = DngWriter::new_with_version(&mut buf, DngVersionNumber([1, 4, 2, 1]), DNG_VERSION_V1_4)?;
    let mut frame = dng.subframe(0);
    frame.raw_image(&rawimage, CropMode::None, DngCompression::Uncompressed, DngPhotometricConversion::Original, 1)?;
    frame.finalize()?;
    dng.close()?;
    let mut data = buf.into_inner();
    // Rename the DNGBackwardVersion entry to an unused tag
    let entry = find_backward_version_entry(&data);
    data[entry..entry + 2].copy_from_slice(&0xFFFE_u16.to_ne_bytes());
    let decoder = dng_decoder(&data)?;
    assert_eq!(decoder.dng_backward_version(), Some(DngVersionNumber([1, 4, 0, 0])));
    Ok(())
  }

  /// Write a lossless DNG with the given DNGVersion. The image has a jump of 32768
  /// between same-colored pixels, which needs the special-cased 16-bit LJPEG difference.
  fn lossless_dng_with_32768_diff(dng_version: DngVersionNumber) -> TestResult<(Vec<u8>, Vec<u16>)> {
    let mut rawimage = synthetic_rawimage();
    let pixels: Vec<u16> = (0..16 * 16).map(|i| if i % 16 < 8 { 0 } else { 32768 }).collect();
    rawimage.data = RawImageData::Integer(pixels.clone());
    rawimage.whitelevel = WhiteLevel::new_bits(16, 1);
    rawimage.bps = 16;
    let mut buf = Cursor::new(Vec::new());
    let mut dng = DngWriter::new_with_version(&mut buf, dng_version, DNG_VERSION_V1_0)?;
    dng.load_base_tags(&rawimage)?;
    let mut frame = dng.subframe(0);
    frame.raw_image(&rawimage, CropMode::None, DngCompression::Lossless, DngPhotometricConversion::Original, 1)?;
    frame.finalize()?;
    dng.close()?;
    Ok((buf.into_inner(), pixels))
  }

  fn integer_data(image: RawImage) -> Option<Vec<u16>> {
    match image.data {
      RawImageData::Integer(data) => Some(data),
      RawImageData::Float(_) => None,
    }
  }

  #[test]
  fn ljpeg_dng_bug_depends_on_dng_version() -> TestResult {
    // DNG 1.1: compliant LJPEG decoding, the image matches
    let (data, pixels) = lossless_dng_with_32768_diff(DNG_VERSION_V1_1)?;
    assert_eq!(integer_data(decode_dng(&data)?), Some(pixels));

    // DNG 1.0: the decoder expects the old 16-bit deviation. The compliant
    // stream from our encoder is therefore not decoded to the same image.
    let (data, pixels) = lossless_dng_with_32768_diff(DNG_VERSION_V1_0)?;
    match decode_dng(&data) {
      Ok(image) => assert_ne!(integer_data(image), Some(pixels)),
      Err(_) => {} // Decoder may also fail to decode the stream
    }
    Ok(())
  }

  /// Zlib compress `data`
  fn zlib(data: &[u8]) -> TestResult<Vec<u8>> {
    use libflate::zlib::Encoder;
    use std::io::Write;

    let mut encoder = Encoder::new(Vec::new())?;
    encoder.write_all(data)?;
    Ok(encoder.finish().into_result()?)
  }

  /// Write a 16x16 DNG whose raw image is a single Deflate compressed strip
  fn write_deflate_dng(compressed: &[u8], bits: u16, sample_format: SampleFormat, predictor: Option<u16>) -> TestResult<Vec<u8>> {
    use crate::formats::tiff::CompressionMethod;

    let rawimage = synthetic_rawimage();
    let mut buf = Cursor::new(Vec::new());
    let mut dng = DngWriter::new(&mut buf, DNG_VERSION_V1_4)?;
    dng.load_base_tags(&rawimage)?;
    let offset = dng.dng.write_data(compressed)?;
    let mut frame = dng.subframe(0);
    frame.raw_image(&rawimage, CropMode::None, DngCompression::Uncompressed, DngPhotometricConversion::Original, 1)?;
    // Replace the 16 bit image data by the deflate compressed data
    let ifd = frame.ifd_mut();
    ifd.add_tag(TiffCommonTag::BitsPerSample, bits);
    ifd.add_tag(TiffCommonTag::SampleFormat, sample_format);
    ifd.add_tag(TiffCommonTag::Compression, CompressionMethod::Deflate);
    match predictor {
      Some(predictor) => ifd.add_tag(TiffCommonTag::Predictor, predictor),
      None => ifd.remove_tag(TiffCommonTag::Predictor),
    }
    ifd.add_tag(TiffCommonTag::StripOffsets, offset);
    ifd.add_tag(TiffCommonTag::StripByteCounts, compressed.len() as u32);
    ifd.add_tag(TiffCommonTag::RowsPerStrip, 16_u32);
    ifd.remove_tag(TiffCommonTag::WhiteLevel);
    frame.finalize()?;
    dng.close()?;
    Ok(buf.into_inner())
  }

  fn float_data(image: RawImage) -> Vec<f32> {
    match image.data {
      RawImageData::Float(data) => data,
      RawImageData::Integer(_) => panic!("expected float data"),
    }
  }

  #[test]
  fn integer_32bit_deflate() -> TestResult {
    // Values above u16 range, but exact in f32 (< 2^24)
    let samples: Vec<u32> = (0..16 * 16_u32).map(|i| i * 50_000 % (1 << 24)).collect();
    // Horizontal difference (predictor 2) per row, native byte order
    let mut predicted = Vec::new();
    for row in samples.chunks_exact(16) {
      for col in 0..16 {
        let v = if col > 0 { row[col].wrapping_sub(row[col - 1]) } else { row[col] };
        predicted.extend_from_slice(&v.to_ne_bytes());
      }
    }
    let data = write_deflate_dng(&zlib(&predicted)?, 32, SampleFormat::Uint, Some(2))?;

    let decoded = decode_dng(&data)?;
    assert_eq!(decoded.whitelevel.0, vec![u32::MAX]);
    assert_eq!(float_data(decoded), samples.iter().map(|v| *v as f32).collect::<Vec<_>>());
    Ok(())
  }

  /// Float samples for 16x16 image
  fn float_samples() -> Vec<f32> {
    (0..16 * 16).map(|i| i as f32 / 256.0).collect()
  }

  #[test]
  fn float_deflate_strips_without_predictor() -> TestResult {
    // No Predictor tag, samples in native byte order
    let samples = float_samples();
    let raw: Vec<u8> = samples.iter().flat_map(|v| v.to_ne_bytes()).collect();
    let data = write_deflate_dng(&zlib(&raw)?, 32, SampleFormat::IEEEFP, None)?;

    let decoded = decode_dng(&data)?;
    assert_eq!(decoded.whitelevel.0, vec![1]);
    assert_eq!(float_data(decoded), samples);
    Ok(())
  }

  #[test]
  fn float_deflate_strips_fp_predictor() -> TestResult {
    // Floating point predictor 3: byte planes per row (big endian), then byte delta
    let samples = float_samples();
    let mut predicted = Vec::new();
    for row in samples.chunks_exact(16) {
      let mut bytes = vec![0_u8; 16 * 4];
      for (col, v) in row.iter().enumerate() {
        for (b, byte) in v.to_be_bytes().into_iter().enumerate() {
          bytes[b * 16 + col] = byte;
        }
      }
      for i in (1..bytes.len()).rev() {
        bytes[i] = bytes[i].wrapping_sub(bytes[i - 1]);
      }
      predicted.extend_from_slice(&bytes);
    }
    let data = write_deflate_dng(&zlib(&predicted)?, 32, SampleFormat::IEEEFP, Some(3))?;
    assert_eq!(float_data(decode_dng(&data)?), samples);
    Ok(())
  }

  /// Rows in interleaved field order: rows 0, n, 2n, ..., then 1, n+1, ...
  fn to_field_order(image: &[u16], width: usize, factor: usize) -> Vec<u16> {
    let height = image.len() / width;
    (0..factor)
      .flat_map(|field| (field..height).step_by(factor))
      .flat_map(|row| image[row * width..(row + 1) * width].to_vec())
      .collect()
  }

  /// Whole image as a single tile in sub-tile block order
  fn to_block_order(image: &[u16], width: usize, block_rows: usize, block_cols: usize) -> Vec<u16> {
    let height = image.len() / width;
    let mut out = Vec::with_capacity(image.len());
    for block_y in (0..height).step_by(block_rows) {
      for block_x in (0..width).step_by(block_cols) {
        for row in block_y..block_y + block_rows {
          out.extend_from_slice(&image[row * width + block_x..row * width + block_x + block_cols]);
        }
      }
    }
    out
  }

  /// Write the synthetic image with `stored` as image data and decode it again
  fn decode_stored_layout(stored: Vec<u16>, modify: impl FnOnce(&mut DirectoryWriter)) -> TestResult<Option<Vec<u16>>> {
    let mut rawimage = synthetic_rawimage();
    rawimage.data = RawImageData::Integer(stored);
    Ok(integer_data(decode_dng(&write_dng(&rawimage, modify)?)?))
  }

  fn synthetic_pixels() -> Vec<u16> {
    match synthetic_rawimage().data {
      RawImageData::Integer(data) => data,
      RawImageData::Float(_) => unreachable!(),
    }
  }

  #[test]
  fn row_interleave_factor() -> TestResult {
    let image = synthetic_pixels();
    let decoded = decode_stored_layout(to_field_order(&image, 16, 3), |ifd| ifd.add_tag(DngTag::RowInterleaveFactor, 3_u16))?;
    assert_eq!(decoded, Some(image));
    Ok(())
  }

  #[test]
  fn sub_tile_block_size() -> TestResult {
    let image = synthetic_pixels();
    let decoded = decode_stored_layout(to_block_order(&image, 16, 2, 4), |ifd| ifd.add_tag(DngTag::SubTileBlockSize, [2_u16, 4]))?;
    assert_eq!(decoded, Some(image));
    Ok(())
  }

  #[test]
  fn sub_tile_block_size_with_row_interleave() -> TestResult {
    // Rows are interleaved first, the result is stored in sub-tile blocks
    let image = synthetic_pixels();
    let stored = to_block_order(&to_field_order(&image, 16, 2), 16, 4, 2);
    let decoded = decode_stored_layout(stored, |ifd| {
      ifd.add_tag(DngTag::RowInterleaveFactor, 2_u16);
      ifd.add_tag(DngTag::SubTileBlockSize, [4_u16, 2]);
    })?;
    assert_eq!(decoded, Some(image));
    Ok(())
  }

  #[test]
  fn row_interleave_factor_float() -> TestResult {
    let image = float_samples();
    let stored: Vec<f32> = (0..2)
      .flat_map(|field| (field..16).step_by(2))
      .flat_map(|row| image[row * 16..(row + 1) * 16].to_vec())
      .collect();
    let mut rawimage = synthetic_rawimage();
    rawimage.data = RawImageData::Float(stored);
    rawimage.whitelevel = WhiteLevel::new(vec![1]);
    let data = write_dng(&rawimage, |ifd| ifd.add_tag(DngTag::RowInterleaveFactor, 2_u16))?;
    assert_eq!(float_data(decode_dng(&data)?), image);
    Ok(())
  }

  #[test]
  fn integer_whitelevel_defaults_to_bits() -> TestResult {
    let data = write_dng(&synthetic_rawimage(), |ifd| ifd.remove_tag(TiffCommonTag::WhiteLevel))?;
    assert_eq!(decode_dng(&data)?.whitelevel.0, vec![u16::MAX as u32]);
    Ok(())
  }
}
