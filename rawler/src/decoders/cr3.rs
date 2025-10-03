// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use image::DynamicImage;
use log::{debug, warn};
use std::convert::{TryFrom, TryInto};
use std::fmt::Debug;

use crate::bits::Endian;
use crate::decoders::*;
use crate::decompressors::crx::decompress_crx_image;
use crate::envparams::{rawler_crx_raw_trak, rawler_ignore_previews};
use crate::exif::ExifGPS;
use crate::formats::bmff::FileBox;
use crate::formats::bmff::ext_cr3::cmp1::Cmp1Box;
use crate::formats::bmff::ext_cr3::cr3desc::Cr3DescBox;
use crate::formats::bmff::ext_cr3::iad1::{Iad1Box, Iad1Type};
use crate::formats::bmff::trak::TrakBox;
use crate::formats::tiff::reader::TiffReader;
use crate::formats::tiff::{Entry, GenericTiffReader, Rational, Value};
use crate::imgop::{Point, Rect};
use crate::lens::{LensDescription, LensId, LensResolver};
use crate::{RawImage, pumps::ByteStream};

const CANON_CN_MOUNT: &str = "cn-mount";
const CANON_EF_MOUNT: &str = "ef-mount";
const CANON_RF_MOUNT: &str = "rf-mount";

/// Decoder for CR3 and CRM files
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Cr3Decoder<'a> {
  camera: Camera,
  rawloader: &'a RawLoader,
  bmff: Bmff,
  // Basic EXIF information
  cmt1: GenericTiffReader,
  // EXIF
  cmt2: GenericTiffReader,
  // Makernotes
  cmt3: GenericTiffReader,
  // GPS
  cmt4: GenericTiffReader,
  // Metadata cache
  md_cache: DecoderCache<Cr3Metadata>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
struct Cr3Metadata {
  ctmd_exposure: Option<CtmdExposureInfo>,
  ctmd_focallen: Option<Rational>,
  ctmd_rec7_exif: Option<GenericTiffReader>,
  ctmd_rec7_makernotes: Option<GenericTiffReader>,
  // CTMD Makernotes: COLORDATA
  ctmd_rec8: Option<GenericTiffReader>,
  ctmd_rec9: Option<GenericTiffReader>,
  xpacket: Option<Vec<u8>>,
  image_unique_id: Option<[u8; 16]>,
  lens_description: Option<&'static LensDescription>,
  exif: Option<GenericTiffReader>,
  makernotes: Option<GenericTiffReader>,
  wb: Option<[f32; 4]>,
  blacklevels: Option<[u16; 4]>,
  whitelevel: Option<u16>,
}

#[allow(dead_code)]
const CR3_CTMD_BLOCK_EXIFIFD: u16 = 0x8769;
const CR3_CTMD_BLOCK_MAKERNOTES: u16 = 0x927c;

/// Type values fro CCTP records
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
enum Cr3ImageType {
  CrxBix = 0,
  CrxSmall = 1,
  PreviewBig = 2,
  Ctmd = 3,
  CrxDual = 4,
}

