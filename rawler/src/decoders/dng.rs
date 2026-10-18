use crate::RawImage;
use crate::cfa::*;
use crate::decoders::*;
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
    Ok(DngDecoder { tiff, rawloader })
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

    let raw_data = plain_image_from_ifd(raw, file)?;
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
        let subsampled = match (**ifd).get_entry(TiffCommonTag::NewSubFileType) {
          Some(e) => e.force_u32(0) & 1 != 0,
          None => false,
        };
        !subsampled && (compression == 7 || compression == 8 || compression == 1 || compression == 0x884c || compression == 52546)
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
      Ok([1.0 / levels.force_f32(0), 1.0 / levels.force_f32(1), 1.0 / levels.force_f32(2), f32::NAN])
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

  use crate::RawImage;
  use crate::cfa::CFA;
  use crate::decoders::{Camera, RawDecodeParams};
  use crate::dng::writer::DngWriter;
  use crate::dng::{CropMode, DNG_VERSION_V1_4, DngCompression, DngPhotometricConversion};
  use crate::formats::tiff::DirectoryWriter;
  use crate::pixarray::PixU16;
  use crate::rawimage::{BlackLevel, CFAConfig, RawImageData, RawPhotometricInterpretation, WhiteLevel};
  use crate::rawsource::RawSource;
  use crate::tags::TiffCommonTag;

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
  fn integer_whitelevel_defaults_to_bits() -> TestResult {
    let data = write_dng(&synthetic_rawimage(), |ifd| ifd.remove_tag(TiffCommonTag::WhiteLevel))?;
    assert_eq!(decode_dng(&data)?.whitelevel.0, vec![u16::MAX as u32]);
    Ok(())
  }
}
