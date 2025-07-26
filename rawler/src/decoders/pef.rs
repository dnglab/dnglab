use image::DynamicImage;
use log::debug;
use log::warn;
use serde::Deserialize;
use serde::Serialize;

use super::BlackLevel;
use super::Camera;
use super::Decoder;
use super::FormatHint;
use super::RawDecodeParams;
use super::RawMetadata;
use crate::RawImage;
use crate::RawLoader;
use crate::RawlerError;
use crate::Result;
use crate::alloc_image_ok;
use crate::analyze::FormatDump;
use crate::bits::Endian;
use crate::decompressors::ljpeg::huffman::*;
use crate::exif::Exif;
use crate::formats::tiff::Entry;
use crate::formats::tiff::GenericTiffReader;
use crate::formats::tiff::IFD;
use crate::formats::tiff::Value;
use crate::formats::tiff::ifd::OffsetMode;
use crate::formats::tiff::reader::TiffReader;
use crate::lens::LensDescription;
use crate::lens::LensResolver;
use crate::packed::*;
use crate::pixarray::PixU16;
use crate::pumps::BitPumpMSB;
use crate::pumps::ByteStream;
use crate::rawimage::CFAConfig;
use crate::rawimage::RawPhotometricInterpretation;
use crate::rawsource::RawSource;
use crate::tags::ExifTag;
use crate::tags::TiffCommonTag;

#[derive(Debug, Clone)]
pub struct PefDecoder<'a> {
  camera: Camera,
  #[allow(unused)]
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
  makernote: IFD,
  /// Offset of makernote, needed to correct offsets of preview image
  makernote_offset: u32,
}

impl<'a> PefDecoder<'a> {
  pub fn new(file: &RawSource, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<PefDecoder<'a>> {
    debug!("PEF decoder choosen");

    let camera = rawloader.check_supported(tiff.root_ifd())?;

    let makernote = if let Some(exif) = tiff.find_first_ifd_with_tag(ExifTag::MakerNotes) {
      exif.parse_makernote(&mut file.reader(), OffsetMode::Absolute, &[])?
    } else {
      warn!("PEF makernote not found");
      None
    }
    .ok_or("File has not makernotes")?;

    let makernote_offset = tiff
      .find_first_ifd_with_tag(ExifTag::MakerNotes)
      .and_then(|exif| exif.get_entry(ExifTag::MakerNotes))
      .map(|entry| entry.offset().unwrap() as u32)
      .unwrap_or(0);

    //eprintln!("IFD makernote:");
    //for line in makernote.dump::<PefMakernote>(10) {
    //  eprintln!("{}", line);
    //}

    Ok(PefDecoder {
      camera,
      tiff,
      rawloader,
      makernote,
      makernote_offset,
    })
  }
}

/// CR2 format encapsulation for analyzer
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PefFormat {
  tiff: GenericTiffReader,
}

impl<'a> Decoder for PefDecoder<'a> {
  fn format_dump(&self) -> FormatDump {
    FormatDump::Pef(PefFormat { tiff: self.tiff.clone() })
  }

