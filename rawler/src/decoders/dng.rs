use std::cmp;

use image::EncodableLayout;
use image::ImageBuffer;
use image::Rgb;

use crate::alloc_image_ok;
use crate::alloc_image_plain;
use crate::bits::Endian;
use crate::bits::LEu32;
use crate::bits::LookupTable;
use crate::cfa::*;
use crate::decoders::*;
use crate::decompressors::ljpeg::*;
use crate::formats::tiff::Entry;
use crate::formats::tiff::Rational;
use crate::formats::tiff::Value;
use crate::imgop::xyz::FlatColorMatrix;
use crate::imgop::xyz::Illuminant;
use crate::imgop::Dim2;
use crate::imgop::Point;
use crate::imgop::Rect;
use crate::packed::*;
use crate::tags::DngTag;
use crate::tags::TiffCommonTag;
use crate::RawImage;

#[derive(Debug, Clone)]
pub struct DngDecoder<'a> {
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
}

impl<'a> DngDecoder<'a> {
  pub fn new(_file: &mut RawFile, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<DngDecoder<'a>> {
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
  fn raw_image(&self, file: &mut RawFile, _params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let raw = self.get_raw_ifd()?;
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let cpp = fetch_tiff_tag!(raw, TiffCommonTag::SamplesPerPixel).force_usize(0);
    let bits = fetch_tiff_tag!(raw, TiffCommonTag::BitsPerSample).force_u32(0);

    let mut image = match fetch_tiff_tag!(raw, TiffCommonTag::Compression).force_u32(0) {
      1 => self.decode_uncompressed(file, raw, width * cpp, height, dummy)?,
      7 => self.decode_compressed(file, raw, width * cpp, height, cpp, dummy)?,
      c => return Err(RawlerError::DecoderFailed(format!("Don't know how to read DNGs with compression {}", c))),
    };

    if let Some(lintable) = raw.get_entry(TiffCommonTag::Linearization) {
      if bits != 8 && fetch_tiff_tag!(raw, TiffCommonTag::Compression).force_u32(0) != 1 {
        // 8 bit uncompressed data is already read delinearized
        apply_linearization(&mut image, &lintable.value, bits);
      }
    }

    let orientation = Orientation::from_tiff(self.tiff.root_ifd());

    let mut cam = self.make_camera(raw, width, height)?;
    // If we know the camera, re-use the clean names
    if let Ok(known_cam) = self.rawloader.check_supported(self.tiff.root_ifd()) {
      cam.clean_make = known_cam.clean_make;
      cam.clean_model = known_cam.clean_model;
    }

    let blacklevel = self.get_blacklevels(raw)?;
    let whitelevel = self.get_whitelevels(raw)?.or(Some(WhiteLevel::new_bits(bits, cpp)));

    let photometric = match fetch_tiff_tag!(raw, TiffCommonTag::PhotometricInt).force_u32(0) {
      1 => RawPhotometricInterpretation::BlackIsZero,
      32803 => RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&cam)),
      34892 => RawPhotometricInterpretation::LinearRaw,
      _ => todo!(),
    };

    let mut image = RawImage::new(cam, image, cpp, self.get_wb()?, photometric, blacklevel, whitelevel, dummy);
    image.orientation = orientation;

