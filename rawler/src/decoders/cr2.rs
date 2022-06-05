// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use core::panic;
use image::DynamicImage;
use image::ImageBuffer;
use image::Rgb;
use log::debug;
use log::info;
use rayon::iter::ParallelIterator;
use rayon::slice::ParallelSliceMut;
use serde::Deserialize;
use serde::Serialize;
use std::convert::TryFrom;
use std::f32::NAN;

use crate::alloc_image_plain;
use crate::analyze::FormatDump;
use crate::bits::clampbits;
use crate::bits::LookupTable;
use crate::decompressors::ljpeg::*;
use crate::exif::Exif;
use crate::formats::tiff::reader::TiffReader;
use crate::formats::tiff::Entry;
use crate::formats::tiff::GenericTiffReader;
use crate::formats::tiff::Rational;
use crate::formats::tiff::Value;
use crate::formats::tiff::IFD;
use crate::imgop::Dim2;
use crate::imgop::Point;
use crate::imgop::Rect;
use crate::lens::LensDescription;
use crate::lens::LensResolver;
use crate::tags::ExifTag;
use crate::tags::TiffCommonTag;
use crate::RawFile;
use crate::RawImage;
use crate::RawLoader;
use crate::RawlerError;

use super::Camera;
use super::Decoder;
use super::RawDecodeParams;
use super::RawMetadata;
use super::Result;

const CANON_EF_MOUNT: &str = "ef-mount";
const CANON_CN_MOUNT: &str = "cn-mount";

/// CR2 Decoder
pub struct Cr2Decoder<'a> {
  #[allow(dead_code)]
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
  exif: IFD,
  makernote: Option<IFD>,
  #[allow(dead_code)]
  mode: Cr2Mode,
  xpacket: Option<Vec<u8>>,
  camera: Camera,
  model_id: Option<u32>,
}

/// CR2 format encapsulation for analyzer
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cr2Format {
  tiff: GenericTiffReader,
}

impl<'a> Decoder for Cr2Decoder<'a> {
  fn format_dump(&self) -> FormatDump {
    FormatDump::Cr2(Cr2Format { tiff: self.tiff.clone() })
  }