  fn raw_image(&self, file: &RawSource, _params: &RawDecodeParams, dummy: bool) -> Result<RawImage> {
    //for (i, ifd) in self.tiff.chains().iter().enumerate() {
    //  eprintln!("IFD {}", i);
    //  for line in ifd.dump::<crate::tags::LegacyTiffRootTag>(10) {
    //    eprintln!("{}", line);
    //  }
    //}

    let raw = self
      .tiff
      .find_first_ifd_with_tag(TiffCommonTag::StripOffsets)
      .ok_or_else(|| RawlerError::unsupported(&self.camera, "Unable to find IFD"))?;
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let offset = fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_usize(0);

    let src = file.subview_until_eof(offset as u64)?;

    let image = match fetch_tiff_tag!(raw, TiffCommonTag::Compression).get_u32(0) {
      Ok(Some(1)) => decode_16be(src, width, height, dummy),
      Ok(Some(32773)) => decode_12be(src, width, height, dummy),
      Ok(Some(65535)) => self.decode_compressed(src, width, height, dummy)?,
      Ok(Some(c)) => return Err(RawlerError::unsupported(&self.camera, format!("PEF: Don't know how to read compression {}", c))),
      _ => return Err(RawlerError::unsupported(&self.camera, "PEF: No compression tag found")),
    };

    let cpp = 1;
    let wb = self.get_wb()?;
    let blacklevel = self.get_blacklevel()?;
    let whitelevel = None;
    debug!("Found WB: {:?}", wb);
    let photometric = if self.camera.cfa.is_valid() {
      RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&self.camera))
    } else {
      RawPhotometricInterpretation::LinearRaw
    };
    Ok(RawImage::new(self.camera.clone(), image, cpp, wb, photometric, blacklevel, whitelevel, dummy))
  }

  fn full_image(&self, file: &RawSource, params: &RawDecodeParams) -> Result<Option<DynamicImage>> {
    if params.image_index != 0 {
      return Ok(None);
    }
    let size = self.makernote.get_entry(PefMakernote::PreviewImageSize);
    let length = self.makernote.get_entry(PefMakernote::PreviewImageLength);
    let start = self.makernote.get_entry(PefMakernote::PreviewImageStart);

    let image = match (size, length, start) {
      (Some(size), Some(length), Some(start)) => {
        let _width = size.force_u16(0);
        let _height = size.force_u16(1);
        let len = length.force_u32(0);
        let offset = start.force_u32(0);
        if len > 0 && offset > 0 {
          let buf = file.subview((self.makernote_offset + offset) as u64, len as u64)?;
          match image::load_from_memory_with_format(buf, image::ImageFormat::Jpeg) {
            Ok(img) => Some(img),
            Err(_) => {
              // Test offset without correction
              let buf = file.subview(offset as u64, len as u64)?;
              let img = image::load_from_memory_with_format(buf, image::ImageFormat::Jpeg)
                .map_err(|err| RawlerError::DecoderFailed(format!("Failed to read JPEG: {:?}", err)))?;
              Some(img)
            }
          }
        } else {
          None
        }
      }
      _ => todo!(),
    };

    if let Some(image) = image {
      // This tag contains the border definitions for the preview image.
      // We cut away these black borders.
      if let Some(Entry {
        value: Value::Byte(borders), ..
      }) = self.makernote.get_entry(PefMakernote::PreviewImageBorders)
      {
        let y = borders[0] as u32;
        let x = borders[2] as u32;
        let width = image.width() - x - borders[3] as u32;
        let height = image.height() - y - borders[1] as u32;
        return Ok(Some(image.crop_imm(x, y, width, height)));
      } else {
        return Ok(Some(image));
      }
    }

    todo!()
  }

  fn raw_metadata(&self, _file: &RawSource, _params: &RawDecodeParams) -> Result<RawMetadata> {
    let exif = Exif::new(self.tiff.root_ifd())?;
    let mdata = RawMetadata::new_with_lens(&self.camera, exif, self.get_lens_description()?.cloned());
    Ok(mdata)
  }

  fn format_hint(&self) -> FormatHint {
    FormatHint::PEF
  }
}

impl<'a> PefDecoder<'a> {
  fn get_wb(&self) -> Result<[f32; 4]> {
    match self.makernote.get_entry(PefMakernote::WhitePoint) {
      Some(wb) => {
        let raw_wb = [wb.force_u16(0) as f32, wb.force_u16(1) as f32, wb.force_u16(2) as f32, wb.force_u16(3) as f32];
        Ok(normalize_wb(raw_wb))
      }
      None => Ok([f32::NAN, f32::NAN, f32::NAN, f32::NAN]),
    }
  }