impl<'a> Cr3Decoder<'a> {
  /// Construct new CR3 or CRM deocder
  pub fn new(_rawfile: &RawSource, bmff: Bmff, rawloader: &'a RawLoader) -> Result<Cr3Decoder<'a>> {
    if let Some(Cr3DescBox { cmt1, cmt2, cmt3, cmt4, .. }) = bmff.filebox.moov.cr3desc.as_ref() {
      let mode = Self::get_mode(cmt3.tiff.root_ifd())?;
      let camera = rawloader.check_supported_with_mode(cmt1.tiff.root_ifd(), mode)?;
      let decoder = Cr3Decoder {
        camera,
        rawloader,
        cmt1: cmt1.tiff.clone(),
        cmt2: cmt2.tiff.clone(),
        cmt3: cmt3.tiff.clone(),
        cmt4: cmt4.tiff.clone(),
        bmff,
        md_cache: DecoderCache::new(),
      };
      Ok(decoder)
    } else {
      Err("It's not a CR3 file: CMT boxes not found.".into())
    }
  }

  // Search for quality tag inside makernotes and derive
  // our mode string for configuration.
  fn get_mode(makernotes: &IFD) -> Result<&str> {
    Ok(if let Some(entry) = makernotes.get_entry(0x0001) {
      match entry.force_u16(3) {
        4 => "raw",
        7 => "craw",
        130 => "crm",
        131 => "crm",
        _ => "undefined",
      }
    } else {
      "undefined"
    })
  }

  /// Get trak from moov box
  fn moov_trak(&self, trak_id: usize) -> Option<&TrakBox> {
    self.bmff.filebox.moov.traks.get(trak_id)
  }

  /// Get IAD1 box for specific trak
  fn iad1_box(&self, trak_idx: usize) -> Option<&Iad1Box> {
    let trak = &self.bmff.filebox.moov.traks[trak_idx];
    let craw = trak.mdia.minf.stbl.stsd.craw.as_ref();
    craw.and_then(|craw| craw.cdi1.as_ref()).map(|cdi1| &cdi1.iad1)
  }

  /// Get CMP1 box for specific trak
  fn cmp1_box(&self, trak_idx: usize) -> Option<&Cmp1Box> {
    let trak = &self.bmff.filebox.moov.traks[trak_idx];
    let craw = trak.mdia.minf.stbl.stsd.craw.as_ref();
    craw.and_then(|craw| craw.cmp1.as_ref())
  }

  /// Read CTMD records for given sample
  /// Each sample (for movie files) have their own CTMD records
  fn read_ctmd(&self, rawfile: &RawSource, sample_idx: u32) -> Result<Option<Ctmd>> {
    // Search for a trak which has a CTMD box (there should be only one)
    if let Some(ctmd_trak_index) = self
      .bmff
      .filebox
      .moov
      .traks
      .iter()
      .enumerate()
      .find(|(_, trak)| trak.mdia.minf.stbl.stsd.ctmd.is_some())
      .map(|(id, _)| id)
    {
      log::debug!("CTMD trak_index: {}", ctmd_trak_index);
      let ctmd_trak = &self.bmff.filebox.moov.traks[ctmd_trak_index];
      let (offset, size) = ctmd_trak
        .mdia
        .minf
        .stbl
        .get_sample_offset(sample_idx as u32 + 1)
        .ok_or_else(|| RawlerError::DecoderFailed(format!("CTMD sample index {} out of bound", sample_idx)))?;
      debug!("CR3 CTMD mdat offset for sample_idx {}: {}, len: {}", sample_idx, offset, size);
      let buf = rawfile
        .subview(offset as u64, size as u64)
        .map_err(|e| RawlerError::with_io_error("CR3: failed to read CTMD", rawfile.path(), e))?;

      //dump_buf("/tmp/ctmd.buf", &buf);
      let mut substream = ByteStream::new(buf, Endian::Little);
      let ctmd = Ctmd::new(&mut substream);
      Ok(Some(ctmd))
    } else {
      log::warn!("No CTMD trak found");
      Ok(None)
    }
  }
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

  fn raw_metadata(&self, file: &RawSource, params: &RawDecodeParams) -> Result<RawMetadata> {
    let cr3md = self.read_cr3_metadata(file, params)?;

    let mut exif = Exif::default();
    exif.extend_from_ifd(self.cmt1.root_ifd())?;
    exif.extend_from_ifd(self.cmt2.root_ifd())?;
    exif.extend_from_gps_ifd(self.cmt4.root_ifd())?;

    if exif.gps.is_none() {
      exif.gps = Some(ExifGPS::default());
    }

    if let Some(gps) = &mut exif.gps {
      for (tag, entry) in self.cmt4.root_ifd().entries() {
        match tag {
          // Special handling for Exif.GPSInfo.GPSLatitude and Exif.GPSInfo.GPSLongitude.
          // Exif.GPSInfo.GPSTimeStamp is wrong, too and can be fixed with the same logic.
          // Canon CR3 contains only two rationals, but these tags are specified as a vector
          // of three reationals (degrees, minutes, seconds).
          // We fix this by extending with 0/1 as seconds value.
          0x0002 | 0x0004 | 0x0007 => match &entry.value {
            Value::Rational(v) => {
              let fixed_value = if v.len() == 2 { vec![v[0], v[1], Rational::new(0, 1)] } else { v.clone() };
              match tag {
                0x0002 => gps.gps_latitude = fixed_value.try_into().ok(),
                0x0004 => gps.gps_longitude = fixed_value.try_into().ok(),
                0x0007 => gps.gps_timestamp = fixed_value.try_into().ok(),
                _ => unreachable!(),
              }
            }
            _ => {
              warn!("CR3: Exif.GPSInfo.GPSLatitude and Exif.GPSInfo.GPSLongitude expected to be of type RATIONAL, GPS data is ignored");
            }
          },
          _ => {}
        }
      }
    }

    let mut mdata = RawMetadata::new_with_lens(&self.camera, exif, cr3md.lens_description.cloned());

    if let Some(unique_id) = &cr3md.image_unique_id {
      // For CR3, we use the already included Makernote tag with unique image ID
      mdata.unique_image_id = Some(u128::from_le_bytes(*unique_id));
    }

    Ok(mdata)
  }

  fn xpacket(&self, file: &RawSource, params: &RawDecodeParams) -> Result<Option<Vec<u8>>> {
    let cr3md = self.read_cr3_metadata(file, params)?;
    Ok(cr3md.xpacket)
  }

  /// CR3 can store multiple samples in trak
  fn raw_image_count(&self) -> Result<usize> {
    let raw_trak_id = rawler_crx_raw_trak()
      .or_else(|| self.get_trak_index(Cr3ImageType::CrxBix))
      .ok_or("Unable to find trak index")?;
    let moov_trak = self.moov_trak(raw_trak_id).ok_or(format!("Unable to get MOOV trak {}", raw_trak_id))?;
    Ok(moov_trak.mdia.minf.stbl.stsz.sample_count as usize)
  }

  /// Decode raw image
  fn raw_image(&self, file: &RawSource, params: &RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let sample_idx = params.image_index;
    if sample_idx >= self.raw_image_count()? {
      return Err(RawlerError::DecoderFailed(format!(
        "Raw image index {} out of range ({})",
        sample_idx,
        self.raw_image_count()?
      )));
    }

    let cr3md = self.read_cr3_metadata(file, params)?;

    if let Some(cr3desc) = &self.bmff.filebox.moov.cr3desc {
      for item in cr3desc.cctp.ccdts.iter() {
        log::debug!("CCDT: trak {} type {} dual {}", item.trak_index, item.image_type, item.dual_pixel);
      }
    }

    let raw_trak_id = rawler_crx_raw_trak()
      .or_else(|| self.get_trak_index(Cr3ImageType::CrxBix))
      .ok_or("Unable to find trak index")?;

    // Load trak with raw MDAT section
    let moov_trak = self.moov_trak(raw_trak_id).ok_or(format!("Unable to get MOOV trak {}", raw_trak_id))?;
    let (offset, size) = moov_trak
      .mdia
      .minf
      .stbl
      .get_sample_offset(sample_idx as u32 + 1)
      .ok_or_else(|| RawlerError::DecoderFailed(format!("stbl sample not found")))?;
    debug!("RAW mdat offset: {}, len: {}", offset, size);
    // Raw data buffer
    let buf = file
      .subview(offset as u64, size as u64)
      .map_err(|e| RawlerError::with_io_error("CR3: failed to read raw data", file.path(), e))?;

    let cmp1 = self.cmp1_box(raw_trak_id).ok_or(format!("CMP1 box not found for trak {}", raw_trak_id))?;
    debug!("cmp1 mdat hdr size: {}", cmp1.mdat_hdr_size);

    let mut wb = cr3md.wb.unwrap_or_else(|| {
      // This is known for R5 C CRM Standard-Raw files
      log::warn!("No WB info in CR3 metadata found, fallback to 1.0 coefficients");
      [1.0, 1.0, 1.0, f32::NAN]
    });
    let whitelevel = cr3md.whitelevel.unwrap_or(((1_u32 << self.camera.bps.unwrap_or(16)) - 1) as u16);

    // Special handling for CRM movie files
    if let Some(entry) = self.cmt3.get_entry(0x0001) {
      if 130 == entry.force_u16(3) {
        // Light Raw
        // WB is already applied, use 1.0
        wb = [1.0, 1.0, 1.0, f32::NAN];
      }
      if 131 == entry.force_u16(3) { // Standard Raw
        // Nothing special for Standard raw
      }
    }

    let image = if !dummy {
      PixU16::new_with(
        decompress_crx_image(buf, cmp1).map_err(|e| format!("Failed to decode raw: {}", e))?,
        cmp1.f_width as usize,
        cmp1.f_height as usize,
      )
    } else {
      PixU16::new_uninit(cmp1.f_width as usize, cmp1.f_height as usize)
    };

    let cpp = 1;
    let blacklevel = cr3md
      .blacklevels
      .map(|x| BlackLevel::new(&x, self.camera.cfa.width, self.camera.cfa.height, cpp));
    let whitelevel = WhiteLevel(vec![whitelevel as u32]);
    let photometric = RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&self.camera));
    let mut img = RawImage::new(self.camera.clone(), image, cpp, wb, photometric, blacklevel, Some(whitelevel), dummy);