  fn raw_image(&self, file: &mut RawFile, _params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    /*
    for (i, ifd) in self.tiff.chains().iter().enumerate() {
      eprintln!("IFD {}", i);
      for line in ifd_dump::<crate::tags::LegacyTiffRootTag>(ifd, 10) {
        eprintln!("{}", line);
      }
    }
     */

    let camera = &self.camera;
    let (raw, offset) = {
      if let Some(raw) = self.tiff.find_first_ifd(TiffCommonTag::Cr2Id) {
        (raw, fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_usize(0))
      } else if let Some(raw) = self.tiff.find_first_ifd(TiffCommonTag::CFAPattern) {
        (raw, fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_usize(0))
      } else if let Some(off) = self.tiff.root_ifd().get_entry(TiffCommonTag::Cr2OldOffset) {
        (self.tiff.root_ifd(), off.value.force_usize(0))
      } else {
        return Err(RawlerError::General("CR2: Couldn't find raw info".to_string()));
      }
    };

    // We don't have an excact length, so read until end.
    let src = file
      .stream_len()
      .and_then(|len| file.subview(offset as u64, len - offset as u64))
      .map_err(|e| RawlerError::General(format!("I/O error: failed to read raw data from file: {}", e)))?;

    let (width, height, cpp, image) = {
      let decompressor = LjpegDecompressor::new(&src)?;
      let ljpegwidth = decompressor.width();
      let mut width = ljpegwidth;
      let mut height = decompressor.height();
      let cpp = if decompressor.super_h() == 2 { 3 } else { 1 };
      debug!("CR2 ljpeg components: {}", decompressor.components());
      debug!("CR2 final cpp: {}", cpp);
      debug!("CR2 dimension: {},{}", width / cpp, height);
      let mut ljpegout = alloc_image_plain!(width, height, dummy);

      decompressor.decode(ljpegout.pixels_mut(), 0, width, width, height, dummy)?;

      //crate::devtools::dump_image_u16(&ljpegout, width, height, "/tmp/cr2_before_striped.pnm");

      // Linearize the output (applies only to D2000 as far as I can tell)
      if !dummy && camera.find_hint("linearization") {
        let table = {
          let linearization = fetch_tiff_tag!(raw, TiffCommonTag::GrayResponse);
          let mut t = [0_u16; 4096];
          for i in 0..t.len() {
            t[i] = linearization.force_u16(i);
          }
          LookupTable::new(&t)
        };

        let mut random = ljpegout[0] as u32;
        for p in ljpegout.pixels_mut().iter_mut() {
          *p = table.dither(*p, &mut random);
        }
      }

      if cpp == 3 {
        if raw.has_entry(TiffCommonTag::ImageWidth) {
          width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0) * cpp;
          height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
        } else if width / cpp < height {
          let temp = width / cpp;
          width = height * cpp;
          height = temp;
        }
      } else if camera.find_hint("double_line") {
        width /= 2;
        height *= 2;
      }

      debug!("CR2 dimension2: {},{}", width / cpp, height);

      // Take each of the vertical fields and put them into the right location
      // FIXME: Doing this at the decode would reduce about 5% in runtime but I haven't
      //        been able to do it without hairy code
      if let Some(canoncol) = raw.get_entry(TiffCommonTag::Cr2StripeWidths) {
        debug!("Found Cr2StripeWidths tag: {:?}", canoncol.value);
        if canoncol.value.force_usize(0) == 0 {
          if cpp == 3 {
            self.convert_to_rgb(file, camera, &decompressor, width, height, ljpegout.pixels_mut(), dummy)?;
            width /= 3;
          }
          (width, height, cpp, ljpegout)
        } else {
          let mut out = alloc_image_plain!(width, height, dummy);
          if !dummy {
            let mut fieldwidths = Vec::new();
            debug_assert!(canoncol.value.force_usize(0) > 0);
            debug_assert!(canoncol.value.force_usize(1) > 0);
            debug_assert!(canoncol.value.force_usize(2) > 0);
            for _ in 0..canoncol.value.force_usize(0) {
              fieldwidths.push(canoncol.value.force_usize(1));
            }
            fieldwidths.push(canoncol.value.force_usize(2));

            if decompressor.super_v() == 2 {
              debug!("CR2 v=2 decoder used, h={}", decompressor.super_h());
              // We've decoded 2 lines at a time so we also need to copy two strips at a time
              let nfields = fieldwidths.len();
              let fieldwidth = fieldwidths[0];
              let mut fieldstart = 0;
              let mut inpos = 0;
              for _ in 0..nfields {
                for row in (0..height).step_by(2) {
                  for col in (0..fieldwidth).step_by(3) {
                    let outpos = row * width + fieldstart + col;
                    out[outpos..outpos + 3].copy_from_slice(&ljpegout[inpos..inpos + 3]);
                    let outpos = (row + 1) * width + fieldstart + col;
                    let inpos2 = inpos + ljpegwidth;
                    out[outpos..outpos + 3].copy_from_slice(&ljpegout[inpos2..inpos2 + 3]);
                    inpos += 3;
                    if inpos % ljpegwidth == 0 {
                      // we've used a full input line and we're reading 2 by 2 so skip one
                      inpos += ljpegwidth;
                    }
                  }
                }
                fieldstart += fieldwidth;
              }
            } else {
              let sh = decompressor.super_h();
              debug!("CR2 v=1 decoder used, super_h: {}", sh);
              let mut fieldstart = 0;
              let mut fieldpos = 0;
              for fieldwidth in fieldwidths {
                // fix the inconsistent slice width in sRaw mode, ask Canon.
                let fieldwidth = fieldwidth / sh * cpp;
                // The output for full height of a vertical stripe is
                // composed by the lines of all input stripes N:
                // outb(line0) = slice[0](line[0])
                // outb(line1) = slice[1](line[0])
                // outb(line2) = slice[N-1](line[0])
                for row in 0..height {
                  let outpos = row * width + fieldstart;
                  let inpos = fieldpos + row * fieldwidth;
                  let outb = &mut out[outpos..outpos + fieldwidth];
                  let inb = &ljpegout[inpos..inpos + fieldwidth];
                  outb.copy_from_slice(inb);
                }
                fieldstart += fieldwidth;
                fieldpos += fieldwidth * height;
              }
            }
          }
          if cpp == 3 {
            self.convert_to_rgb(file, camera, &decompressor, width, height, out.pixels_mut(), dummy)?;
            width /= 3;
          }
          (width, height, cpp, out)
        }
      } else {
        (width, height, cpp, ljpegout)
      }
    };