  fn get_blacklevel(&self) -> Result<Option<BlackLevel>> {
    match self.makernote.get_entry(PefMakernote::BlackPoint) {
      Some(data) => {
        if self.camera.cfa.is_valid() {
          let levels = [data.force_u16(0), data.force_u16(1), data.force_u16(2), data.force_u16(3)];
          Ok(Some(BlackLevel::new(&levels, self.camera.cfa.width, self.camera.cfa.height, 1)))
        } else {
          // Monochrome PEF like K-3
          let levels = [data.force_u16(0)];
          Ok(Some(BlackLevel::new(&levels, 1, 1, 1)))
        }
      }
      None => Ok(None),
    }
  }

  /// Get lens description by analyzing TIFF tags and makernotes
  fn get_lens_description(&self) -> Result<Option<&'static LensDescription>> {
    if let Some(Entry {
      value: Value::Byte(settings), ..
    }) = self.makernote.get_entry(PefMakernote::LensRec)
    {
      let lens_id = (settings[0] as u32, settings[1] as u32);
      debug!("LensRec tag: {:?}", lens_id);
      if [0, 1, 2].contains(&lens_id.0) {
        // 0 = M-42 or no lens
        // 1 = K or M lens
        // 2 = A Series lens
        return Ok(None);
      } else {
        let resolver = LensResolver::new()
          .with_camera(&self.camera)
          .with_lens_id(lens_id)
          .with_mounts(&["k-mount".into()]);
        return Ok(resolver.resolve());
      }
    }
    Ok(None)
  }

  fn decode_compressed(&self, src: &[u8], width: usize, height: usize, dummy: bool) -> Result<PixU16> {
    if let Some(huff) = self.makernote.get_entry(PefMakernote::HuffmanTable) {
      match &huff.value {
        Value::Undefined(data) => Self::do_decode(src, Some((data, self.tiff.get_endian())), width, height, dummy),
        _ => todo!(), // should not happen!
      }
    } else {
      Self::do_decode(src, None, width, height, dummy)
    }
  }

  pub(crate) fn do_decode(src: &[u8], huff: Option<(&[u8], Endian)>, width: usize, height: usize, dummy: bool) -> Result<PixU16> {
    let mut out = alloc_image_ok!(width, height, dummy);
    let mut htable = HuffTable::empty();

    /* Attempt to read huffman table, if found in makernote */
    if let Some((huff, endian)) = huff {
      debug!("Use in-file Huffman table");
      let mut stream = ByteStream::new(huff, endian);

      let depth: usize = (stream.get_u16() as usize + 12) & 0xf;
      stream.consume_bytes(12);

      let mut v0: [u32; 16] = [0; 16];
      for i in 0..depth {
        v0[i] = stream.get_u16() as u32;
      }

      let mut v1: [u32; 16] = [0; 16];
      for i in 0..depth {
        v1[i] = stream.get_u8() as u32;
      }

      // Calculate codes and store bitcounts
      let mut v2: [u32; 16] = [0; 16];
      for c in 0..depth {
        v2[c] = v0[c] >> (12 - v1[c]);
        htable.bits[v1[c] as usize] += 1;
      }

      // Find smallest
      for i in 0..depth {
        let mut sm_val: u32 = 0xfffffff;
        let mut sm_num: u32 = 0xff;
        for j in 0..depth {
          if v2[j] <= sm_val {
            sm_num = j as u32;
            sm_val = v2[j];
          }
        }
        htable.huffval[i] = sm_num;
        v2[sm_num as usize] = 0xffffffff;
      }
    } else {
      debug!("Fallback to standard Huffman table");
      // Initialize with legacy data
      let pentax_tree: [u8; 29] = [0, 2, 3, 1, 1, 1, 1, 1, 1, 2, 0, 0, 0, 0, 0, 0, 3, 4, 2, 5, 1, 6, 0, 7, 8, 9, 10, 11, 12];
      let mut acc: usize = 0;
      for i in 0..16 {
        htable.bits[i + 1] = pentax_tree[i] as u32;
        acc += htable.bits[i + 1] as usize;
      }
      for i in 0..acc {
        htable.huffval[i] = pentax_tree[i + 16] as u32;
      }
    }

    htable.initialize()?;

    let mut pump = BitPumpMSB::new(src);
    let mut pred_up1: [i32; 2] = [0, 0];
    let mut pred_up2: [i32; 2] = [0, 0];
    let mut pred_left1: i32;
    let mut pred_left2: i32;

    for row in 0..height {
      pred_up1[row & 1] += htable.huff_decode(&mut pump)?;
      pred_up2[row & 1] += htable.huff_decode(&mut pump)?;
      pred_left1 = pred_up1[row & 1];
      pred_left2 = pred_up2[row & 1];
      out[row * width + 0] = pred_left1 as u16;
      out[row * width + 1] = pred_left2 as u16;
      for col in (2..width).step_by(2) {
        pred_left1 += htable.huff_decode(&mut pump)?;
        pred_left2 += htable.huff_decode(&mut pump)?;
        out[row * width + col + 0] = pred_left1 as u16;
        out[row * width + col + 1] = pred_left2 as u16;
      }
    }
    Ok(out)
  }
}

