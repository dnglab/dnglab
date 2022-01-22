// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use image::DynamicImage;
use log::{debug, warn};
use std::convert::{TryFrom, TryInto};
use std::f32::NAN;
use std::io::SeekFrom;

use crate::bits::Endian;
use crate::decompressors::crx::decompress_crx_image;
use crate::formats::bmff::ext_cr3::cr3desc::Cr3DescBox;
use crate::formats::bmff::ext_cr3::iad1::Iad1Type;
use crate::formats::bmff::FileBox;
use crate::formats::tiff::reader::TiffReader;
use crate::formats::tiff::{Entry, Rational, GenericTiffReader, Value};
use crate::imgop::{Point, Rect};
use crate::lens::{LensDescription, LensResolver};
use crate::tags::{DngTag, TiffTagEnum};
use crate::{decoders::*, RawFile};
use crate::{pumps::ByteStream, RawImage};

#[derive(Debug, Clone)]
pub struct Cr3Decoder<'a> {
  //filebuf:Arc<Buffer>,
  rawloader: &'a RawLoader,
  //tiff: TiffIFD<'a>,
  bmff: Bmff,
  #[allow(dead_code)]
  exif: Option<GenericTiffReader>,
  #[allow(dead_code)]
  makernotes: Option<GenericTiffReader>,
  wb: Option<[f32; 4]>,
  blacklevels: Option<[u16; 4]>,
  whitelevel: Option<u16>,
  cmt1: Option<GenericTiffReader>,
  cmt2: Option<GenericTiffReader>,
  cmt3: Option<GenericTiffReader>,
  cmt4: Option<GenericTiffReader>,
  xpacket: Option<Vec<u8>>,
  image_unique_id: Option<[u8; 16]>,
  lens_description: Option<&'static LensDescription>,
}