    let wb = self.get_wb(file, camera)?;
    debug!("CR2 WB: {:?}", wb);
    assert_eq!(image.width, width * cpp);
    let mut img = RawImage::new(camera.clone(), cpp, wb, image, dummy);

    img.crop_area = Some(self.get_sensor_area(camera, width, height)?);
    if let Some(forced_area) = camera.crop_area {
      let area = Rect::new_with_borders(Dim2::new(width, height), &forced_area);
      debug!("Metadata says crop area is: {:?}, overriding with forced: {:?}", img.crop_area, area);
      img.crop_area = Some(area);
    }

    img.blacklevels = self.get_blacklevel(file, camera)?;
    img.whitelevels = self.get_whitelevel(file, camera)?;
    if cpp == 3 {
      img.cpp = 3;
      img.crop_area = None;
      //img.blacklevels = [0, 0, 0, 0];
      //img.whitelevels = [65535, 65535, 65535, 65535];
    }

    debug!("Black areas: {:?}", img.blackareas);
    debug!("Active area: {:?}", img.active_area);
    debug!("Crop area: {:?}", img.crop_area);
    Ok(img)
  }

  fn raw_metadata(&self, _file: &mut RawFile, _params: RawDecodeParams) -> Result<RawMetadata> {
    let exif = Exif::new(self.tiff.root_ifd())?;
    let mdata = RawMetadata::new_with_lens(&self.camera, exif, self.get_lens_description()?.cloned());
    Ok(mdata)
  }

  fn xpacket(&self, _file: &mut RawFile, _params: RawDecodeParams) -> Result<Option<Vec<u8>>> {
    Ok(self.xpacket.clone())
  }

  fn full_image(&self, file: &mut RawFile) -> Result<Option<DynamicImage>> {
    // For CR2, there is a full resolution image in IFD0.
    // This is compressed with old-JPEG compression (Compression = 6)
    let root_ifd = &self.tiff.root_ifd();
    let buf = root_ifd
      .singlestrip_data(file.inner())
      .map_err(|e| RawlerError::General(format!("Failed to get strip data: {}", e)))?;
    let compression = root_ifd.get_entry(TiffCommonTag::Compression).ok_or("Missing tag")?.force_usize(0);
    let width = fetch_tiff_tag!(root_ifd, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(root_ifd, TiffCommonTag::ImageLength).force_usize(0);
    if compression == 1 {
      Ok(Some(DynamicImage::ImageRgb8(
        ImageBuffer::<Rgb<u8>, Vec<u8>>::from_raw(width as u32, height as u32, buf).unwrap(),
      )))
    } else {
      let img = image::load_from_memory_with_format(&buf, image::ImageFormat::Jpeg).unwrap();
      Ok(Some(img))
    }
  }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
enum Cr2Mode {
  Raw,
  Sraw1,
  Sraw2,
}

impl<'a> Cr2Decoder<'a> {
  fn get_mode(makernote: &Option<IFD>) -> Result<Cr2Mode> {
    if let Some(settings) = makernote.as_ref().and_then(|mn| mn.get_entry(Cr2MakernoteTag::CameraSettings)) {
      match settings.get_u16(46) {
        Ok(Some(0)) => Ok(Cr2Mode::Raw),
        Ok(Some(1)) => Ok(Cr2Mode::Sraw1),
        Ok(Some(2)) => Ok(Cr2Mode::Sraw2),
        Ok(None) => Ok(Cr2Mode::Raw),
        Ok(Some(v)) => Err(RawlerError::General(format!("Unknown sraw quality value found: {}", v))),
        Err(_) => Err(RawlerError::General("Unknown sraw quality value".to_string())),
      }
    } else {
      Ok(Cr2Mode::Raw)
    }
  }

  /// Construct new CR2 decoder
  /// This parses the RawFile again to include specific sub IFDs.
  pub fn new(file: &mut RawFile, _tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<Cr2Decoder<'a>> {
    debug!("CR2 decoder choosen");

    // Parse the TIFF again, with custom settings
    file
      .seek_to_start()
      .map_err(|e| RawlerError::General(format!("I/O error while reading raw file: {:?}", e)))?;
    let tiff = GenericTiffReader::new(file.inner(), 0, 0, None, &[33424])?;

    let exif = Self::new_exif_ifd(file, &tiff, rawloader)?;
    let makernote = Self::new_makernote(file, &tiff, &exif, rawloader)?;
    let mode = Self::get_mode(&makernote)?;

    debug!("sRaw quality: {:?}", mode);
    let mode_str = match mode {
      Cr2Mode::Raw => "",
      Cr2Mode::Sraw1 => "sRaw1",
      Cr2Mode::Sraw2 => "sRaw2",
    };

    let camera = rawloader.check_supported_with_mode(tiff.root_ifd(), mode_str)?;

    let xpacket = Self::read_xpacket(file, &tiff, rawloader)?;

    let model_id = makernote
      .as_ref()
      .and_then(|mn| mn.get_entry(Cr2MakernoteTag::ModelId).and_then(|v| v.get_u32(0).transpose()))
      .transpose()
      .map_err(|_| RawlerError::General("invalid model id".to_string()))?;
    Ok(Cr2Decoder {
      tiff,
      rawloader,
      exif,
      makernote,
      mode,
      xpacket,
      camera,
      model_id,
    })
  }

  /// Search for EXIF IFD, if not found, fallback to root IFD.
  /// This is useful for EOS D2000 where EXIF tags are located in the root.
  fn new_exif_ifd(_file: &mut RawFile, tiff: &GenericTiffReader, _rawloader: &RawLoader) -> Result<IFD> {
    if let Some(exif_ifd) = tiff
      .root_ifd()
      .sub_ifds()
      .get(&TiffCommonTag::ExifIFDPointer.into())
      .and_then(|subs| subs.get(0))
    {
      Ok(exif_ifd.clone())
    } else {
      debug!("No EXIF IFD found, fallback to root IFD");
      Ok(tiff.root_ifd().clone())
    }
    /*
    if let Some(exif_ifd) = tiff.root_ifd().get_ifd(LegacyTiffRootTag::ExifIFDPointer, file.inner())? {
      return Ok(exif_ifd);
    } else {
      return Ok(tiff.root_ifd().clone());
    }
     */
  }

  fn get_focal_len(&self) -> Result<Option<Rational>> {
    if let Some(Entry {
      value: Value::Short(focal), ..
    }) = self.makernote.as_ref().and_then(|mn| mn.get_entry(Cr2MakernoteTag::FocalLen))
    {
      return Ok(focal.get(1).map(|v| Rational::new(*v as u32, 1)));
    }
    Ok(None)
  }

  /// Get lens description by analyzing TIFF tags and makernotes
  fn get_lens_description(&self) -> Result<Option<&'static LensDescription>> {
    let exif_lens_name = if let Some(Entry {
      value: Value::Ascii(lens_id), ..
    }) = self.exif.get_entry(ExifTag::LensModel)
    {
      lens_id.strings().get(0)
    } else {
      None
    };
    match self.makernote.as_ref().and_then(|mn| mn.get_entry(Cr2MakernoteTag::CameraSettings)) {
      Some(Entry {
        value: Value::Short(settings), ..
      }) => {
        let lens_info = settings[22];
        debug!("Lens Info tag: {}", lens_info);
        let resolver = LensResolver::new()
          .with_lens_keyname(exif_lens_name)
          .with_lens_id((lens_info as u32, 0))
          .with_focal_len(self.get_focal_len()?)
          .with_mounts(&[CANON_CN_MOUNT.into(), CANON_EF_MOUNT.into()]);
        return Ok(resolver.resolve());
      }
      _ => {
        log::warn!("Camera settings in makernote not found, no lens data available");
      }
    }
    Ok(None)
  }

  /// Parse the Canon makernote IFD
  fn new_makernote(file: &mut RawFile, tiff: &GenericTiffReader, exif_ifd: &IFD, _rawloader: &RawLoader) -> Result<Option<IFD>> {
    if let Some(entry) = exif_ifd.get_entry(TiffCommonTag::Makernote) {
      let offset = entry.offset().expect("Makernote internal offset is not present but should be");
      let makernote = tiff.parse_ifd(file.inner(), offset as u32, 0, 0, exif_ifd.endian, &[])?;
      return Ok(Some(makernote));
    }
    info!("No makernote tag found");
    Ok(None)
  }

  /// Read XMP data from TIFF entry
  /// This is useful as it stores the image rating (if present).
  fn read_xpacket(_file: &mut RawFile, tiff: &GenericTiffReader, _rawloader: &RawLoader) -> Result<Option<Vec<u8>>> {
    if let Some(entry) = tiff.root_ifd().get_entry(TiffCommonTag::Xmp) {
      if let Entry { value: Value::Byte(xmp), .. } = entry {
        Ok(Some(xmp.clone()))
      } else {
        Err("Image has XMP data but invalid tag type!".into())
      }
    } else {
      Ok(None)
    }
  }

  /*
  pub fn new_makernote(buf: &'a[u8], offset: usize, base_offset: usize, chain_level: isize, e: Endian) -> Result<LegacyTiffIFD<'a>> {
    let mut off = 0;
    let data = &buf[offset..];
    let mut endian = e;

    // Some have MM or II to indicate endianness - read that
    if data[off..off+2] == b"II"[..] {
      off +=2;
      endian = Endian::Little;
    } if data[off..off+2] == b"MM"[..] {
      off +=2;
      endian = Endian::Big;
    }

    Ok(LegacyTiffIFD::new(buf, offset+off, base_offset, 0, chain_level+1, endian, &vec![])?)
  }
  */

  /// Build firmware value from string
  fn get_firmware(&self) -> Result<Option<u32>> {
    Ok(
      match self
        .makernote
        .as_ref()
        .and_then(|mn| mn.get_entry(Cr2MakernoteTag::FirmareVer).and_then(|v| v.as_string()))
      {
        Some(fw) => {
          let str: String = fw.chars().filter(|c| c.is_ascii_digit() || c == &'.').collect();
          let v: Vec<u8> = str.split('.').map(|v| v.parse().expect("Only digits here")).collect();
          Some(v.iter().rev().enumerate().map(|(i, v)| 10_u32.pow(i as u32 * 3) * *v as u32).sum())
        }
        None => None,
      },
    )
  }

  /// Get the SRAW white balance coefficents from COLORDATA tag
  /// The offsets are always at offset 78.
  /// These coefficents are used for SRAW YUV2RGB conversion.
  fn get_sraw_wb(&self, rawfile: &mut RawFile, _cam: &Camera) -> Result<[f32; 4]> {
    if let Some(levels) = self
      .makernote
      .as_ref()
      .and_then(|mn| mn.get_entry_raw(Cr2MakernoteTag::ColorData, rawfile.inner()).transpose())
      .transpose()?
    {
      let offset = 78;
      return Ok([
        levels.get_force_u16(offset) as f32,
        (levels.get_force_u16(offset + 1) as f32 + levels.get_force_u16(offset + 2) as f32) / 2.0,
        levels.get_force_u16(offset + 3) as f32,
        NAN,
      ]);
    }
    Ok([NAN, NAN, NAN, NAN])
  }

  /// Get the white balance coefficents from COLORDATA tag
  /// The offsets are different, so we take the offset from camera params.
  fn get_wb(&self, rawfile: &mut RawFile, cam: &Camera) -> Result<[f32; 4]> {
    if let Some(levels) = self
      .makernote
      .as_ref()
      .and_then(|mn| mn.get_entry_raw(Cr2MakernoteTag::ColorData, rawfile.inner()).transpose())
      .transpose()?
    {
      if let Some(offset) = cam.param_usize("colordata_wbcoeffs") {
        let raw_wb = [
          levels.get_force_u16(offset) as f32,
          levels.get_force_u16(offset + 1) as f32,
          levels.get_force_u16(offset + 2) as f32,
          levels.get_force_u16(offset + 3) as f32,
        ];
        return Ok(normalize_wb(raw_wb));
      }
    }

    // TODO: check if these tags belongs to RootIFD or makernote
    if let Some(levels) = self.tiff.get_entry_raw(TiffCommonTag::Cr2PowerShotWB, rawfile.inner())? {
      Ok([
        levels.get_force_u32(3) as f32,
        levels.get_force_u32(2) as f32,
        levels.get_force_u32(4) as f32,
        NAN,
      ])
    } else if let Some(levels) = self.tiff.get_entry(TiffCommonTag::Cr2OldWB) {
      Ok([levels.force_f32(0), levels.force_f32(1), levels.force_f32(2), NAN])
    } else {
      // At least the D2000 has no WB
      Ok([NAN, NAN, NAN, NAN])
    }
  }

  /// Get the black level from COLORDATA tag
  /// The offsets are different, so we take the offset from camera params.
  fn get_blacklevel(&self, rawfile: &mut RawFile, cam: &Camera) -> Result<[u16; 4]> {
    if let Some(levels) = self
      .makernote
      .as_ref()
      .and_then(|mn| mn.get_entry_raw(Cr2MakernoteTag::ColorData, rawfile.inner()).transpose())
      .transpose()?
    {
      if let Some(offset) = cam.param_usize("colordata_blacklevel") {
        return Ok([
          levels.get_force_u16(offset),
          levels.get_force_u16(offset),
          levels.get_force_u16(offset),
          levels.get_force_u16(offset),
        ]);
      }
    }
    Ok(cam.blacklevels)
  }

  /// Get the white level from COLORDATA tag
  /// The offsets are different, so we take the offset from camera params.
  fn get_whitelevel(&self, rawfile: &mut RawFile, cam: &Camera) -> Result<[u16; 4]> {
    if let Some(levels) = self
      .makernote
      .as_ref()
      .and_then(|mn| mn.get_entry_raw(Cr2MakernoteTag::ColorData, rawfile.inner()).transpose())
      .transpose()?
    {
      if let Some(offset) = cam.param_usize("colordata_whitelevel") {
        return Ok([
          levels.get_force_u16(offset),
          levels.get_force_u16(offset),
          levels.get_force_u16(offset),
          levels.get_force_u16(offset),
        ]);
      }
    }
    Ok(cam.whitelevels)
  }

  /// Get the SENSOR information, if available
  /// If not, fall back to sensor dimension reported by width/hight values.
  fn get_sensor_area(&self, _cam: &Camera, width: usize, height: usize) -> Result<Rect> {
    if let Some(sensorinfo) = self.makernote.as_ref().and_then(|mn| mn.get_entry(Cr2MakernoteTag::SensorInfo)) {
      match &sensorinfo.value {
        Value::Short(v) => {
          debug!("Sensor info: {:?}", v);
          let _w = v[1] as usize;
          let _h = v[2] as usize;
          let left = v[5] as usize;
          let top = v[6] as usize;
          let right = v[7] as usize;
          let bottom = v[8] as usize;
          Ok(Rect::new_with_points(Point::new(left, top), Point::new(right + 1, bottom + 1)))
        }
        _ => Err(RawlerError::General("Makernote contains invalid type for SensorInfo tag".to_string())),
      }
    } else {
      Ok(Rect::new(Point::zero(), Dim2::new(width, height)))
    }
  }

  /// Interpolate YCbCr (YUV) data
  fn interpolate_yuv(&self, ljpeg: &LjpegDecompressor, width: usize, _height: usize, image: &mut [u16]) {
    if ljpeg.super_h() == 1 && ljpeg.super_v() == 1 {
      return; // No interpolation needed
    }
    // Iterate over a block of 3 rows, smaller chunks are okay
    // but mus always a multiple of row width.
    image.par_chunks_mut(width * 3).for_each(|slice| {
      // Do horizontal interpolation.
      // [y1 Cb Cr ] [ y2 . . ] [y1 Cb Cr ] [ y2 . . ] ...
      if ljpeg.super_h() == 2 {
        debug_assert_eq!(slice.len() % width, 0);
        for row in 0..(slice.len() / width) {
          for col in (6..width).step_by(6) {
            let pix1 = row * width + col - 6;
            let pix2 = pix1 + 3;
            let pix3 = row * width + col;
            slice[pix2 + 1] = ((slice[pix1 + 1] as i32 + slice[pix3 + 1] as i32 + 1) / 2) as u16;
            slice[pix2 + 2] = ((slice[pix1 + 2] as i32 + slice[pix3 + 2] as i32 + 1) / 2) as u16;
          }
        }
      }
      // Do vertical interpolation
      //          pixel n      pixel n+1       pixel n+2    pixel n+3       ...
      // row i  : [y1 Cb  Cr ] [ y2 Cb*  Cr* ] [y1 Cb  Cr ] [ y2 Cb*  Cr* ] ...
      // row i+1: [y3 Cb* Cr*] [ y4 Cb** Cr**] [y3 Cb* Cr*] [ y4 Cb** Cr**] ...
      // row i+2: [y1 Cb  Cr ] [ y2 Cb*  Cr* ] [y1 Cb  Cr ] [ y2 Cb*  Cr* ] ...
      // row i+3: [y3 Cb* Cr*] [ y4 Cb** Cr**] [y3 Cb* Cr*] [ y4 Cb** Cr**] ...
      if ljpeg.super_v() == 2 && slice.len() == width * 3 {
        for col in (0..width).step_by(3) {
          let pix1 = col;
          let pix2 = width + col;
          let pix3 = 2 * width + col;
          slice[pix2 + 1] = ((slice[pix1 + 1] as i32 + slice[pix3 + 1] as i32 + 1) / 2) as u16;
          slice[pix2 + 2] = ((slice[pix1 + 2] as i32 + slice[pix3 + 2] as i32 + 1) / 2) as u16;
        }
      }
    });

    /* Old non-parallel code
    if ljpeg.super_h() == 2 {
      for row in 0..height {
        for col in (6..width).step_by(6) {
          let pix1 = row * width + col - 6;
          let pix2 = pix1 + 3;
          let pix3 = row * width + col;
          image[pix2 + 1] = ((image[pix1 + 1] as i32 + image[pix3 + 1] as i32 + 1) / 2) as u16;
          image[pix2 + 2] = ((image[pix1 + 2] as i32 + image[pix3 + 2] as i32 + 1) / 2) as u16;
        }
      }
    }


    if ljpeg.super_v() == 2 {
      for row in (1..height - 1).step_by(2) {
        for col in (0..width).step_by(3) {
          let pix1 = (row - 1) * width + col;
          let pix2 = row * width + col;
          let pix3 = (row + 1) * width + col;
          image[pix2 + 1] = ((image[pix1 + 1] as i32 + image[pix3 + 1] as i32 + 1) / 2) as u16;
          image[pix2 + 2] = ((image[pix1 + 2] as i32 + image[pix3 + 2] as i32 + 1) / 2) as u16;
        }
      }
    }
    */
  }

  /// Convert YCbCr (YUV) data to linear RGB
  fn convert_to_rgb(
    &self,
    rawfile: &mut RawFile,
    cam: &Camera,
    ljpeg: &LjpegDecompressor,
    width: usize,
    height: usize,
    image: &mut [u16],
    dummy: bool,
  ) -> Result<()> {
    debug!("YUV2RGB: Regular WB: {:?}", self.get_wb(rawfile, cam));
    debug!("YUV2RGB: SRAW WB: {:?}", self.get_sraw_wb(rawfile, cam));
    debug!("Model ID: 0x{:X}", self.model_id.unwrap_or(0));
    if dummy {
      return Ok(());
    }

    let do_interpolate = std::env::var("RAWLER_CR2_YUV_INTERPOLATE")
      .ok()
      .map(|id| id.parse::<bool>().expect("RAWLER_CR2_YUV_INTERPOLATE must by of type bool"))
      .unwrap_or(true);
    if do_interpolate {
      self.interpolate_yuv(ljpeg, width, height, image);
    }

    let coeffs = self.get_sraw_wb(rawfile, cam)?;
    let (c1, c2, c3) = if cam.find_hint("invert_sraw_wb") {
      let c1 = (1024.0 * 1024.0 / coeffs[0]) as i32;
      let c2 = coeffs[1] as i32;
      let c3 = (1024.0 * 1024.0 / coeffs[2]) as i32;
      (c1, c2, c3)
    } else {
      (coeffs[0] as i32, coeffs[1] as i32, coeffs[2] as i32)
    };

    // Starting with 40D, sRaw format was introduced. This uses
    // version 0. With 5D Mark II, version 1 gets used.
    // And with 5D Mark III, back to version 0 method
    // but without an offset of 512 for y.
    let version = if cam.find_hint("sraw_40d") {
      0
    } else if cam.find_hint("sraw_new") {
      2
    } else {
      1
    };

    let fw = self.get_firmware()?;
    debug!("Firmware: {:?}", fw);

    // This magic comes from dcraw.
    // Seems to because of rounding during interpolation, we need to
    // adjust the hue a little bit (only guessing)
    let hue = match self.model_id {
      None => 0,
      Some(model_id) => {
        if model_id >= 0x80000281 || (model_id == 0x80000218 && fw.unwrap_or(0) > 1000006) {
          (((ljpeg.super_h() * ljpeg.super_v()) - 1) >> 1) as i32
        } else {
          (ljpeg.super_h() * ljpeg.super_v()) as i32
        }
      }
    };
    debug!("SRAW hue correction: {:?}", hue);

    // Now calculate RGB for each YUV tuple.
    image.par_chunks_exact_mut(3).for_each(|pix| {
      let y = pix[0] as i32;
      let cb = pix[1] as i32 - 16383;
      let cr = pix[2] as i32 - 16383;
      match version {
        0 => {
          let y = y - 512; // correction for 40D and others
          let r = c1 * (y + cr);
          let g = c2 * (y + ((-778 * cb - (cr << 11)) >> 12));
          let b = c3 * (y + cb);
          pix[0] = clampbits(r >> 8, 16);
          pix[1] = clampbits(g >> 8, 16);
          pix[2] = clampbits(b >> 8, 16);
        }
        1 => {
          // found in EOS 5D Mark II
          let cb = (cb << 2) + hue;
          let cr = (cr << 2) + hue;
          let r = c1 * (y + ((50 * cb + 22929 * cr) >> 14));
          let g = c2 * (y + ((-5640 * cb - 11751 * cr) >> 14));
          let b = c3 * (y + ((29040 * cb - 101 * cr) >> 14));
          pix[0] = clampbits(r >> 8, 16);
          pix[1] = clampbits(g >> 8, 16);
          pix[2] = clampbits(b >> 8, 16);
        }
        2 => {
          // found in EOS 5D Mark III and others
          let r = c1 * (y + cr);
          let g = c2 * (y + ((-778 * cb - (cr << 11)) >> 12));
          let b = c3 * (y + cb);
          pix[0] = clampbits(r >> 8, 16);
          pix[1] = clampbits(g >> 8, 16);
          pix[2] = clampbits(b >> 8, 16);
        }
        _ => {
          unreachable!()
        }
      }
    });
    Ok(())
  }
}

fn normalize_wb(raw_wb: [f32; 4]) -> [f32; 4] {
  debug!("CR2 raw wb: {:?}", raw_wb);
  // We never have more then RGB colors so far (no RGBE etc.)
  // So we combine G1 and G2 to get RGB wb.
  let div = raw_wb[1]; // G1 should be 1024 and we use this as divisor
  let mut norm = raw_wb;
  norm.iter_mut().for_each(|v| {
    if v.is_normal() {
      *v /= div
    }
  });
  [norm[0], (norm[1] + norm[2]) / 2.0, norm[3], NAN]
}

crate::tags::tiff_tag_enum!(Cr2MakernoteTag);

/// Specific Canon CR2 Makernotes tags.
/// These are only related to the Makernote IFD.
#[derive(Debug, Copy, Clone, PartialEq, enumn::N)]
#[repr(u16)]
pub enum Cr2MakernoteTag {
  CameraSettings = 0x0001,
  FocalLen = 0x0002,
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

//const CR2_MODEL_40D: u32 = 0x80000190;
