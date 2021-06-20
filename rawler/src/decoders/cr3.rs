use image::DynamicImage;
use log::debug;
use std::convert::TryFrom;
use std::f32::NAN;

use crate::bits::Endian;
use crate::decoders::*;
use crate::decompressors::crx::decompress_crx_image;
use crate::formats::bmff::ext_cr3::cr3desc::Cr3DescBox;
use crate::formats::bmff::ext_cr3::iad1::Iad1Type;
use crate::formats::tiff::*;
use crate::tags::TiffTagEnum;
use crate::tiff::{Entry, TiffReader};
use crate::{pumps::ByteStream, RawImage};

#[derive(Debug, Clone)]
pub struct Cr3Decoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  //tiff: TiffIFD<'a>,
  bmff: Bmff,
  exif: Option<TiffReader>,
  makernotes: Option<TiffReader>,
  wb: Option<[f32; 4]>,
  blacklevels: Option<[u16; 4]>,
  whitelevel: Option<u16>,
  cmt1: Option<TiffReader>,
  cmt2: Option<TiffReader>,
  cmt3: Option<TiffReader>,
  cmt4: Option<TiffReader>,
  xpacket: Option<Vec<u8>>,
  lens_maker: Option<String>,
  lens_model: Option<String>,
}

impl<'a> Cr3Decoder<'a> {
  pub fn new(buf: &'a [u8], bmff: Bmff, rawloader: &'a RawLoader) -> Cr3Decoder<'a> {
    Cr3Decoder {
      buffer: buf,
      bmff,
      rawloader: rawloader,
      exif: None,
      makernotes: None,
      wb: None,
      blacklevels: None,
      whitelevel: None,
      cmt1: None,
      cmt2: None,
      cmt3: None,
      cmt4: None,
      xpacket: None,
      lens_maker: None,
      lens_model: None,
    }
  }
}

const EXIF_TRANSFER_TAGS: [u16; 23] = [
  ExifTag::ExposureTime as u16,
  ExifTag::FNumber as u16,
  ExifTag::ISOSpeedRatings as u16,
  ExifTag::SensitivityType as u16,
  ExifTag::RecommendedExposureIndex as u16,
  ExifTag::ISOSpeed as u16,
  ExifTag::FocalLength as u16,
  ExifTag::ExposureBiasValue as u16,
  ExifTag::DateTimeOriginal as u16,
  ExifTag::CreateDate as u16,
  ExifTag::OffsetTime as u16,
  ExifTag::OffsetTimeDigitized as u16,
  ExifTag::OffsetTimeOriginal as u16,
  ExifTag::OwnerName as u16,
  ExifTag::LensSerialNumber as u16,
  ExifTag::SerialNumber as u16,
  ExifTag::ExposureProgram as u16,
  ExifTag::MeteringMode as u16,
  ExifTag::Flash as u16,
  ExifTag::ExposureMode as u16,
  ExifTag::WhiteBalance as u16,
  ExifTag::SceneCaptureType as u16,
  ExifTag::ShutterSpeedValue as u16,
];

fn transfer_exif_tag(tag: u16) -> bool {
  EXIF_TRANSFER_TAGS.contains(&tag)
}

impl<'a> Decoder for Cr3Decoder<'a> {
  fn xpacket(&self) -> Option<&Vec<u8>> {
    self.xpacket.as_ref()
  }

  fn populate_dng_root(&mut self, root_ifd: &mut DirectoryWriter) -> Result<(), String> {
    // Copy Orientation tag
    if let Some(cmt1_ifd) = self.cmt1.as_ref() {
      let ifd = cmt1_ifd.root_ifd();
      if let Some(orientation) = ifd.get_entry(ExifTag::Orientation) {
        root_ifd.add_value(ExifTag::Orientation, orientation.value.clone()).unwrap();
      }
    }

    if let Some(cmt4) = self.cmt4.as_ref() {
      let gpsinfo_offset = {
        let mut gps_ifd = root_ifd.new_directory();
        let ifd = cmt4.root_ifd();
        // Copy all GPS tags
        for (tag, entry) in ifd.entries() {
          gps_ifd.add_value(*tag, entry.value.clone()).unwrap();
        }
        gps_ifd.build().unwrap()
      };
      root_ifd.add_tag(ExifTag::GPSInfo, gpsinfo_offset as u32).unwrap();
    }
    Ok(())
  }