    // IAD1 box contains sensor information
    // We use the sensor crop from IAD1 as recommended image crop.
    // The same crop is used as ActiveArea, because black areas in IAD1 are not
    // correct (they differs like 4-6 pixels from real values).
    match self.iad1_box(raw_trak_id) {
      Some(iad1) => {
        match &iad1.iad1_type {
          // IAD1 (small, used for CRM movie files)
          Iad1Type::Small(small) => {
            img.crop_area = Some(Rect::new_with_points(
              Point::new(small.crop_left_offset as usize, small.crop_top_offset as usize),
              Point::new((small.crop_right_offset + 1) as usize, (small.crop_bottom_offset + 1) as usize),
            ));
            img.active_area = img.crop_area;
          }
          // IAD1 (big, used for full size raws)
          Iad1Type::Big(big) => {
            img.crop_area = Some(Rect::new_with_points(
              Point::new(big.crop_left_offset as usize, big.crop_top_offset as usize),
              Point::new((big.crop_right_offset + 1) as usize, (big.crop_bottom_offset + 1) as usize),
            ));

            // For uncropped files this is fine, but for 1.6 crop files, the dimension is wrong.
            // For example, R5 crop is total height of 3510, but active_area_bottom_offset is 3512.
            let rect = {
              // Limit the offsets to image bounds.
              // Probably broken firmware, glitches in sensor size calculation or I'm just making
              // wrong asumptions...
              let right = usize::min(cmp1.f_width as usize, (big.active_area_right_offset - 1) as usize);
              let bottom = usize::min(cmp1.f_height as usize, (big.active_area_bottom_offset - 1) as usize);
              Rect::new_with_points(
                Point::new(big.active_area_left_offset as usize, big.active_area_top_offset as usize),
                Point::new(right, bottom),
              )
            };
            log::debug!("IAD1 active area: {:?}", rect);
            img.active_area = Some(rect);
            //img.active_area = img.crop_area;

            let blackarea_h = Rect::new_with_points(
              Point::new(big.lob_left_offset as usize, big.lob_top_offset as usize),
              Point::new((big.lob_right_offset - 1) as usize, (big.lob_bottom_offset - 1) as usize),
            );
            if !blackarea_h.is_empty() {
              //img.blackareas.push(blackarea_h);
            }
            let blackarea_v = Rect::new_with_points(
              Point::new(big.tob_left_offset as usize, big.tob_top_offset as usize),
              Point::new((big.tob_right_offset - 1) as usize, (big.tob_bottom_offset - 1) as usize),
            );
            if !blackarea_v.is_empty() {
              //img.blackareas.push(blackarea_v);
            }
          }
        }
      }

      None => {
        warn!("No IAD1 box found for sensor data");
      }
    }
    debug!("Canon active area: {:?}", img.active_area);
    debug!("Canon crop area: {:?}", img.crop_area);
    debug!("Black areas: {:?}", img.blackareas);
    Ok(img)
  }

  /// Extract preview image embedded in CR3
  fn full_image(&self, file: &RawSource, params: &RawDecodeParams) -> Result<Option<DynamicImage>> {
    if params.image_index != 0 {
      return Ok(None);
    }
    if rawler_ignore_previews() {
      return Err(RawlerError::DecoderFailed("Unable to extract preview image".into()));
    }
    let offset = self.bmff.filebox.moov.traks[0].mdia.minf.stbl.co64.as_ref().expect("co64 box").entries[0] as usize;
    let size = self.bmff.filebox.moov.traks[0].mdia.minf.stbl.stsz.sample_sizes[0] as usize;
    debug!("JPEG preview mdat offset: {}, len: {}", offset, size);
    let buf = file
      .subview(offset as u64, size as u64)
      .map_err(|e| RawlerError::with_io_error("CR3: failed to read full image data", file.path(), e))?;
    match image::load_from_memory_with_format(buf, image::ImageFormat::Jpeg) {
      Ok(img) => Ok(Some(img)),
      Err(e) => {
        debug!("TRAK 0 contains no JPEG preview, is it PQ/HEIF? Error: {}", e);
        Err(RawlerError::DecoderFailed(
          "Unable to extract preview image from CR3 HDR-PQ file. Please see 'https://github.com/dnglab/dnglab/issues/7'".into(),
        ))
      }
    }
  }

  fn format_hint(&self) -> FormatHint {
    FormatHint::CR3
  }
}