    Ok(image)
  }

  fn format_dump(&self) -> FormatDump {
    FormatDump::Dng(DngFormat { tiff: self.tiff.clone() })
  }

  fn raw_metadata(&self, _file: &mut RawFile, _params: RawDecodeParams) -> Result<RawMetadata> {
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

  fn thumbnail_image(&self, file: &mut RawFile, params: RawDecodeParams) -> Result<Option<DynamicImage>> {
    if params.image_index != 0 {
      return Ok(None);
    }
    if let Some(thumb_ifd) = Some(self.tiff.root_ifd()).filter(|ifd| ifd.get_entry(TiffCommonTag::NewSubFileType).map(|entry| entry.force_u32(0)) == Some(1)) {
      let buf = thumb_ifd
        .strip_data(file.inner())
        .map_err(|e| RawlerError::DecoderFailed(format!("Failed to get strip data: {}", e)))?
        .into_iter()
        .flatten()
        .collect();
      let compression = thumb_ifd.get_entry(TiffCommonTag::Compression).ok_or("Missing tag")?.force_usize(0);
      let width = fetch_tiff_tag!(thumb_ifd, TiffCommonTag::ImageWidth).force_usize(0);
      let height = fetch_tiff_tag!(thumb_ifd, TiffCommonTag::ImageLength).force_usize(0);
      if compression == 1 {
        return Ok(Some(DynamicImage::ImageRgb8(
          ImageBuffer::<Rgb<u8>, Vec<u8>>::from_raw(width as u32, height as u32, buf).unwrap(), // TODO: remove unwraps
        )));
      } else {
        let img = image::load_from_memory_with_format(&buf, image::ImageFormat::Jpeg).unwrap();
        return Ok(Some(img));
      }
    }
    Ok(None)
  }

  fn full_image(&self, file: &mut RawFile, params: RawDecodeParams) -> Result<Option<DynamicImage>> {
    if params.image_index != 0 {
      return Ok(None);
    }
    if let Some(sub_ifds) = self.tiff.root_ifd().get_sub_ifd_all(TiffCommonTag::SubIFDs) {
      let first_ifd = sub_ifds
        .iter()
        .find(|ifd| ifd.get_entry(TiffCommonTag::NewSubFileType).map(|entry| entry.force_u32(0)) == Some(1));
      if let Some(preview_ifd) = first_ifd {
        let buf = preview_ifd
          .strip_data(file.inner())
          .map_err(|e| RawlerError::DecoderFailed(format!("Failed to get strip data: {}", e)))?
          .into_iter()
          .flatten()
          .collect();
        let compression = preview_ifd.get_entry(TiffCommonTag::Compression).ok_or("Missing tag")?.force_usize(0);
        let width = fetch_tiff_tag!(preview_ifd, TiffCommonTag::ImageWidth).force_usize(0);
        let height = fetch_tiff_tag!(preview_ifd, TiffCommonTag::ImageLength).force_usize(0);
        if compression == 1 {
          return Ok(Some(DynamicImage::ImageRgb8(
            ImageBuffer::<Rgb<u8>, Vec<u8>>::from_raw(width as u32, height as u32, buf).unwrap(), // TODO: remove unwraps
          )));
        } else {
          let img = image::load_from_memory_with_format(&buf, image::ImageFormat::Jpeg).unwrap();
          return Ok(Some(img));
        }
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
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileHueSatMapData1);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileHueSatMapData2);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileHueSatMapData3);
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
        let compression = (**ifd).get_entry(TiffCommonTag::Compression).unwrap().force_u32(0);
        let subsampled = match (**ifd).get_entry(TiffCommonTag::NewSubFileType) {
          Some(e) => e.force_u32(0) & 1 != 0,
          None => false,
        };
        !subsampled && (compression == 7 || compression == 1 || compression == 0x884c)
      })
      .collect::<Vec<&IFD>>();
    Ok(ifds[0])
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

  fn get_wb(&self) -> Result<[f32; 4]> {
    if let Some(levels) = self.tiff.get_entry(TiffCommonTag::AsShotNeutral) {
      Ok([1.0 / levels.force_f32(0), 1.0 / levels.force_f32(1), 1.0 / levels.force_f32(2), f32::NAN])
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
    Ok(CFA::new_from_tag(pattern))
  }

  fn get_active_area(&self, raw: &IFD, width: usize, height: usize) -> Option<[usize; 4]> {
    if let Some(crops) = raw.get_entry(DngTag::ActiveArea) {
      let rect = [crops.force_usize(0), crops.force_usize(1), crops.force_usize(2), crops.force_usize(3)];
      Some(Rect::new_with_dng(&rect).as_ltrb_offsets(width, height))
    } else {
      // Ignore missing crops, at least some pentax DNGs don't have it
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
        let illuminant: Illuminant = fetch_tiff_tag!(self.tiff, cal).force_u16(0).try_into()?;
        let mut matrix = FlatColorMatrix::new();
        for i in 0..c.count() as usize {
          matrix.push(c.force_f32(i));
        }
        assert!(matrix.len() <= 12 && !matrix.is_empty());
        result.insert(illuminant, matrix);
      }
      Ok(())
    };

    read_matrix(DngTag::CalibrationIlluminant1, DngTag::ColorMatrix1)?;
    read_matrix(DngTag::CalibrationIlluminant2, DngTag::ColorMatrix2)?;
    // TODO: add 3

    Ok(result)
  }

  pub fn decode_uncompressed(&self, file: &mut RawFile, raw: &IFD, width: usize, height: usize, dummy: bool) -> Result<PixU16> {
    let src: Vec<u8> = raw.strip_data(file.inner())?.into_iter().flatten().collect();
    match (raw.endian, fetch_tiff_tag!(raw, TiffCommonTag::BitsPerSample).force_u32(0)) {
      // 16 bits, encoding depends on TIFF endianess
      (Endian::Big, 16) => Ok(decode_16be(&src, width, height, dummy)),
      (Endian::Little, 16) => Ok(decode_16le(&src, width, height, dummy)),
      // 12 Bits, DNG spec says it must be always encoded as big-endian
      (_, 12) => Ok(decode_12be(&src, width, height, dummy)),
      // 10 Bits, DNG spec says it must be always encoded as big-endian
      (_, 10) => Ok(decode_10be(&src, width, height, dummy)),
      // 8 bits with linearization table
      (_, 8) if raw.has_entry(TiffCommonTag::Linearization) => {
        let linearization = fetch_tiff_tag!(self.tiff, TiffCommonTag::Linearization);
        let curve = {
          let mut points = vec![0_u16; 256];
          for i in 0..256 {
            points[i] = linearization.force_u32(i) as u16;
          }
          LookupTable::new(&points)
        };
        Ok(decode_8bit_wtable(&src, &curve, width, height, dummy))
      }
      // 8 bits
      (_, 8) => Ok(decode_8bit(&src, width, height, dummy)),
      // Generic MSB decoder for exotic packed bit sizes
      (_, bps) if bps > 0 && bps < 16 => Ok(decode_generic_msb(&src, width, height, bps, dummy)),
      // Unhandled bits
      (_, bps) => Err(format_args!("DNG: Don't know how to handle DNG with {} bps", bps).into()),
    }
  }

  pub fn decode_compressed(&self, file: &mut RawFile, raw: &IFD, width: usize, height: usize, cpp: usize, dummy: bool) -> Result<PixU16> {
    if let Some(offsets) = raw.get_entry(TiffCommonTag::StripOffsets) {
      // We're in a normal offset situation
      if offsets.count() != 1 {
        return Err("DNG: files with more than one slice not supported yet".into());
      }
      let offset = offsets.force_u64(0);
      let size = fetch_tiff_tag!(raw, TiffCommonTag::StripByteCounts).force_u64(0);
      let src = file.subview(offset, size).unwrap();
      let mut out = alloc_image_ok!(width, height, dummy);
      let decompressor = LjpegDecompressor::new(&src)?;
      decompressor.decode(out.pixels_mut(), 0, width, width, height, dummy)?;
      Ok(out)
    } else if let Some(offsets) = raw.get_entry(TiffCommonTag::TileOffsets) {
      // They've gone with tiling
      let twidth = fetch_tiff_tag!(raw, TiffCommonTag::TileWidth).force_usize(0) * cpp;
      let tlength = fetch_tiff_tag!(raw, TiffCommonTag::TileLength).force_usize(0);
      let coltiles = (width - 1) / twidth + 1;
      let rowtiles = (height - 1) / tlength + 1;
      if coltiles * rowtiles != offsets.count() as usize {
        return Err(format_args!("DNG: trying to decode {} tiles from {} offsets", coltiles * rowtiles, offsets.count()).into());
      }
      let buffer = file.as_vec().unwrap();
      decode_threaded_multiline(
        width,
        height,
        tlength,
        dummy,
        &(|strip: &mut [u16], row| {
          let row = row / tlength;
          for col in 0..coltiles {
            // Calculate output width & length for current tile (right or bottom tile may have smaller dimension)
            let owidth = cmp::min(width, (col + 1) * twidth) - col * twidth;
            let olength = cmp::min(height, (row + 1) * tlength) - row * tlength;
            let offset = offsets.force_usize(row * coltiles + col);
            let src = &buffer[offset..];
            let decompressor = LjpegDecompressor::new(src)?;

            // If SOF width & height matches tile dimension, we can decode directly to strip
            if decompressor.width() == twidth && decompressor.height() == tlength {
              decompressor.decode(strip, col * twidth, width, owidth, olength, dummy)?;
            }
            // If SOF has other dimension but still same pixel count, we can decode
            // by using a temporary tile buffer. This is the case if RGGB data is encoded
            // by two input lines into one output line.
            // Encoded data is aligned like this:
            //   RGRGRGRGRGRGRGRGRGGBGBGBGBGBGBGBGBGB
            //   RGRGRGRGRGRGRGRGRGGBGBGBGBGBGBGBGBGB
            else if decompressor.width() * decompressor.height() == twidth * tlength {
              // cps is already included in all values, so just compare them.
              let mut tile = alloc_image_plain!(decompressor.width(), decompressor.height(), dummy);
              // The stripwidth is of same width as the temporary tile.
              let stripwidth = decompressor.width();
              decompressor.decode(tile.pixels_mut(), 0, stripwidth, decompressor.width(), decompressor.height(), dummy)?;
              // Copy lines from temporary tile to strip buffer, but limit it if destination strip
              // is smaller than temporary tile (right/bottom tiles).
              for (i, pix) in tile.pixels().chunks_exact(twidth).enumerate().take(olength) {
                let start = (i * width) + (col * twidth);
                strip[start..start + owidth].copy_from_slice(&pix[0..owidth]);
              }
            } else {
              return Err(format!(
                "ljpeg92 decoding failed: tile dimensions {}x{} not matching SOF dimensions {}x{}, dng cps: {}, sof cps: {}",
                twidth,
                tlength,
                decompressor.width(),
                decompressor.height(),
                cpp,
                decompressor.components(),
              ));
            }
          }
          Ok(())
        }),
      )
      .map_err(RawlerError::DecoderFailed)
    } else {
      Err(RawlerError::DecoderFailed("DNG: didn't find tiles or strips".to_string()))
    }
  }
}

fn apply_linearization(image: &mut PixU16, tbl: &Value, bits: u32) {
  match tbl {
    Value::Short(points) => {
      let table = LookupTable::new_with_bits(points, bits);
      image.pixel_rows_mut().for_each(|row| {
        let mut random = LEu32(row.as_bytes(), 0);
        row.iter_mut().for_each(|p| {
          *p = table.dither(*p, &mut random);
        })
      });
    }
    _ => {
      panic!("Unsupported linear table");
    }
  }
}