  fn populate_dng_exif(&mut self, exif_ifd: &mut DirectoryWriter) -> Result<(), String> {
    if let Some(cmt2_ifd) = self.cmt2.as_ref() {
      let ifd = cmt2_ifd.root_ifd();
      for (tag, entry) in ifd.entries().iter().filter(|(tag, _)| transfer_exif_tag(**tag)) {
        exif_ifd.add_value(*tag, entry.value.clone()).unwrap();
      }
    } else {
      debug!("CMT2 is not available, no EXIF!");
    }

    if let Some(value) = self.lens_maker.as_ref() {
      exif_ifd.add_tag(ExifTag::LensMake, value).unwrap();
    }

    if let Some(value) = self.lens_model.as_ref() {
      exif_ifd.add_tag(ExifTag::LensModel, value).unwrap();
    }

    Ok(())
  }

  fn decode_metadata(&mut self) -> Result<(), String> {
    if let Some(Cr3DescBox { cmt1, cmt2, cmt3, cmt4, .. }) = self.bmff.filebox.moov.cr3desc.as_ref() {
      let buf1 = cmt1.header.make_view(self.buffer, 0, 0);
      self.cmt1 = Some(TiffReader::new_with_buffer(buf1, 0, None).unwrap());
      let buf2 = cmt2.header.make_view(self.buffer, 0, 0);
      self.cmt2 = Some(TiffReader::new_with_buffer(buf2, 0, None).unwrap());
      let buf3 = cmt3.header.make_view(self.buffer, 0, 0);
      self.cmt3 = Some(TiffReader::new_with_buffer(buf3, 0, None).unwrap());
      let buf4 = cmt4.header.make_view(self.buffer, 0, 0);
      self.cmt4 = Some(TiffReader::new_with_buffer(buf4, 0, None).unwrap());
    }

    if let Some(cmt1) = &self.cmt1 {
      let make = cmt1.get_entry(ExifTag::Make).unwrap().value.as_string().unwrap();
      let model = cmt1.get_entry(ExifTag::Model).unwrap().value.as_string().unwrap();

      let cam = self.rawloader.check_supported_with_everything(&make, &model, "")?;

      let offset = self.bmff.filebox.moov.traks[3].mdia.minf.stbl.co64.as_ref().unwrap().entries[0] as usize;
      let size = self.bmff.filebox.moov.traks[3].mdia.minf.stbl.stsz.sample_sizes[0] as usize;

      debug!("CTMD mdat offset: {}", offset);
      debug!("CTMD mdat size: {}", size);

      let buf = &self.buffer[offset..offset + size];

      let mut substream = ByteStream::new(buf, Endian::Little);

      let ctmd = Ctmd::new(&mut substream);

      if let Some(rec8) = ctmd.records.get(&8).as_ref() {
        // We skip 8 bytes here as this is the record header

        //let makernotes = TiffIFD::new(&rec8.payload[8..], 0, 0, 0, 1, Endian::Little, &vec![]).unwrap();

        //let mut filebuf = File::create("/tmp/fdump.tif").unwrap();
        //filebuf.write(&rec8.payload).unwrap();

        let ctmd_record8 = TiffReader::new_with_buffer(&rec8.payload[8..], 0, Some(0)).unwrap();

        //let ctmd_record8 = TiffIFD::new_root(&rec8.payload[8..], 0, &vec![]).unwrap();

        if let Some(levels) = ctmd_record8.get_entry(Cr3MakernoteTag::ColorData) {
          let wb_idx = if cam.wb_offset != 0 { cam.wb_offset } else { 0 }; // TODO: fail if not found
          let bl_idx = if cam.bl_offset != 0 { cam.bl_offset } else { 0 };
          let wl_idx = if cam.wl_offset != 0 { cam.wl_offset } else { 0 };
          if let crate::tiff::Value::Short(v) = &levels.value {
            self.wb = Some([v[wb_idx] as f32, v[wb_idx + 1] as f32, v[wb_idx + 3] as f32, NAN]);
            self.blacklevels = Some([v[bl_idx], v[bl_idx + 1], v[bl_idx + 2], v[bl_idx + 3]]);
            self.whitelevel = Some(v[wl_idx]);
          }
        }
      }

      if let Some(cmt3) = self.cmt3.as_ref() {
        if let Some(Entry {
          value: crate::tiff::Value::Short(v),
          ..
        }) = cmt3.get_entry(Cr3MakernoteTag::CameraSettings)
        {
          let lens_info = v[22];
          debug!("Lens Info tag: {}", lens_info);

          if let Some(cmt2) = self.cmt2.as_ref() {
            if let Some(Entry {
              value: crate::tiff::Value::Ascii(lens_id),
              ..
            }) = cmt2.get_entry(ExifTag::LensModel)
            {
              if lens_id.strings()[0] == "EF135mm f/2L USM" {
                self.lens_maker = Some(String::from("Canon"));
                self.lens_model = Some(String::from("Canon EF 135mm f/2L USM"));
              } else if lens_id.strings()[0] == "EF16-35mm f/4L IS USM" {
                self.lens_maker = Some(String::from("Canon"));
                self.lens_model = Some(String::from("Canon EF 16-35mm f/4L IS USM"));
              } else if lens_id.strings()[0] == "RF15-35mm F2.8 L IS USM" {
                self.lens_maker = Some(String::from("Canon"));
                self.lens_model = Some(String::from("Canon RF 15-35mm F2.8L IS USM"));
              }
            }
          }
        }
      }

      if let Some(xpacket_box) = self.bmff.filebox.cr3xpacket.as_ref() {
        let offset = (xpacket_box.header.offset + xpacket_box.header.header_len) as usize;
        let size = (xpacket_box.header.size - xpacket_box.header.header_len) as usize;
        let buf = &self.buffer[offset..offset + size];
        self.xpacket = Some(Vec::from(buf));
      }
    } else {
      return Err(format!("CMT1 not found"));
    }

    Ok(())
  }