impl<'a> Cr3Decoder<'a> {
  fn read_lens_id(&self) -> Result<LensId> {
    let mut id = (0, 0);
    if let Some(Entry { value: Value::Short(v), .. }) = self.cmt3.get_entry(Cr3MakernoteTag::CameraSettings) {
      id.0 = v[22] as u32;
    }

    if let Some(Entry { value: Value::Short(v), .. }) = self.cmt3.get_entry(Cr3MakernoteTag::FileInfo) {
      id.1 = v[61] as u32;
    }
    Ok(id)
  }

  fn read_lens_name(&self) -> Result<Option<&String>> {
    if let Some(Entry {
      value: crate::formats::tiff::Value::Ascii(lens_id),
      ..
    }) = self.cmt2.get_entry(ExifTag::LensModel)
    {
      return Ok(lens_id.strings().get(0));
    }
    Ok(None)
  }

  fn read_cr3_metadata(&self, rawfile: &RawSource, params: &RawDecodeParams) -> Result<Cr3Metadata> {
    if let Some(md) = self.md_cache.get(params) {
      return Ok(md);
    }
    let mut md = Cr3Metadata::default();

    let resolver = LensResolver::new()
      .with_lens_keyname(self.read_lens_name()?)
      .with_camera(&self.camera) // must follow with_lens_keyname() as it my override key
      .with_lens_id(self.read_lens_id()?)
      .with_mounts(&[CANON_CN_MOUNT.into(), CANON_EF_MOUNT.into(), CANON_RF_MOUNT.into()]);
    md.lens_description = resolver.resolve();

    if let Some(Entry {
      value: crate::formats::tiff::Value::Byte(v),
      ..
    }) = self.cmt3.get_entry(Cr3MakernoteTag::ImgUniqueID)
    {
      if v.len() == 16 {
        debug!("CR3 makernote ImgUniqueID: {:x?}", v);
        md.image_unique_id = Some(v.as_slice().try_into().expect("Invalid slice size"));
      }
    }

    if let Some(entry) = self.cmt3.get_entry(0x0001) {
      let quality = match entry.force_u16(3) {
        4 => "RAW",
        5 => "Superfine",
        7 => "CRAW",
        130 => "LightRaw",
        131 => "StandardRaw",
        _ => "unknown",
      };
      debug!("Canon quality mode: {}", quality);
    }

    if let Some(entry) = self.cmt3.get_entry(0x4026) {
      let clog = match entry.force_u32(11) {
        0 => "OFF",
        1 => "CLog1",
        2 => "CLog2",
        3 => "CLog3",
        _ => "unknown",
      };
      debug!("Canon CLog mode: {}", clog);
    }

    if let Some(xpacket_box) = self.bmff.filebox.cr3xpacket.as_ref() {
      let offset = xpacket_box.header.offset + xpacket_box.header.header_len;
      let size = xpacket_box.header.size - xpacket_box.header.header_len;
      let buf = rawfile
        .subview(offset, size)
        .map_err(|e| RawlerError::with_io_error("CR3: failed to read XPACKET", rawfile.path(), e))?;
      md.xpacket = Some(buf.to_vec());
    }

    if let Some(ctmd) = self.read_ctmd(rawfile, params.image_index as u32)? {
      if let Some(rec5) = ctmd.exposure_info()? {
        debug!("CTMD Rec(5): {:?}", rec5);
        md.ctmd_exposure = Some(rec5);
      }
      md.ctmd_focallen = ctmd.focal_len()?;

      if let Some(rec7) = ctmd.get_as_tiff(7, CR3_CTMD_BLOCK_EXIFIFD)? {
        md.ctmd_rec7_exif = Some(rec7);
      }
      if let Some(rec7) = ctmd.get_as_tiff(7, CR3_CTMD_BLOCK_MAKERNOTES)? {
        md.ctmd_rec7_makernotes = Some(rec7);
      }
      if let Some(rec8) = ctmd.get_as_tiff(8, CR3_CTMD_BLOCK_MAKERNOTES)? {
        if let Some(colordata) = rec8.get_entry(Cr3MakernoteTag::ColorData) {
          let colordata = cr2::parse_colordata(colordata)?;
          //rec8.root_ifd().dump::<TiffCommonTag>(10).iter().for_each(|line| eprintln!("MKD: {}", line));
          md.wb = Some(normalize_wb(colordata.wb));
          md.blacklevels = colordata.blacklevel;
          md.whitelevel = colordata.specular_whitelevel;
          /*
          if let crate::formats::tiff::Value::Short(v) = &levels.value {
            if let Some(offset) = self.camera.param_usize("colordata_wbcoeffs") {
              let raw_wb = [v[offset] as f32, v[offset + 1] as f32, v[offset + 2] as f32, v[offset + 3] as f32];
              md.wb = Some(normalize_wb(raw_wb));
            }
            if let Some(offset) = self.camera.param_usize("colordata_blacklevel") {
              debug!("Blacklevel offset: {:x}", offset);
              md.blacklevels = Some([v[offset], v[offset + 1], v[offset + 2], v[offset + 3]]);
            }
            if let Some(offset) = self.camera.param_usize("colordata_whitelevel") {
              md.whitelevel = Some(v[offset]);
            }
          }
           */
        }
        md.ctmd_rec8 = Some(rec8);
      }
      if let Some(rec9) = ctmd.get_as_tiff(9, CR3_CTMD_BLOCK_MAKERNOTES)? {
        md.ctmd_rec9 = Some(rec9);
      }
    }

    debug!("CR3 blacklevels: {:?}", md.blacklevels);
    debug!("CR3 whitelevel: {:?}", md.whitelevel);

    self.md_cache.set(params, md.clone());
    Ok(md)
  }