impl<'a> Cr3Decoder<'a> {
  pub fn new(_rawfile: &mut RawFile, bmff: Bmff, rawloader: &'a RawLoader) -> Result<Cr3Decoder<'a>> {
    let mut decoder = Cr3Decoder {
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
      image_unique_id: None,
      lens_description: None,
    };
    decoder.decode_metadata(_rawfile)?;
    Ok(decoder)
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

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cr3Format {
  pub filebox: FileBox,
}

impl<'a> Decoder for Cr3Decoder<'a> {
  fn format_dump(&self) -> FormatDump {
    FormatDump::Cr3(Cr3Format {
      filebox: self.bmff.filebox.clone(),
    })
  }

  fn xpacket(&self, _file: &mut RawFile) -> Option<&Vec<u8>> {
    self.xpacket.as_ref()
  }

  fn populate_capture_info(&mut self, capture_info: &mut CaptureInfo) -> Result<()> {
    if let Some(cmt2_ifd) = self.cmt2.as_ref() {
      let ifd = cmt2_ifd.root_ifd();
      if let Some(Entry { value: Value::Rational(v), .. }) = ifd.get_entry(ExifTag::ExposureTime) {
        capture_info.exposure_time = Some(v[0])
      }
      if let Some(Entry {
        value: Value::SRational(v), ..
      }) = ifd.get_entry(ExifTag::ExposureBiasValue)
      {
        capture_info.exposure_bias = Some(v[0])
      }
      if let Some(Entry {
        value: Value::SRational(v), ..
      }) = ifd.get_entry(ExifTag::ShutterSpeedValue)
      {
        capture_info.shutter_speed = Some(v[0])
      }
    } else {
      debug!("CMT2 is not available, no EXIF!");
    }

    if let Some(lens) = self.lens_description {
      let lens_spec: [Rational; 4] = [lens.focal_range[0], lens.focal_range[1], lens.aperture_range[0], lens.aperture_range[1]];
      capture_info.lens_make = Some(lens.lens_make.clone());
      capture_info.lens_model = Some(lens.lens_model.clone());
      capture_info.lens_spec = Some(lens_spec);
    }

    Ok(())
  }

  fn populate_dng_root(&mut self, root_ifd: &mut DirectoryWriter) -> Result<()> {
    // Copy Orientation tag
    if let Some(cmt1_ifd) = self.cmt1.as_ref() {
      let ifd = cmt1_ifd.root_ifd();
      if let Some(orientation) = ifd.get_entry(ExifTag::Orientation) {
        root_ifd.add_value(ExifTag::Orientation, orientation.value.clone())?;
      }
      if let Some(artist) = ifd.get_entry(ExifTag::Artist) {
        root_ifd.add_value(ExifTag::Artist, artist.value.clone())?;
      }
      if let Some(copyright) = ifd.get_entry(ExifTag::Copyright) {
        root_ifd.add_value(ExifTag::Copyright, copyright.value.clone())?;
      }
    }

    if let Some(lens) = self.lens_description {
      let lens_info: [Rational; 4] = [lens.focal_range[0], lens.focal_range[1], lens.aperture_range[0], lens.aperture_range[1]];
      root_ifd.add_tag(DngTag::LensInfo, lens_info)?;
    }

    if let Some(unique_id) = self.image_unique_id {
      // For CR3, we use the already included Makernote tag with unique image ID
      root_ifd.add_tag(DngTag::RawDataUniqueID, unique_id)?;
    }

    if let Some(cmt4) = self.cmt4.as_ref() {
      let gpsinfo_offset = {
        let mut gps_ifd = root_ifd.new_directory();
        let ifd = cmt4.root_ifd();
        // Copy all GPS tags
        for (tag, entry) in ifd.entries() {
          match tag {
            // Special handling for Exif.GPSInfo.GPSLatitude and Exif.GPSInfo.GPSLongitude.
            // Exif.GPSInfo.GPSTimeStamp is wrong, too and can be fixed with the same logic.
            // Canon CR3 contains only two rationals, but these tags are specified as a vector
            // of three reationals (degrees, minutes, seconds).
            // We fix this by extending with 0/1 as seconds value.
            0x0002 | 0x0004 | 0x0007 => match &entry.value {
              Value::Rational(v) => {
                let fixed_value = if v.len() == 2 { vec![v[0], v[1], Rational::new(0, 1)] } else { v.clone() };
                gps_ifd.add_value(*tag, Value::Rational(fixed_value))?;
              }
              _ => {
                warn!("CR3: Exif.GPSInfo.GPSLatitude and Exif.GPSInfo.GPSLongitude expected to be of type RATIONAL, GPS data is ignored");
              }
            },
            _ => {
              gps_ifd.add_value(*tag, entry.value.clone())?;
            }
          }
        }
        gps_ifd.build()?
      };
      root_ifd.add_tag(ExifTag::GPSInfo, gpsinfo_offset as u32)?;
    }
    Ok(())
  }

  fn populate_dng_exif(&mut self, exif_ifd: &mut DirectoryWriter) -> Result<()> {
    if let Some(cmt2_ifd) = self.cmt2.as_ref() {
      let ifd = cmt2_ifd.root_ifd();
      for (tag, entry) in ifd.entries().iter().filter(|(tag, _)| transfer_exif_tag(**tag)) {
        exif_ifd.add_value(*tag, entry.value.clone())?;
      }
    } else {
      debug!("CMT2 is not available, no EXIF!");
    }

    if let Some(lens) = self.lens_description {
      let lens_info: [Rational; 4] = [lens.focal_range[0], lens.focal_range[1], lens.aperture_range[0], lens.aperture_range[1]];
      exif_ifd.add_tag(ExifTag::LensSpecification, lens_info)?;
      exif_ifd.add_tag(ExifTag::LensMake, &lens.lens_make)?;
      exif_ifd.add_tag(ExifTag::LensModel, &lens.lens_model)?;
    }

    Ok(())
  }

  fn decode_metadata(&mut self, rawfile: &mut RawFile) -> Result<()> {
    if let Some(Cr3DescBox { cmt1, cmt2, cmt3, cmt4, .. }) = self.bmff.filebox.moov.cr3desc.as_ref() {
      /*
      let buf1 = cmt1.header.make_view(self.filebuf.raw_buf(), 0, 0);
      self.cmt1 = Some(TiffReader::new_with_buffer(buf1, 0, 0, None)?);
      let buf2 = cmt2.header.make_view(self.filebuf.raw_buf(), 0, 0);
      self.cmt2 = Some(TiffReader::new_with_buffer(buf2, 0, 0, None)?);
      let buf3 = cmt3.header.make_view(self.filebuf.raw_buf(), 0, 0);
      self.cmt3 = Some(TiffReader::new_with_buffer(buf3, 0, 0, None)?);
      let buf4 = cmt4.header.make_view(self.filebuf.raw_buf(), 0, 0);
      self.cmt4 = Some(TiffReader::new_with_buffer(buf4, 0, 0, None)?);
      */
      {
        let cmt1offset = cmt1.header.offset + cmt1.header.header_len;
        rawfile.inner().seek(SeekFrom::Start(cmt1offset)).unwrap();
        self.cmt1 = Some(GenericTiffReader::new(rawfile.inner(), cmt1offset as u32, 0, None, &[])?);
      }
      {
        let cmt2offset = cmt2.header.offset + cmt2.header.header_len;
        rawfile.inner().seek(SeekFrom::Start(cmt2offset)).unwrap();
        self.cmt2 = Some(GenericTiffReader::new(rawfile.inner(), cmt2offset as u32, 0, None, &[])?);
      }
      {
        let cmt3offset = cmt3.header.offset + cmt3.header.header_len;
        rawfile.inner().seek(SeekFrom::Start(cmt3offset)).unwrap();
        self.cmt3 = Some(GenericTiffReader::new(rawfile.inner(), cmt3offset as u32, 0, None, &[])?);
      }
      {
        let cmt4offset = cmt4.header.offset + cmt4.header.header_len;
        rawfile.inner().seek(SeekFrom::Start(cmt4offset)).unwrap();
        self.cmt4 = Some(GenericTiffReader::new(rawfile.inner(), cmt4offset as u32, 0, None, &[])?);
      }
    }

    if let Some(cmt1) = &self.cmt1 {
      let make = cmt1.get_entry(ExifTag::Make).unwrap().value.as_string().unwrap();
      let model = cmt1.get_entry(ExifTag::Model).unwrap().value.as_string().unwrap();

      let cam = self.rawloader.check_supported_with_everything(&make, &model, "")?;

      let offset = self.bmff.filebox.moov.traks[3].mdia.minf.stbl.co64.as_ref().unwrap().entries[0];
      let size = self.bmff.filebox.moov.traks[3].mdia.minf.stbl.stsz.sample_sizes[0] as u64;

      debug!("CTMD mdat offset: {}", offset);
      debug!("CTMD mdat size: {}", size);

      let buf = rawfile
        .get_range(offset, size)
        .map_err(|e| RawlerError::General(format!("I/O error while reading CR3 CTMD: {:?}", e)))?;

      let mut substream = ByteStream::new(&buf, Endian::Little);

      let ctmd = Ctmd::new(&mut substream);

      if let Some(rec8) = ctmd.records.get(&8).as_ref() {
        // We skip 8 bytes here as this is the record header

        //let makernotes = TiffIFD::new(&rec8.payload[8..], 0, 0, 0, 1, Endian::Little, &vec![]).unwrap();

        //let mut filebuf = File::create("/tmp/fdump.tif").unwrap();
        //filebuf.write(&rec8.payload).unwrap();

        let ctmd_record8 = GenericTiffReader::new_with_buffer(&rec8.payload[8..], 0, 0, Some(0))?;

        //let ctmd_record8 = TiffIFD::new_root(&rec8.payload[8..], 0, &vec![]).unwrap();

        if let Some(levels) = ctmd_record8.get_entry(Cr3MakernoteTag::ColorData) {
          if let crate::formats::tiff::Value::Short(v) = &levels.value {
            if let Some(offset) = cam.param_usize("colordata_wbcoeffs") {
              self.wb = Some([v[offset] as f32, v[offset + 1] as f32, v[offset + 3] as f32, NAN]);
            }
            if let Some(offset) = cam.param_usize("colordata_blacklevel") {
              debug!("Blacklevel offset: {:x}", offset);
              self.blacklevels = Some([v[offset], v[offset + 1], v[offset + 2], v[offset + 3]]);
            }
            if let Some(offset) = cam.param_usize("colordata_whitelevel") {
              self.whitelevel = Some(v[offset]);
            }
          }
        }
      }

      debug!("CR3 blacklevels: {:?}", self.blacklevels);
      debug!("CR3 whitelevel: {:?}", self.whitelevel);

      if let Some(cmt3) = self.cmt3.as_ref() {
        if let Some(Entry {
          value: crate::formats::tiff::Value::Short(v),
          ..
        }) = cmt3.get_entry(Cr3MakernoteTag::CameraSettings)
        {
          let lens_info = v[22];
          debug!("Lens Info tag: {}", lens_info);

          if let Some(cmt2) = self.cmt2.as_ref() {
            if let Some(Entry {
              value: crate::formats::tiff::Value::Ascii(lens_id),
              ..
            }) = cmt2.get_entry(ExifTag::LensModel)
            {
              let lens_str = &lens_id.strings()[0];
              let resolver = LensResolver::new().with_lens_model(lens_str);
              self.lens_description = resolver.resolve();
            }
          }
        }
      }

      if let Some(cmt3) = self.cmt3.as_ref() {
        if let Some(Entry {
          value: crate::formats::tiff::Value::Byte(v),
          ..
        }) = cmt3.get_entry(Cr3MakernoteTag::ImgUniqueID)
        {
          if v.len() == 16 {
            debug!("CR3 makernote ImgUniqueID: {:x?}", v);
            self.image_unique_id = Some(v.as_slice().try_into().expect("Invalid slice size"));
          }
        }
      }

      if let Some(xpacket_box) = self.bmff.filebox.cr3xpacket.as_ref() {
        let offset = xpacket_box.header.offset + xpacket_box.header.header_len;
        let size = xpacket_box.header.size - xpacket_box.header.header_len;
        let buf = rawfile
          .get_range(offset, size)
          .map_err(|e| RawlerError::General(format!("I/O error while reading CR3 XPACKET: {:?}", e)))?;
        self.xpacket = Some(Vec::from(buf));
      }
    } else {
      return Err(RawlerError::General(format!("CMT1 not found")));
    }

    Ok(())
  }

  fn raw_image_count(&self) -> Result<usize> {
    let raw_trak_id = std::env::var("RAWLER_CRX_RAW_TRAK")
      .ok()
      .map(|id| id.parse::<usize>().expect("RAWLER_CRX_RAW_TRAK must by of type usize"))
      .unwrap_or(2);
    let moov_trak = self
      .bmff
      .filebox
      .moov
      .traks
      .get(raw_trak_id)
      .ok_or(format!("Unable to get MOOV trak {}", raw_trak_id))?;
    let co64 = moov_trak.mdia.minf.stbl.co64.as_ref().ok_or(format!("No co64 box found"))?;
    Ok(co64.entries.len())
  }

  fn raw_image(&self, file: &mut RawFile, params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    // TODO: add support check

    let raw_trak_id = std::env::var("RAWLER_CRX_RAW_TRAK")
      .ok()
      .map(|id| id.parse::<usize>().expect("RAWLER_CRX_RAW_TRAK must by of type usize"))
      .unwrap_or(2);
    let raw_image_id = params.image_index;

    if let Some(cmt1) = &self.cmt1 {
      let make = cmt1.get_entry(ExifTag::Make).unwrap().value.as_string().unwrap();
      let model = cmt1.get_entry(ExifTag::Model).unwrap().value.as_string().unwrap();

      let camera = self.rawloader.check_supported_with_everything(&make, &model, "")?;

      let moov_trak = self
        .bmff
        .filebox
        .moov
        .traks
        .get(raw_trak_id)
        .ok_or(format!("Unable to get MOOV trak {}", raw_trak_id))?;
      let co64 = moov_trak.mdia.minf.stbl.co64.as_ref().ok_or(format!("No co64 box found"))?;
      let stsz = &moov_trak.mdia.minf.stbl.stsz;

      let offset = *co64.entries.get(raw_image_id).ok_or(format!("image index {} is out of range", raw_image_id))? as usize;
      let size = *stsz
        .sample_sizes
        .get(raw_image_id)
        .ok_or(format!("image index {} is out of range", raw_image_id))? as usize;
      debug!("raw mdat offset: {}", offset);
      debug!("raw mdat size: {}", size);

      let buf = file
        .get_range(offset as u64, size as u64)
        .map_err(|e| RawlerError::General(format!("I/O error while reading CR3 raw image: {:?}", e)))?;

      let cmp1 = self.bmff.filebox.moov.traks[raw_trak_id]
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

      let image = if !dummy {
        decompress_crx_image(&buf, cmp1).map_err(|e| format!("Failed to decode raw: {}", e.to_string()))?
      } else {
        Vec::new()
      };

      let wb = self.wb.unwrap();
      let blacklevel = self.blacklevels.as_ref().unwrap();
      let cpp = 1;

      let mut img = RawImage::new(camera, cmp1.f_width as usize, cmp1.f_height as usize, cpp, wb, image, dummy);

      img.blacklevels = *blacklevel;
      img.whitelevels = [
        *self.whitelevel.as_ref().unwrap(),
        *self.whitelevel.as_ref().unwrap(),
        *self.whitelevel.as_ref().unwrap(),
        *self.whitelevel.as_ref().unwrap(),
      ];

      let iad1 = &self.bmff.filebox.moov.traks[raw_trak_id]
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
        let rect = Rect::new_with_points(
          Point::new(iad1_borders.crop_left_offset as usize, iad1_borders.crop_top_offset as usize),
          Point::new((iad1_borders.crop_right_offset + 1) as usize, (iad1_borders.crop_bottom_offset + 1) as usize),
        );
        img.crop_area = Some(rect);

        let _rect = Rect::new_with_points(
          Point::new(iad1_borders.active_area_left_offset as usize, iad1_borders.active_area_top_offset as usize),
          Point::new(
            (iad1_borders.active_area_right_offset - 1) as usize,
            (iad1_borders.active_area_bottom_offset - 1) as usize,
          ),
        );
        //img.active_area = Some(rect);
        img.active_area = img.crop_area;

        let blackarea_h = Rect::new_with_points(
          Point::new(iad1_borders.lob_left_offset as usize, iad1_borders.lob_top_offset as usize),
          Point::new((iad1_borders.lob_right_offset - 1) as usize, (iad1_borders.lob_bottom_offset - 1) as usize),
        );
        if !blackarea_h.is_empty() {
          //img.blackareas.push(blackarea_h);
        }
        let blackarea_v = Rect::new_with_points(
          Point::new(iad1_borders.tob_left_offset as usize, iad1_borders.tob_top_offset as usize),
          Point::new((iad1_borders.tob_right_offset - 1) as usize, (iad1_borders.tob_bottom_offset - 1) as usize),
        );
        if !blackarea_v.is_empty() {
          //img.blackareas.push(blackarea_v);
        }

        debug!("Canon active area: {:?}", img.active_area);
        debug!("Canon crop area: {:?}", img.crop_area);
        debug!("Black areas: {:?}", img.blackareas);

        /*
        img.crops = [
          iad1_borders.crop_top_offset as usize,                            // top
          (iad1.img_width - iad1_borders.crop_right_offset - 1) as usize,   // right
          (iad1.img_height - iad1_borders.crop_bottom_offset - 1) as usize, // bottom
          iad1_borders.crop_left_offset as usize,                           // left
        ];
         */
      }

      return Ok(img);
    } else {
      return Err(RawlerError::General(format!("Camera model unknown")));
    }
  }

  fn full_image(&self, file: &mut RawFile) -> Result<DynamicImage> {
    let offset = self.bmff.filebox.moov.traks[0].mdia.minf.stbl.co64.as_ref().expect("co64 box").entries[0] as usize;
    let size = self.bmff.filebox.moov.traks[0].mdia.minf.stbl.stsz.sample_sizes[0] as usize;
    debug!("jpeg mdat offset: {}", offset);
    debug!("jpeg mdat size: {}", size);
    //let mdat_data_offset = (self.bmff.filebox.mdat.header.offset + self.bmff.filebox.mdat.header.header_len) as usize;

    let buf = file
      .get_range(offset as u64, size as u64)
      .map_err(|e| RawlerError::General(format!("I/O error while reading CR3 full image: {:?}", e)))?;
    match image::load_from_memory_with_format(&buf, image::ImageFormat::Jpeg) {
      Ok(img) => Ok(img),
      Err(e) => {
        debug!("TRAK 0 contains no JPEG preview, is it PQ/HEIF? Error: {}", e);
        Err(RawlerError::General(
          "Unable to extract preview image from CR3 HDR-PQ file. Please see 'https://github.com/dnglab/dnglab/issues/7'".into(),
        ))
      }
    }
  }
}

impl<'a> Cr3Decoder<'a> {
  pub fn _new_makernote(buf: &'a [u8], offset: usize, base_offset: usize, chain_level: isize, e: Endian) -> Result<LegacyTiffIFD<'a>> {
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

    Ok(LegacyTiffIFD::new(buf, offset + off, base_offset, 0, chain_level + 1, endian, &vec![])?)
  }
}

#[derive(Clone, Debug)]
struct Ctmd {
  pub records: HashMap<u16, CtmdRecord>,
}

#[derive(Clone, Debug)]
struct CtmdRecord {
  #[allow(dead_code)]
  pub rec_size: u32,
  pub rec_type: u16,
  #[allow(dead_code)]
  pub reserved1: u8,
  #[allow(dead_code)]
  pub reserved2: u8,
  #[allow(dead_code)]
  pub reserved3: u16,
  #[allow(dead_code)]
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

  fn try_from(value: u16) -> std::result::Result<Self, Self::Error> {
    Self::n(value).ok_or(format!("Unable to convert tag: {}, not defined in enum", value))
  }
}

impl TiffTagEnum for Cr3MakernoteTag {}