  fn raw_image(&self, dummy: bool) -> Result<RawImage, String> {
    // TODO: add support check

    if let Some(cmt1) = &self.cmt1 {
      let make = cmt1.get_entry(ExifTag::Make).unwrap().value.as_string().unwrap();
      let model = cmt1.get_entry(ExifTag::Model).unwrap().value.as_string().unwrap();

      let camera = self.rawloader.check_supported_with_everything(&make, &model, "")?;

      let offset = self.bmff.filebox.moov.traks[2].mdia.minf.stbl.co64.as_ref().unwrap().entries[0] as usize;
      let size = self.bmff.filebox.moov.traks[2].mdia.minf.stbl.stsz.sample_sizes[0] as usize;
      debug!("raw mdat offset: {}", offset);
      debug!("raw mdat size: {}", size);
      //let mdat_data_offset = (self.bmff.filebox.mdat.header.offset + self.bmff.filebox.mdat.header.header_len) as usize;

      let buf = &self.buffer[offset..offset + size];

      let cmp1 = self.bmff.filebox.moov.traks[2]
        .mdia
        .minf
        .stbl
        .stsd
        .craw
        .as_ref()
        .unwrap()
        .cmp1
        .as_ref()
        .unwrap();

      debug!("cmp1 mdat hdr size: {}", cmp1.mdat_hdr_size);

      let image = decompress_crx_image(&buf, cmp1).unwrap();

      let wb = self.wb.unwrap();
      let blacklevel = self.blacklevels.as_ref().unwrap();

      let mut img = RawImage::new(camera, cmp1.f_width as usize, cmp1.f_height as usize, wb, image, dummy);

      img.blacklevels = *blacklevel;
      img.whitelevels = [
        *self.whitelevel.as_ref().unwrap(),
        *self.whitelevel.as_ref().unwrap(),
        *self.whitelevel.as_ref().unwrap(),
        *self.whitelevel.as_ref().unwrap(),
      ];

      let iad1 = &self.bmff.filebox.moov.traks[2]
        .mdia
        .minf
        .stbl
        .stsd
        .craw
        .as_ref()
        .unwrap()
        .cdi1
        .as_ref()
        .unwrap()
        .iad1;

      if let Iad1Type::Big(iad1_borders) = &iad1.iad1_type {
        img.crops = [
          iad1_borders.crop_top_offset as usize,                            // top
          (iad1.img_width - iad1_borders.crop_right_offset - 1) as usize,   // right
          (iad1.img_height - iad1_borders.crop_bottom_offset - 1) as usize, // bottom
          iad1_borders.crop_left_offset as usize,                           // left
        ];

        debug!("Canon active area: {:?}", img.crops);
      }

      return Ok(img);
    } else {
      return Err(format!("Camera model unknown"));
    }
  }