  fn get_trak_index(&self, image_type: Cr3ImageType) -> Option<usize> {
    if let Some(cr3desc) = &self.bmff.filebox.moov.cr3desc {
      cr3desc.cctp.ccdts.iter().find(|ccdt| ccdt.image_type == image_type as u64).map(|rec| {
        debug_assert!(rec.trak_index > 0);
        (rec.trak_index - 1) as usize
      })
    } else {
      None
    }
  }
}

/// CTMD section with multiple records
#[derive(Clone, Debug)]
struct Ctmd {
  pub records: HashMap<u16, CtmdRecord>,
}

/// Record inside CTMD section
#[allow(dead_code)]
#[derive(Clone, Debug)]
struct CtmdRecord {
  pub rec_size: u32,
  pub rec_type: u16,
  pub reserved1: u8,
  pub reserved2: u8,
  pub reserved3: u8,
  pub reserved4: u8,
  pub reserved5: i8,
  pub reserved6: i8,
  pub payload: Vec<u8>,
  pub blocks: HashMap<u16, Vec<u8>>,
}

impl Ctmd {
  pub fn new(data: &mut ByteStream) -> Self {
    let mut records = HashMap::new();

    while data.remaining_bytes() >= 12 {
      let size = data.get_u32();
      let mut rec = CtmdRecord {
        rec_size: size,
        rec_type: data.get_u16(),
        reserved1: data.get_u8(),
        reserved2: data.get_u8(),
        reserved3: data.get_u8(),
        reserved4: data.get_u8(),
        reserved5: data.get_i8(),
        reserved6: data.get_i8(),
        payload: data.get_bytes(size as usize - 12),
        blocks: HashMap::new(),
      };
      debug!(
        "CTMD Rec {:02}:  {}, {}, {}, {}, {}, {}",
        rec.rec_type, rec.reserved1, rec.reserved2, rec.reserved3, rec.reserved4, rec.reserved5, rec.reserved6
      );
      //dump_buf(&format!("/tmp/ctmd_rec{}.bin", rec.rec_type), rec.payload.as_slice());
      if [7, 8, 9, 10, 11, 12].contains(&rec.rec_type) {
        let mut bs = ByteStream::new(rec.payload.as_slice(), Endian::Little);
        let mut _block_id = 0;
        while bs.remaining_bytes() >= (4 + 2 + 2) {
          let sz = bs.get_u32() as usize;
          let tag = bs.get_u16();
          let _uk = bs.get_u16();
          if sz >= 8 && bs.remaining_bytes() >= (sz - 8) {
            log::debug!("CTMD BLOCK: size {}, tag {}, remaining: {}", sz, tag, bs.remaining_bytes());
            let data = bs.get_bytes(sz as usize - 8);
            //dump_buf(&format!("/tmp/ctmd_rec{}_block{}_tag0x{:X}_uk{:X}.bin", rec.rec_type, block_id, tag, uk), data.as_slice());
            if [CR3_CTMD_BLOCK_EXIFIFD, CR3_CTMD_BLOCK_MAKERNOTES].contains(&tag) {
              assert_eq!(rec.blocks.contains_key(&tag), false, "Double tag found?!");
              rec.blocks.insert(tag, data);
            }
            _block_id += 1;
          } else {
            log::debug!(
              "CTMD BLOCK: size {}, tag {}, remaining: {} - is invalid, maybe not a block",
              sz,
              tag,
              bs.remaining_bytes()
            );
            break;
          }
        }
      } else {
        log::debug!("CTMD record type {} unknown, ignoring.", rec.rec_type);
        //dump_buf(&format!("/tmp/ctmd_rec{}.bin", rec.rec_type), rec.payload.as_slice());
      }
      records.insert(rec.rec_type, rec);
    }
    Self { records }
  }