fn normalize_wb(raw_wb: [f32; 4]) -> [f32; 4] {
  debug!("PEF raw wb: {:?}", raw_wb);
  // We never have more then RGB colors so far (no RGBE etc.)
  // So we combine G1 and G2 to get RGB wb.
  let div = raw_wb[1];
  let mut norm = raw_wb;
  norm.iter_mut().for_each(|v| {
    if v.is_normal() {
      *v /= div
    }
  });
  [norm[0], (norm[1] + norm[2]) / 2.0, norm[3], f32::NAN]
}

crate::tags::tiff_tag_enum!(PefMakernote);

#[allow(non_camel_case_types)]
#[derive(Debug, Copy, Clone, PartialEq, enumn::N)]
#[repr(u16)]
pub enum PefMakernote {
  PentaxVersion = 0x0000,
  PentaxModelType = 0x0001,
  PreviewImageSize = 0x0002,
  PreviewImageLength = 0x0003,
  PreviewImageStart = 0x0004,
  PentaxModelId = 0x0005,
  Date = 0x0006,
  Time = 0x0007,
  Quality = 0x0008,
  PentaxImageSize = 0x0009,
  PictureMode = 0x000b,
  FlashMode = 0x000c,
  FocusMode = 0x000d,
  AFPointSelected = 0x000e,
  AFPointsInFocus = 0x000f,
  FocusPosition = 0x0010,
  ExposureTime = 0x0012,
  FNumber = 0x0013,
  ISO = 0x0014,
  LightReading = 0x0015,
  ExposureCompensation = 0x0016,
  MeteringMode = 0x0017,
  AutoBracketing = 0x0018,
  WhiteBalance = 0x0019,
  WhiteBalanceMode = 0x001a,
  BlueBalance = 0x001b,
  RedBalance = 0x001c,
  FocalLength = 0x001d,
  DigitalZoom = 0x001e,
  Saturation = 0x001f,
  Contrast = 0x0020,
  Sharpness = 0x0021,
  WordTimeLocation = 0x0022,
  HometownCity = 0x0023,
  DestinationCity = 0x0024,
  HometownDST = 0x0025,
  DestinationDST = 0x0026,
  DSPFirmwareVersion = 0x0027,
  CPUFirmwareVersion = 0x0028,
  FrameNumber = 0x0029,
  EffectiveLV = 0x002d,
  ImageEditing = 0x0032,
  PictureMode2 = 0x0033,
  DriveMode = 0x0034,
  SensorSize = 0x0035,
  ColorSpace = 0x0037,
  ImageAreaOffset = 0x0038,
  RawImageSize = 0x0039,
  AFPointsInFocus2 = 0x003c,
  DataScaling = 0x003d,
  PreviewImageBorders = 0x003e,
  LensRec = 0x003f,
  SensitivityAdjust = 0x0040,
  ImageEditCount = 0x0041,
  CameraTemerature = 0x0047,
  AELock = 0x0048,
  NoiseReduction = 0x0049,
  FlashExposureComp = 0x004d,
  ImageTone = 0x004f,
  ColorTemperature = 0x0050,
  ColorTempDaylight = 0x0053,
  ColorTempShade = 0x0054,
  ColorTempCloudy = 0x0055,
  ColorTempTungsten = 0x0056,
  ColorTempFluorescentD = 0x0057,
  ColorTempFluorescentN = 0x0058,
  ColorTempFluorescentW = 0x0059,
  ColorTempFlash = 0x005a,
  ShakeReductionInfo = 0x005c,
  ShutterCount = 0x005d,
  FaceInfo = 0x0060,
  RawDevelopmentProcess = 0x0062,
  Hue = 0x0067,
  AWBInfo = 0x0068,
  DynamicRangeExpansion = 0x0069,
  TimeInfo = 0x006b,
  HighLowKeyAdj = 0x006c,
  ContastHighlight = 0x006d,
  ContrastShadow = 0x006e,
  ConstrastHightlightShadowAdj = 0x006f,
  FineSharpness = 0x0070,
  HighISONoiseReduction = 0x0071,
  AFAdjustment = 0x0072,
  MonochromeFilterEffect = 0x0073,
  MonochromeToning = 0x0074,
  FaceDetect = 0x0076,
  FaceDetectFrameIsze = 0x0077,
  ShadowCorrection = 0x0079,
  ISOAutoParameters = 0x007a,
  CrossProcess = 0x007b,
  LensCorr = 0x007d,
  WhiteLevel = 0x007e,
  BleachBypassToning = 0x007f,
  AspectRatio = 0x0080,
  BlurControl = 0x0082,
  HDR = 0x0085,
  ShutterType = 0x0087,
  NeutralDensityFilter = 0x0088,
  ISO2 = 0x008b,
  IntervalShooting = 0x0092,
  SkinToneCorrection = 0x0095,
  ClarityControl = 0x0096,
  BlackPoint = 0x0200,
  WhitePoint = 0x0201,
  ColorMatrixA = 0x0203,
  ColorMatrixB = 0x0204,
  CameraSettings = 0x0205,
  AEInfo = 0x0206,
  LensInfo = 0x0207,
  FlashInfo = 0x0208,
  AEMeteringSegements = 0x0209,
  FlashMeteringSegements = 0x020a,
  SlaveFlashMeteringSegements = 0x020b,
  WB_RGGBLevelsDaylight = 0x020d,
  WB_RGGBLevelsShade = 0x020e,
  WB_RGGBLevelsCloudy = 0x020f,
  WB_RGGBLevelsTungsten = 0x0210,
  WB_RGGBLevelsFluorescentD = 0x0211,
  WB_RGGBLevelsFluorescentN = 0x0212,
  WB_RGGBLevelsFluorescentW = 0x0213,
  WB_RGGBLevelsFlash = 0x0214,
  CameraInfo = 0x0215,
  BatteryInfo = 0x0216,
  SaturationInfo = 0x021b,
  ColorMatrixA2 = 0x021c,
  ColorMatrixB2 = 0x021d,
  AFInfo = 0x021f,
  HuffmanTable = 0x0220,
  KelvinWB = 0x0221,
  ColorInfo = 0x0222,
  EVStepInfo = 0x0224,
  ShotInfo = 0x0226,
  FacePos = 0x0227,
  FaceSize = 0x0228,
  SerialNumber = 0x0229,
  FilterInfo = 0x022a,
  LevelInfo = 0x022b,
  WBLevels = 0x022d,
  Artist = 0x022e,
  Copyright = 0x022f,
  FirmwareVersion = 0x0230,
  ConstrastDetectAFArea = 0x0231,
  CrossProcessParams = 0x0235,
  LensInfoQ = 0x0239,
  Model = 0x023f,
  PixelShiftInfo = 0x0243,
  AFPointInfo = 0x0245,
  DataDump = 0x03fe,
  TempInfo = 0x03ff,
  ToneCurve = 0x0402,
  ToneCurves = 0x0403,
  UnknownBlock = 0x0405,
  PrintIM = 0x0e00,
}