  fn full_image(&self) -> Result<DynamicImage, String> {
    let offset = self.bmff.filebox.moov.traks[0].mdia.minf.stbl.co64.as_ref().unwrap().entries[0] as usize;
    let size = self.bmff.filebox.moov.traks[0].mdia.minf.stbl.stsz.sample_sizes[0] as usize;
    debug!("jpeg mdat offset: {}", offset);
    debug!("jpeg mdat size: {}", size);
    //let mdat_data_offset = (self.bmff.filebox.mdat.header.offset + self.bmff.filebox.mdat.header.header_len) as usize;

    let buf = &self.buffer[offset..offset + size];
    let img = image::load_from_memory_with_format(buf, image::ImageFormat::Jpeg).unwrap();
    Ok(img)
  }
}

impl<'a> Cr3Decoder<'a> {
  pub fn _new_makernote(buf: &'a [u8], offset: usize, base_offset: usize, chain_level: isize, e: Endian) -> Result<TiffIFD<'a>, String> {
    let mut off = 0;
    let data = &buf[offset..];
    let mut endian = e;

    // Some have MM or II to indicate endianness - read that
    if data[off..off + 2] == b"II"[..] {
      off += 2;
      endian = Endian::Little;
    }
    if data[off..off + 2] == b"MM"[..] {
      off += 2;
      endian = Endian::Big;
    }

    TiffIFD::new(buf, offset + off, base_offset, 0, chain_level + 1, endian, &vec![])
  }
}

#[derive(Clone, Debug)]
struct Ctmd {
  pub records: HashMap<u16, CtmdRecord>,
}

#[derive(Clone, Debug)]
struct CtmdRecord {
  pub rec_size: u32,
  pub rec_type: u16,
  pub reserved1: u8,
  pub reserved2: u8,
  pub reserved3: u16,
  pub reserved4: u16,
  pub payload: Vec<u8>,
}

impl Ctmd {
  pub fn new(data: &mut ByteStream) -> Self {
    let mut records = HashMap::new();

    while data.remaining_bytes() > 0 {
      let size = data.get_u32();
      let rec = CtmdRecord {
        rec_size: size,
        rec_type: data.get_u16(),
        reserved1: data.get_u8(),
        reserved2: data.get_u8(),
        reserved3: data.get_u16(),
        reserved4: data.get_u16(),
        payload: data.get_bytes(size as usize - 12),
      };
      records.insert(rec.rec_type, rec);
    }
    Self { records }
  }
}

#[derive(Debug, Copy, Clone, PartialEq, enumn::N)]
#[repr(u16)]
pub enum Cr3MakernoteTag {
  CameraSettings = 0x0001,
  FocusInfo = 0x0002,
  FlashInfo = 0x0003,
  ShotInfo = 0x0004,
  Panorama = 0x0005,
  ImageType = 0x0006,
  FirmareVer = 0x0007,
  FileNumber = 0x0008,
  OwnerName = 0x0009,
  UnknownD30 = 0x000a,
  SerialNum = 0x000c,
  CameraInfo = 0x000d,
  FileLen = 0x000e,
  CustomFunc = 0x000f,
  ModelId = 0x0010,
  MovieInfo = 0x0011,
  AFInfo = 0x0012,
  ThumbArea = 0x0013,
  SerialFormat = 0x0014,
  SuperMacro = 0x001a,
  DateStampMode = 0x001c,
  MyColors = 0x001d,
  FirmwareRev = 0x001e,
  Categories = 0x0023,
  FaceDetect1 = 0x0024,
  FaceDetect2 = 0x0025,
  AFInfo2 = 0x0026,
  ContrastInfo = 0x0027,
  ImgUniqueID = 0x0028,
  WBInfo = 0x0029,
  FaceDetect3 = 0x002f,
  TimeInfo = 0x0035,
  BatteryType = 0x0038,
  AFInfo3 = 0x003c,
  RawDataOffset = 0x0081,
  OrigDecisionDataOffset = 0x0083,
  CustomFunc1D = 0x0090,
  PersFunc = 0x0091,
  PersFuncValues = 0x0092,
  FileInfo = 0x0093,
  AFPointsInFocus1D = 0x0094,
  LensModel = 0x0095,
  InternalSerial = 0x0096,
  DustRemovalData = 0x0097,
  CropInfo = 0x0098,
  CustomFunc2 = 0x0099,
  AspectInfo = 0x009a,
  ProcessingInfo = 0x00a0,
  ToneCurveTable = 0x00a1,
  SharpnessTable = 0x00a2,
  SharpnessFreqTable = 0x00a3,
  WhiteBalanceTable = 0x00a4,
  ColorBalance = 0x00a9,
  MeasuredColor = 0x00aa,
  ColorTemp = 0x00ae,
  CanonFlags = 0x00b0,
  ModifiedInfo = 0x00b1,
  TnoeCurveMatching = 0x00b2,
  WhiteBalanceMatching = 0x00b3,
  ColorSpace = 0x00b4,
  PreviewImageInfo = 0x00b6,
  VRDOffset = 0x00d0,
  SensorInfo = 0x00e0,
  ColorData = 0x4001,
  CRWParam = 0x4002,
  ColorInfo = 0x4003,
  Flavor = 0x4005,
  PictureStyleUserDef = 0x4008,
  PictureStylePC = 0x4009,
  CustomPictureStyleFileName = 0x4010,
  AFMicroAdj = 0x4013,
  VignettingCorr = 0x4015,
  VignettingCorr2 = 0x4016,
  LightningOpt = 0x4018,
  LensInfo = 0x4019,
  AmbienceInfo = 0x4020,
  MultiExp = 0x4021,
  FilterInfo = 0x4024,
  HDRInfo = 0x4025,
  AFConfig = 0x4028,
  RawBurstModeRoll = 0x403f,
}

impl Into<u16> for Cr3MakernoteTag {
  fn into(self) -> u16 {
    self as u16
  }
}

impl TryFrom<u16> for Cr3MakernoteTag {
  type Error = String;

  fn try_from(value: u16) -> Result<Self, Self::Error> {
    Self::n(value).ok_or(format!("Unable to convert tag: {}, not defined in enum", value))
  }
}

impl TiffTagEnum for Cr3MakernoteTag {}