  pub fn get_as_tiff(&self, record: u16, tag: u16) -> Result<Option<GenericTiffReader>> {
    if let Some(block) = self.records.get(&record).and_then(|rec| rec.blocks.get(&tag)) {
      Ok(Some(GenericTiffReader::new_with_buffer(block, 0, 0, Some(0))?))
    } else {
      warn!("Unable to find CTMD record {}, tag 0x{:X}", record, tag);
      Ok(None)
    }
  }

  pub fn exposure_info(&self) -> Result<Option<CtmdExposureInfo>> {
    if let Some(rec) = self.records.get(&5) {
      let mut buf = ByteStream::new(rec.payload.as_slice(), Endian::Little);
      let fnumber = Rational::new(buf.get_u16().into(), buf.get_u16().into());
      let exposure = Rational::new(buf.get_u16().into(), buf.get_u16().into());
      let iso_speed = buf.get_u32();
      let unknown = buf.get_bytes(buf.remaining_bytes());
      Ok(Some(CtmdExposureInfo {
        fnumber,
        exposure,
        iso_speed,
        unknown,
      }))
    } else {
      Ok(None)
    }
  }

  pub fn focal_len(&self) -> Result<Option<Rational>> {
    if let Some(rec) = self.records.get(&4) {
      let mut buf = ByteStream::new(rec.payload.as_slice(), Endian::Little);
      let focal_len = Rational::new(buf.get_u16().into(), buf.get_u16().into());
      Ok(Some(focal_len))
    } else {
      Ok(None)
    }
  }
}

fn normalize_wb(raw_wb: [f32; 4]) -> [f32; 4] {
  debug!("CR3 raw wb: {:?}", raw_wb);
  // We never have more then RGB colors so far (no RGBE etc.)
  // So we combine G1 and G2 to get RGB wb.
  let div = raw_wb[1]; // G1 should be 1024 and we use this as divisor
  let mut norm = raw_wb;
  norm.iter_mut().for_each(|v| {
    if v.is_normal() {
      *v /= div
    }
  });
  [norm[0], (norm[1] + norm[2]) / 2.0, norm[3], f32::NAN]
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CtmdExposureInfo {
  pub fnumber: Rational,
  pub exposure: Rational,
  pub iso_speed: u32,
  pub unknown: Vec<u8>,
}

crate::tags::tiff_tag_enum!(Cr3MakernoteTag);

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
