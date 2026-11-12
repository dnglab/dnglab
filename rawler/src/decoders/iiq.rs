// SPDX-License-Identifier: LGPL-2.1
// Copyright 2022 Daniel Vogelbacher <daniel@chaospixel.com>
//
// This code & comments are partially a rewrite of librawspeed and dcraw.
//   Copyright 1997-2016 by Dave Coffin, dcoffin a cybercom o net
//   Copyright (C) 2009-2014 Klaus Post
//   Copyright (C) 2014-2015 Pedro Côrte-Real
//   Copyright (C) 2017-2019 Roman Lebedev
//   Copyright (C) 2019 Robert Bridge

use byteorder::LittleEndian;
use byteorder::ReadBytesExt;
use log::debug;
use log::warn;
use rayon::prelude::*;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::io::Seek;
use std::io::SeekFrom;
use std::mem::size_of;

use crate::Orientation;
use crate::RawImage;
use crate::RawLoader;
use crate::RawlerError;
use crate::Result;
use crate::analyze::FormatDump;
use crate::bits::*;
use crate::camera_profile::CameraProfile;
use crate::cfa;
use crate::decoders::ok_cfa_image_with_blacklevels;
use crate::decompressors::decompress_lines_fn;
use crate::exif::Exif;
use crate::formats::tiff;
use crate::formats::tiff::GenericTiffReader;
use crate::formats::tiff::reader::TiffReader;
use crate::imgop::Dim2;
use crate::imgop::Point;
use crate::imgop::Rect;
use crate::imgop::matrix::{multiply, pseudo_inverse};
use crate::imgop::spline::Spline;
use crate::imgop::xyz::XYZ_TO_PROFOTORGB_D50;
use crate::lens::LensDescription;
use crate::lens::LensResolver;
use crate::pixarray::PixU16;
use crate::pumps::BitPump;
use crate::pumps::BitPumpMSB32;
use crate::rawsource::RawSource;

use super::Camera;
use super::CameraLookup;
use super::Decoder;
use super::FormatHint;
use super::RawDecodeParams;
use super::RawMetadata;
use super::image_bytes;

const MAX_BITDEPTH: u32 = 16;

const SV2_USED_CORR: [u32; 8] = [
  3, 3, 3, 3, //
  1, 1, 1, 1,
];

const SV2_EXTRA_BITS: [u32; 8] = [
  1, 2, 3, 4, //
  0, 0, 0, 0,
];

const SV2_BIT_INDICATOR: [u32; 32] = [
  9, 8, 0, 7, 6, 6, 5, 5, //
  1, 1, 1, 1, 4, 4, 4, 4, //
  3, 3, 3, 3, 3, 3, 3, 3, //
  2, 2, 2, 2, 2, 2, 2, 2,
];

const SV2_SKIP_BITS: [u8; 32] = [
  5, 5, 5, 5, 4, 4, 4, 4, //
  3, 3, 3, 3, 3, 3, 3, 3, //
  2, 2, 2, 2, 2, 2, 2, 2, //
  2, 2, 2, 2, 2, 2, 2, 2,
];

/// Sensor defect information
#[allow(unused)]
#[derive(Clone, Debug)]
struct Defect {
  row: usize,
  col: usize,
  typ: u16,
  reserved: u16,
}

/// Flat field correction data.
#[derive(Debug, Clone)]
struct FlatField {
  typ: u32,
  head: [u16; 8],
  data: Vec<f32>,
}

/// Stuct to hold all known calibration data.
struct SensorCalibration {
  poly_curve_half: Option<[f32; 8]>,
  poly_curve_full: Option<[f32; 4]>,
  quadrant_linearization: Option<Vec<u16>>,
  quadrant_multipliers: Option<[f32; 19]>,
  quadrant_combined: Option<([usize; 7], [usize; 7 * 4])>,
  defects: Option<Vec<Defect>>,
  flats: Vec<FlatField>,
  blacklevel: u16,
  q_blacklevel: Option<(Vec<i16>, Vec<i16>)>,
}

/// Known IIQ compression formats
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[allow(non_camel_case_types)]
enum IiqCompression {
  Uncompressed = 0,
  Raw1 = 1,
  Raw2 = 2,
  // L14
  IIQ_L = 3,
  // Like L14 but with nonlinear data which requires curve multiplication
  IIQ_S = 5,
  // "IIQ 14 Smart" and "IIQ 14 Sensor+"
  IIQ_Sv2 = 6,
  // "IIQ 16 Extended" and "IIQ 16 Large"
  IIQ_L16 = 8,
}

impl TryFrom<usize> for IiqCompression {
  type Error = RawlerError;

  fn try_from(v: usize) -> std::result::Result<Self, Self::Error> {
    match v {
      0 => Ok(Self::Uncompressed),
      1 => Ok(Self::Raw1),
      2 => Ok(Self::Raw2),
      3 => Ok(Self::IIQ_L),
      5 => Ok(Self::IIQ_S),
      6 => Ok(Self::IIQ_Sv2),
      8 => Ok(Self::IIQ_L16),
      _ => Err(decoder_error!("Unsupported IIQ format: {}", v)),
    }
  }
}

/// IIQ format encapsulation for analyzer
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IiqFormat {
  //tiff: GenericTiffReader,
  makernotes: IiqMakernotes,
}

pub type IiqMakernotes = HashMap<u32, (usize, tiff::Value)>;

#[derive(Debug, Clone)]
pub struct IiqDecoder<'a> {
  camera: CameraLookup,
  #[allow(unused)]
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
  makernotes: IiqMakernotes,
}

impl<'a> IiqDecoder<'a> {
  pub fn new(file: &RawSource, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<IiqDecoder<'a>> {
    debug!("IIQ decoder choosen");
    let camera = rawloader.lookup_camera(tiff.root_ifd())?;
    let makernotes = new_makernote(file, 8).map_err(|ioerr| RawlerError::with_io_error("load IIQ makernotes", file.path(), ioerr))?;
    Ok(IiqDecoder {
      camera,
      tiff,
      rawloader,
      makernotes,
    })
  }
}

fn new_makernote(file: &RawSource, moffset: u64) -> std::io::Result<HashMap<u32, (usize, tiff::Value)>> {
  // All makernote offsets are not absolute to file, but to start of makernote data.
  // The makernote data starts straight after the TIFF header (8 bytes).
  // If this may change in future, offsets must be revalidated.
  assert_eq!(moffset, 8);
  // Skip first 8 bytes of makernote
  let ifd = LEu32(file.subview(moffset + 8, 4)?, 0) as u64;
  // The entry count is followed by 4 bytes that are skipped
  let entry_count = LEu32(file.subview(moffset + ifd, 4)?, 0) as usize;
  let size = entry_count
    .checked_mul(16)
    .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("IIQ makernote with {} entries", entry_count)))?;
  // A view of the entries fails for entries beyond the end of the file
  let buf = file.subview(moffset + ifd + 8, size as u64)?;

  let mut entries: HashMap<u32, (usize, tiff::Value)> = HashMap::with_capacity(entry_count);

  for entry in buf.as_chunks::<16>().0 {
    let tag = LEu32(entry, 0);
    let typ = LEu32(entry, 4);
    let byte_count = LEu32(entry, 8) as usize;
    let data = LEu32(entry, 12) as usize;
    //println!("Tag 0x{:x}, typ: {}, count: {}, data: {}", tag, typ, byte_count, data);
    match typ {
      // Sensor calibration file, it's like a TIFF file and stored with ASCII type
      1 if tag == <IiqTag as Into<u32>>::into(IiqTag::SensorCorrection) => match (data as u32).checked_add(moffset as u32) {
        Some(offset) => {
          entries.insert(tag, (byte_count, tiff::Value::Long(vec![offset])));
        }
        None => warn!("IIQ: offset {} of the sensor correction is out of range, ignoring it", data),
      },
      // Others should be just ASCII strings
      1 => {
        let v = file.subview(moffset + data as u64, byte_count as u64)?;
        let value = tiff::Value::Ascii(tiff::TiffAscii::new_from_raw(v));
        entries.insert(tag, (byte_count, value));
      }
      // Short values stored as Long
      2 => {
        entries.insert(tag, (byte_count, tiff::Value::Long(vec![data as u32])));
      }
      // Integer values
      4 => {
        // long
        entries.insert(tag, (byte_count, tiff::Value::Long(vec![data as u32])));
      }
      _ => {
        warn!("Unknow tag 0x{:x}, typ: {}, count: {}, data: {}", tag, typ, byte_count, data);
      }
    }
  }
  Ok(entries)
}

impl<'a> Decoder for IiqDecoder<'a> {
  fn raw_image(&self, file: &RawSource, _params: &RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let camera = self.camera.supported()?;
    let fmt = self.compression_mode()?;

    let wb_offset = self.wb_offset()?;
    let (width, height) = self.dimension()?;
    let (data_offset, data_len) = self.data_offset()?;

    let (strips_offset, strips_len) = if fmt != IiqCompression::Uncompressed { self.strip_offset()? } else { (0, 0) };

    debug!("Strips offset: {}", strips_offset);
    debug!("data offset: {}", data_offset);
    debug!("RAW IIQ Format: {:?}", fmt);

    // The decoders store 16 bits per pixel
    image_bytes("IIQ", width, height, 16)?;

    let data = file
      .subview(data_offset, data_len as u64)
      .map_err(|ioerr| RawlerError::with_io_error("load IIQ data", file.path(), ioerr))?;
    let strips = file
      .subview(strips_offset, strips_len as u64)
      .map_err(|ioerr| RawlerError::with_io_error("load IIQ strips", file.path(), ioerr))?;

    let mut image = match fmt {
      IiqCompression::Raw1 | IiqCompression::Raw2 => return Err(RawlerError::unsupported(camera, format!("IIQ compression {:?}", fmt))),
      IiqCompression::Uncompressed => Self::decode_uncompressed(data, width, height, 14, dummy).map_err(RawlerError::decoder_failed)?,
      IiqCompression::IIQ_L => Self::decode_compressed(data, strips, width, height, 14, dummy).map_err(RawlerError::decoder_failed)?,
      IiqCompression::IIQ_L16 => Self::decode_compressed(data, strips, width, height, 16, dummy).map_err(RawlerError::decoder_failed)?,
      IiqCompression::IIQ_S => Self::decode_nonlinear(data, strips, width, height, 14, dummy).map_err(RawlerError::decoder_failed)?,
      IiqCompression::IIQ_Sv2 => Self::decode_compressed_sv2(data, strips, width, height, 14, dummy).map_err(RawlerError::decoder_failed)?,
    };

    let black = self.blacklevel().unwrap_or(0);
    log::debug!("Blacklevel: {}", black);

    if !dummy {
      let senscorr = self
        .new_sensor_correction(file, black)
        .map_err(|ioerr| RawlerError::with_io_error("read IIQ sensor calibration data", file.path(), ioerr))?;
      self.correct_raw(&mut image, &senscorr)?;
    }

    // Reset blacklevel, as the pixels are already scaled by decoder and sensor correction!
    let blacklevel = [0, 0, 0, 0];
    let cpp = 1;
    let wb = self
      .get_wb(file, wb_offset)
      .map_err(|ioerr| RawlerError::with_io_error("read IIQ white balance", file.path(), ioerr))?;

    let mut image = ok_cfa_image_with_blacklevels(camera.clone(), cpp, wb, blacklevel, image, dummy)?;
    // Cameras without active area in the camera definition use the area of the file
    if image.active_area.is_none() {
      image.active_area = self.image_area(Dim2::new(width, height));
    }
    // Cameras without color matrix in the camera definition use the matrix of the file
    if image.camera_profiles.is_empty() {
      let romm_matrix = self
        .romm_matrix(file)
        .map_err(|ioerr| RawlerError::with_io_error("read IIQ color matrix", file.path(), ioerr))?;
      if let Some(romm_matrix) = romm_matrix {
        debug!("IIQ camera to ROMM matrix: {:?}", romm_matrix);
        let forward_matrix = multiply(&pseudo_inverse(XYZ_TO_PROFOTORGB_D50), &romm_matrix);
        let neutral = [wb[1] / wb[0], 1.0, wb[1] / wb[2]];
        match CameraProfile::from_forward_matrix_d50(&forward_matrix, &neutral) {
          Some(profile) => image.camera_profiles.push(profile),
          None => warn!("IIQ: invalid white balance {:?}, color matrix not available", wb),
        }
      }
    }
    image.orientation = self.orientation();
    Ok(image)
  }

  fn check_supported(&self) -> Result<()> {
    self.camera.supported().map(|_| ())
  }

  fn format_dump(&self) -> FormatDump {
    FormatDump::Iiq(IiqFormat {
      //tiff: self.tiff.clone(),
      makernotes: self.makernotes.clone(),
    })
  }

  fn raw_metadata(&self, _file: &RawSource, _params: &RawDecodeParams) -> Result<RawMetadata> {
    let mut exif = Exif::new(self.tiff.root_ifd())?;
    exif.orientation = self.orientation().map(|orientation| orientation.to_u16());
    let mdata = RawMetadata::new_with_lens(self.camera.metadata(), exif, self.get_lens_description()?.cloned());
    Ok(mdata)
  }

  fn format_hint(&self) -> FormatHint {
    FormatHint::IIQ
  }
}

impl<'a> IiqDecoder<'a> {
  /// Apply correction information for sensor defects
  /// Only bad columns are corrected for now.
  fn correct_raw_defects(&self, img: &mut PixU16, senscorr: &SensorCalibration) -> Result<()> {
    let camera = self.camera.supported()?;
    if let Some(defects) = &senscorr.defects {
      debug!("Apply defect correction");

      for def in defects.iter() {
        if def.col >= img.width {
          continue;
        };
        match def.typ {
          131 | 137 => {
            debug!("Correct bad colum: {}", def.col);
            Self::fix_bad_column(camera, img, def);
          }
          129 => {
            // single bad pixels are ignored, these can be fixed by
            // hot pixel modules in development process.
          }
          _ => {
            // many unknows...
          }
        }
      }
    }
    Ok(())
  }

  /// Apply polynom curve correction for the right side of the sensor.
  fn correct_raw_poly_half(&self, img: &mut PixU16, senscorr: &SensorCalibration) -> Result<()> {
    if let Some(poly) = &senscorr.poly_curve_half {
      debug!("Apply polynom curve half correction");
      let (Some(split_col), Some(sensor_temp)) = (self.split_column()?, self.sensor_temp()?) else {
        warn!("IIQ: the half polynomial curve needs the split column and the sensor temperature, skipping it");
        return Ok(());
      };

      let mut poly = *poly;
      poly[3] += (sensor_temp - poly[7]) * poly[6] + 1.0;

      let mut curve = vec![0_u16; 0x10000];
      for (i, x) in curve.iter_mut().enumerate() {
        let num = (poly[5] * i as f32 + poly[3]) * i as f32 + poly[1];
        *x = clamp(num as i32, 0, 0xffff) as u16;
      }
      // Only apply for right side of image
      img.par_pixel_rows_mut().for_each(|row| {
        for pix in row.iter_mut().skip(split_col) {
          *pix = curve[*pix as usize];
        }
      });
    }
    Ok(())
  }

  /// Apply polynom curve correction to the full sensor.
  fn correct_raw_poly_full(&self, img: &mut PixU16, senscorr: &SensorCalibration) -> Result<()> {
    if let Some(poly) = &senscorr.poly_curve_full {
      debug!("Apply polynom curve full correction");
      let mut curve = vec![0_u16; 0x10000];
      for (i, x) in curve.iter_mut().enumerate() {
        let mut num = 0.0;
        for p in poly.iter().rev() {
          num = num * i as f32 + *p;
        }
        *x = clamp((num + i as f32) as i32, 0, 0xffff) as u16;
      }
      img.for_each(|pix| curve[pix as usize]);
    }
    Ok(())
  }

  /// Apply quadrant linerarization correction
  fn correct_raw_q_linearization(&self, img: &mut PixU16, senscorr: &SensorCalibration) -> Result<()> {
    if let Some(table) = &senscorr.quadrant_linearization {
      debug!("Apply quadrant linearization");
      let Some((split_col, split_row)) = self.quadrant_split("quadrant linearization") else {
        return Ok(());
      };

      debug!("Split col: {}, row: {}", split_col, split_row);

      let mut control_points = [[[Point::default(); 19]; 2]; 2];

      for qr in 0..2 {
        for qc in 0..2 {
          for i in 0..16 {
            control_points[qr][qc][1 + i].x = table[qr * (2 * 16) + (qc * 16) + i] as usize;
          }
        }
      }
      for i in 0..16 {
        let mut v = 0;
        for qr in 0..2 {
          for qc in 0..2 {
            v += control_points[qr][qc][1 + i].x;
          }
        }
        control_points.iter_mut().flatten().for_each(|point| point[1 + i].y = (v + 2) >> 2);
      }

      let mut curves: [[Vec<u16>; 2]; 2] = Default::default();
      for (qr, qc) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
        let cp = &mut control_points[qr][qc];
        // Some images may overflow 65535 here, but it's also in dcraw...
        // We clamp here to be <= then the final control point.
        let Some(end) = (cp[16].y * 65535).checked_div(cp[16].x) else {
          warn!("IIQ: the quadrant linearization has a last control point at 0, skipping it");
          return Ok(());
        };
        cp[17].x = clamp(end as i32, 0, 65535) as usize;
        cp[17].y = cp[17].x;
        cp[18] = Point::new(65535, 65535);
        match Spline::new(cp) {
          Some(spline) => curves[qr][qc] = spline.calculate_curve(),
          None => {
            warn!("IIQ: invalid control points of the quadrant linearization, skipping it");
            return Ok(());
          }
        }
      }
      apply_quadrant_curves(img, split_row, split_col, &curves);
    }
    Ok(())
  }

  /// Apply quadrant multipliers correction
  fn correct_raw_q_mul(&self, img: &mut PixU16, senscorr: &SensorCalibration) -> Result<()> {
    if let Some(multipliers) = &senscorr.quadrant_multipliers {
      debug!("Apply quadrant multiplier correction");
      let Some((split_col, split_row)) = self.quadrant_split("quadrant multiplier correction") else {
        return Ok(());
      };
      debug!("Split col: {}, row: {}", split_col, split_row);

      let mut qmul = [[f32::NAN; 2]; 2];
      qmul[0][0] = 1.0 + multipliers[4];
      qmul[0][1] = 1.0 + multipliers[10];
      qmul[1][0] = 1.0 + multipliers[14];
      qmul[1][1] = 1.0 + multipliers[18];

      img.par_pixel_rows_mut().enumerate().for_each(|(row, line)| {
        let qr = if row >= split_row { 1 } else { 0 };
        for (col, pix) in line.iter_mut().enumerate() {
          let qc = if col >= split_col { 1 } else { 0 };
          let x = qmul[qr][qc] * (*pix as f32);
          *pix = clamp(x as i32, 0, 0xffff) as u16;
        }
      });
    }
    Ok(())
  }

  /// Apply quadrant combined spline curve correction
  fn correct_raw_q_combined(&self, img: &mut PixU16, senscorr: &SensorCalibration) -> Result<()> {
    if let Some((coord_x, coord_y)) = &senscorr.quadrant_combined {
      debug!("Apply quadrant combined correction");
      let Some((split_col, split_row)) = self.quadrant_split("quadrant combined correction") else {
        return Ok(());
      };

      debug!("Split col: {}, row: {}", split_col, split_row);

      let mut control_points = [[[Point::default(); 1 + 7 + 1]; 2]; 2]; // 7 points + start & end

      for qr in 0..2 {
        for qc in 0..2 {
          // we already have a 0 point at [0] by default initialization
          for i in 0..7 {
            control_points[qr][qc][1 + i].x = coord_x[i];
            control_points[qr][qc][1 + i].y = (coord_x[i] * coord_y[qr * (2 * 7) + (qc * 7) + i] as usize) / 10_000;
          }
          control_points[qr][qc][8] = Point::new(65535, 65535);
        }
      }

      let mut curves: [[Vec<u16>; 2]; 2] = Default::default();
      for (qr, qc) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
        match Spline::new(&control_points[qr][qc]) {
          Some(spline) => curves[qr][qc] = spline.calculate_curve(),
          None => {
            warn!("IIQ: invalid control points of the quadrant combined correction, skipping it");
            return Ok(());
          }
        }
      }
      apply_quadrant_curves(img, split_row, split_col, &curves);
    }
    Ok(())
  }

  /// Apply flat field correction, `cfa` and the positions of the flat fields are relative to the raw origin
  fn correct_raw_flatfield(img: &mut PixU16, cfa: &cfa::CFA, flats: &[FlatField]) {
    // Map the second green (on odd rows) to color 3
    let cfa = cfa.map_colors(|row, _col, color| if (color as usize) == cfa::CFA_COLOR_G && (row & 1 == 1) { 3 } else { color });

    for flat in flats.iter() {
      let nc = match flat.typ {
        0x401 => {
          debug!("Apply all-color flat field correction 0x{:x} HEAD: {:?}", flat.typ, flat.head);
          2
        }
        0x410 | 0x416 => {
          debug!("Apply all-color flat field correction 0x{:x} HEAD: {:?}", flat.typ, flat.head);
          2
        }
        0x40b => {
          debug!("Apply red-blue flat field correction 0x{:x} HEAD: {:?}", flat.typ, flat.head);
          4
        }
        _ => {
          panic!("Unsupported flat field typ");
        }
      };

      let mut head: [usize; 8] = Default::default();
      for (i, entry) in flat.head.iter().enumerate() {
        head[i] = *entry as usize;
      }
      if head[2] * head[3] * head[4] * head[5] == 0 {
        continue;
      };
      let wide: usize = head[2] / head[4] + ((head[2] % head[4] != 0) as usize);
      let high: usize = head[3] / head[5] + ((head[3] % head[5] != 0) as usize);
      // Each cell has a value for each pair of colors
      let need = wide * high * (nc / 2);
      if flat.data.len() < need {
        warn!(
          "IIQ: flat field 0x{:x} has {} values, but {} are needed, skipping it",
          flat.typ,
          flat.data.len(),
          need
        );
        continue;
      }
      let mut mrow = vec![0.0; nc * wide];
      let mut pump = flat.data.iter();
      let mut mult = [0.0; 4]; // One multiplier per color (R=0, G1=1, ...)

      for y in 0..high {
        for x in 0..wide {
          for c in (0..nc).step_by(2) {
            // The number of values is checked above
            let num = pump.next().copied().unwrap_or_default();
            if y == 0 {
              mrow[c * wide + x] = num;
            } else {
              mrow[(c + 1) * wide + x] = (num - mrow[c * wide + x]) / head[5] as f32;
            }
          }
        }
        if y == 0 {
          continue;
        };
        let rend = head[1] + y * head[5];
        let mut row = rend - head[5];
        while row < img.height && row < rend && row < head[1] + head[3] - head[5] {
          let line = img.row_mut(row);
          for x in 1..wide {
            for c in (0..nc).step_by(2) {
              mult[c] = mrow[c * wide + x - 1];
              mult[c + 1] = (mrow[c * wide + x] - mult[c]) / head[4] as f32;
            }
            let cend = head[0] + x * head[4];
            let mut col = cend - head[4];
            while col < line.len() && col < cend && col < head[0] + head[2] - head[4] {
              let color = if nc > 2 {
                cfa.color_at(row, col)
              } else {
                0 // match all colors
              };
              // This matches for R (0) and B (2), not for G1 (1) and G2 (3)
              if (color & 1) == 0 {
                let pix = line[col] as f32 * mult[color];
                line[col] = clamp(pix as i32, 0, 65535) as u16;
              }
              for c in (0..nc).step_by(2) {
                mult[c] += mult[c + 1];
              }
              col += 1;
            } // end while
          }

          for x in 0..wide {
            for c in (0..nc).step_by(2) {
              mrow[c * wide + x] += mrow[(c + 1) * wide + x];
            }
          }
          row += 1;
        }
      }
    }
  }

  /// Apply raw sensor corrections
  fn correct_raw_blacklevel(&self, img: &mut PixU16, calib: &SensorCalibration) -> Result<()> {
    // Remove black level and ajdust individual row/col blacklevel
    let q_blacklevel = calib.q_blacklevel.as_ref().filter(|(cblack, rblack)| {
      let matches = cblack.len() == img.height * 2 && rblack.len() == img.width * 2;
      if !matches {
        warn!(
          "IIQ: {} row and {} column black levels don't match the image of {}x{} pixels, using the basic black level",
          cblack.len(),
          rblack.len(),
          img.width,
          img.height
        );
      }
      matches
    });
    if let Some((cblack, rblack)) = q_blacklevel
      && let Some((split_col, split_row)) = self.quadrant_split("black level of the quadrants")
    {
      let black = calib.blacklevel as i32;
      img.par_pixel_rows_mut().enumerate().for_each(|(row, line)| {
        let qr = if row >= split_row { 1 } else { 0 };
        for (col, pix) in line.iter_mut().enumerate() {
          let qc = if col >= split_col { 1 } else { 0 };
          let x = *pix as i32 - black + (cblack[row * 2 + qc] as i32) + (rblack[col * 2 + qr] as i32);
          *pix = clamp(x, 0, 0xffff) as u16;
        }
      });
    } else {
      log::debug!("No q_blacklevel correction available, just apply basic blacklevel correction");
      img.for_each(|pix| clamp(pix as i32 - calib.blacklevel as i32, 0, 0xffff) as u16);
    }
    Ok(())
  }

  /// Apply raw sensor corrections
  fn correct_raw(&self, img: &mut PixU16, calib: &SensorCalibration) -> Result<()> {
    self.correct_raw_blacklevel(img, calib)?;
    self.correct_raw_poly_half(img, calib)?;
    self.correct_raw_poly_full(img, calib)?;
    self.correct_raw_q_mul(img, calib)?;
    self.correct_raw_q_combined(img, calib)?;
    self.correct_raw_q_linearization(img, calib)?;
    self.correct_raw_defects(img, calib)?;
    Self::correct_raw_flatfield(img, &self.camera.supported()?.cfa, &calib.flats);
    Ok(())
  }

  fn dimension(&self) -> Result<(usize, usize)> {
    let width = self.makernotes.get(&IiqTag::Width.into());
    let height = self.makernotes.get(&IiqTag::Height.into());

    match (width, height) {
      (Some(width), Some(height)) => Ok((
        width.1.force_usize(0), //
        height.1.force_usize(0),
      )),
      _ => Err(RawlerError::decoder_failed("Unable to find width/height in IIQ makernotes".to_string())),
    }
  }

  fn sensor_margins(&self) -> Option<(usize, usize)> {
    let left = self.makernotes.get(&IiqTag::MarginLeft.into());
    let top = self.makernotes.get(&IiqTag::MarginTop.into());

    match (left, top) {
      (Some(left), Some(top)) => Some((
        left.1.force_usize(0), //
        top.1.force_usize(0),  //
      )),
      _ => {
        warn!("Unable to find sensor margins in IIQ makernotes");
        None
      }
    }
  }

  /// Area of the image on the sensor, without the masked borders
  fn image_area(&self, sensor: Dim2) -> Option<Rect> {
    let (left, top) = self.sensor_margins()?;
    let width = self.makernotes.get(&IiqTag::ImageWidth.into())?.1.force_usize(0);
    let height = self.makernotes.get(&IiqTag::ImageHeight.into())?.1.force_usize(0);
    let area = Rect::new(Point::new(left, top), Dim2::new(width, height));
    if area.is_empty() || left + width > sensor.w || top + height > sensor.h {
      warn!(
        "IIQ: image area {:?} is empty or exceeds the sensor of {}x{} pixels, ignoring it",
        area, sensor.w, sensor.h
      );
      return None;
    }
    Some(area)
  }

  fn sensor_temp(&self) -> Result<Option<f32>> {
    let sensor_temp = self.makernotes.get(&IiqTag::SensorTemperature1.into());
    if let Some((_, data)) = sensor_temp {
      let x = data.force_u32(0);
      let flt = f32::from_bits(x);
      debug!("Sensor temp: {}", flt);
      return Ok(Some(flt));
    }
    Ok(None)
  }

  fn split_column(&self) -> Result<Option<usize>> {
    let split_col = self.makernotes.get(&IiqTag::SplitCol.into());

    match split_col {
      Some(col) => Ok(Some(col.1.force_usize(0))),
      _ => Ok(None),
    }
  }

  /// Split column and row of the sensor quadrants for `correction`, with a
  /// warning if the file doesn't have them
  fn quadrant_split(&self, correction: &str) -> Option<(usize, usize)> {
    let split = self.split_offsets().ok().flatten();
    if split.is_none() {
      warn!("IIQ: the {} needs the split column and row, skipping it", correction);
    }
    split
  }

  fn split_offsets(&self) -> Result<Option<(usize, usize)>> {
    let split_col = self.makernotes.get(&IiqTag::SplitCol.into());
    let split_row = self.makernotes.get(&IiqTag::SplitRow.into());

    match (split_col, split_row) {
      (Some(col), Some(row)) => Ok(Some((col.1.force_usize(0), row.1.force_usize(0)))),
      _ => Ok(None),
    }
  }

  fn split_blacks(&self, file: &RawSource) -> std::io::Result<Option<(Vec<i16>, Vec<i16>)>> {
    let black_col = self.makernotes.get(&IiqTag::BlackCol.into());
    let black_row = self.makernotes.get(&IiqTag::BlackRow.into());

    match (black_col, black_row) {
      (Some(black_col), Some(black_row)) => {
        // Levels of i16, a view of them fails for levels beyond the end of the file
        let levels = |(len, offset): &(usize, tiff::Value)| -> std::io::Result<Vec<i16>> {
          let data = file.subview(offset.force_u64(0) + 8, (len / 2 * 2) as u64)?;
          Ok(data.as_chunks::<2>().0.iter().map(|level| i16::from_le_bytes(*level)).collect())
        };
        Ok(Some((levels(black_col)?, levels(black_row)?)))
      }
      _ => Ok(None),
    }
  }

  fn compression_mode(&self) -> Result<IiqCompression> {
    match self.makernotes.get(&IiqTag::Format.into()) {
      Some(mode) => {
        let code = mode.1.force_u32(0);
        Ok(IiqCompression::try_from(code as usize)?)
      }
      _ => Err(RawlerError::decoder_failed("Unable to find compression mode in IIQ makernotes".to_string())),
    }
  }

  fn data_offset(&self) -> Result<(u64, usize)> {
    match self.makernotes.get(&IiqTag::DataOffset.into()) {
      Some(mode) => Ok((
        (mode.1.force_u64(0) + 8), //
        mode.0,
      )),
      _ => Err(RawlerError::decoder_failed("Unable to find data offset in IIQ makernotes".to_string())),
    }
  }

  fn strip_offset(&self) -> Result<(u64, usize)> {
    match self.makernotes.get(&IiqTag::StripOffset.into()) {
      Some(mode) => Ok((
        (mode.1.force_u64(0) + 8), //
        mode.0,
      )),
      _ => Err(RawlerError::decoder_failed("Unable to find strip offset in IIQ makernotes".to_string())),
    }
  }

  /// The sensor is mounted rotated in the camera back, so the makernote wins
  /// over the TIFF tag, which is not rotated with the sensor
  fn orientation(&self) -> Option<Orientation> {
    match self.makernotes.get(&IiqTag::Orientation.into()) {
      Some(entry) => Some(match entry.1.force_u32(0) & 3 {
        1 => Orientation::Rotate90,
        2 => Orientation::Rotate270,
        3 => Orientation::Rotate180,
        _ => Orientation::Normal,
      }),
      None => Orientation::from_tiff(self.tiff.root_ifd()),
    }
  }

  fn wb_offset(&self) -> Result<u64> {
    match self.makernotes.get(&IiqTag::WhiteBalance.into()) {
      Some(mode) => Ok((mode.1.force_u64(0) + 8) as u64),
      _ => Err(RawlerError::decoder_failed("Unable to find whitebalance offset in IIQ makernotes".to_string())),
    }
  }

  fn blacklevel(&self) -> Result<u16> {
    match self.makernotes.get(&IiqTag::BlackLevel.into()) {
      Some(mode) => Ok(mode.1.force_u16(0)),
      _ => Err(RawlerError::decoder_failed("Unable to find blacklevel in IIQ makernotes".to_string())),
    }
  }

  #[allow(unused)]
  fn camera_model(&self) -> Result<Option<&String>> {
    match self.makernotes.get(&IiqTag::CameraModel.into()) {
      Some(model) => Ok(model.1.as_string()),
      _ => Ok(None),
    }
  }

  fn lens_model(&self) -> Result<Option<&String>> {
    match self.makernotes.get(&IiqTag::LensModel.into()) {
      Some(model) => Ok(model.1.as_string()),
      _ => Ok(None),
    }
  }

  fn new_sensor_correction(&self, file: &RawSource, blacklevel: u16) -> std::io::Result<SensorCalibration> {
    let q_blacklevel = self.split_blacks(file)?;

    match self.makernotes.get(&IiqTag::SensorCorrection.into()) {
      Some((_len, offset)) => {
        let offset = offset.force_u64(0);
        debug!("Sensor correction data offset: {}", offset);
        let mut stream = file.reader();
        stream.seek(SeekFrom::Start(offset + 8))?;
        let bytes_to_entries = stream.read_u32::<LittleEndian>()? as u64;
        stream.seek(SeekFrom::Start(offset + bytes_to_entries))?;
        let entries_count = stream.read_u32::<LittleEndian>()? as usize;
        stream.seek(SeekFrom::Current(4))?; // skip 4 bytes

        let mut poly_curve_half = None;
        let mut poly_curve_full = None;
        let mut quadrant_linearization = None;
        let mut quadrant_multipliers = None;
        let mut quadrant_combined = None;
        let mut defects = None;
        let mut flats = Vec::new();
        let unexpected_length = |tag, len| warn!("IIQ: sensor correction 0x{:x} has an unexpected length of {} bytes, ignoring it", tag, len);

        //println!("Entry count:{}", entries_count);
        for _ in 0..entries_count {
          let tag = stream.read_u32::<LittleEndian>()?;
          let len = stream.read_u32::<LittleEndian>()?;
          let tag_offset = stream.read_u32::<LittleEndian>()? as u64;

          //println!("Tag 0x{:x}, count: {}, data: {}", tag, len, tag_offset);
          let stream_pos = stream.stream_position()?;
          match tag {
            0x0400 => {
              stream.seek(SeekFrom::Start(offset + tag_offset))?;
              // Each defect has 8 bytes, like dcraw a remainder is ignored
              let mut defect_list = Vec::new();
              for _ in 0..(len / 8) {
                let col = stream.read_u16::<LittleEndian>()? as usize;
                let row = stream.read_u16::<LittleEndian>()? as usize;
                let typ = stream.read_u16::<LittleEndian>()?;
                let reserved = stream.read_u16::<LittleEndian>()?;
                //println!("Defect: col: {}, row: {}, typ: {}, res: {}", col, row, typ, reserved);
                defect_list.push(Defect { col, row, typ, reserved });
              }
              defects = Some(defect_list);
            }

            0x401 | 0x410 | 0x416 | 0x40b => {
              debug!("Found flat field correction: 0x{:x}", tag);
              stream.seek(SeekFrom::Start(offset + tag_offset))?;
              let mut head = [0; 8];
              let mut data = Vec::new();
              for entry in head.iter_mut() {
                *entry = stream.read_u16::<LittleEndian>()?;
              }
              if tag == 0x401 {
                for _ in (8..(len)).step_by(size_of::<f32>()) {
                  data.push(stream.read_f32::<LittleEndian>()?);
                }
              } else {
                for _ in (8..(len)).step_by(size_of::<u16>()) {
                  data.push(stream.read_u16::<LittleEndian>()? as f32 / 32768.0);
                }
              }
              flats.push(FlatField { typ: tag, head, data });
            }

            // Like in dcraw, the 8 coefficients follow a value of 4 bytes
            0x0419 if (len as usize) < 4 + 8 * size_of::<f32>() => unexpected_length(tag, len),
            0x0419 => {
              stream.seek(SeekFrom::Start(offset + tag_offset + 4))?;
              let mut data = [f32::NAN; 8];
              for entry in data.iter_mut() {
                *entry = stream.read_f32::<LittleEndian>()?;
              }
              poly_curve_half = Some(data);
            }
            0x041a if len as usize != 4 * size_of::<f32>() => unexpected_length(tag, len),
            0x041a => {
              stream.seek(SeekFrom::Start(offset + tag_offset))?;
              let mut data = [f32::NAN; 4];
              for entry in data.iter_mut() {
                *entry = stream.read_f32::<LittleEndian>()?;
              }
              poly_curve_full = Some(data);
            }
            // there is an extra value...
            0x041f if len as usize != 2 * 2 * 16 * size_of::<u32>() + 4 => unexpected_length(tag, len),
            0x041f => {
              stream.seek(SeekFrom::Start(offset + tag_offset))?;
              let mut data = vec![0_u16; 2 * 2 * 16];
              for entry in data.iter_mut() {
                *entry = stream.read_u32::<LittleEndian>()? as u16;
              }
              quadrant_linearization = Some(data);
            }
            0x041e => {
              stream.seek(SeekFrom::Start(offset + tag_offset))?;
              let mut data = [f32::NAN; 19];
              for entry in data.iter_mut() {
                *entry = stream.read_f32::<LittleEndian>()?;
              }
              quadrant_multipliers = Some(data);
            }
            0x0431 => {
              stream.seek(SeekFrom::Start(offset + tag_offset))?;

              let mut x_coords = [0; 7];
              for coord in x_coords.iter_mut() {
                *coord = stream.read_u32::<LittleEndian>()? as usize;
              }

              let mut y_coords = [0; 7 * 4];
              for coord in y_coords.iter_mut() {
                *coord = stream.read_u32::<LittleEndian>()? as usize;
              }

              quadrant_combined = Some((x_coords, y_coords));
            }

            _ => {}
          }
          stream.seek(SeekFrom::Start(stream_pos))?; // restore
        }

        Ok(SensorCalibration {
          poly_curve_half,
          poly_curve_full,
          quadrant_linearization,
          quadrant_multipliers,
          quadrant_combined,
          defects,
          flats,
          blacklevel,
          q_blacklevel,
        })
      }
      _ => {
        warn!("IIQ: no sensor calibration data, only the black level is corrected");
        Ok(SensorCalibration {
          poly_curve_half: None,
          poly_curve_full: None,
          quadrant_linearization: None,
          quadrant_multipliers: None,
          quadrant_combined: None,
          defects: None,
          flats: Vec::new(),
          blacklevel,
          q_blacklevel,
        })
      }
    }
  }

  fn fix_bad_column(camera: &Camera, img: &mut PixU16, defect: &Defect) {
    let col = defect.col;
    // The fix uses the pixels 2 columns left and right
    if col < 2 || col + 2 >= img.width {
      warn!("IIQ: bad column {} is at the edge of the image, skipping it", col);
      return;
    }
    for row in 2..img.height.saturating_sub(2) {
      match camera.cfa.color_at(row, col) {
        cfa::CFA_COLOR_G => {
          // Do green pixels. Let's pretend we are in "G" pixel, in the middle:
          //   G=G
          //   BGB
          //   G0G
          // We accumulate the values 4 "G" pixels form diagonals, then check which
          // of 4 values is most distant from the mean of those 4 values, subtract
          // it from the sum, average (divide by 3) and round to nearest int.

          let mut max = 0;
          let mut val = [0_u16; 4];
          let mut dev = [0_i32; 4];

          val[0] = *img.at(row - 1, col - 1);
          val[1] = *img.at(row + 1, col - 1);
          val[2] = *img.at(row - 1, col + 1);
          val[3] = *img.at(row + 1, col + 1);

          let sum: i32 = val.iter().map(|x| *x as i32).sum();

          for (i, v) in val.iter().enumerate() {
            dev[i] = (((*v as i32) * 4_i32) - sum).abs();
            if dev[max] < dev[i] {
              max = i;
            }
          }
          let three_pixels = sum - val[max] as i32;
          debug_assert!(three_pixels >= 0);
          *img.at_mut(row, col) = ((three_pixels + 1) / 3) as u16;
          //image[row * width + col] = ((three_pixels + 1) / 3) as u16;
        }
        cfa::CFA_COLOR_R | cfa::CFA_COLOR_B => {
          // Do non-green pixels. Let's pretend we are in "R" pixel, in the middle:
          //   RG=GR
          //   GB=BG
          //   RGRGR
          //   GB0BG
          //   RG0GR
          // We have 6 other "R" pixels - 2 by horizontal, 4 by diagonals.
          // We need to combine them, to get the value of the pixel we are in.

          let diags = *img.at(row + 2, col - 2) as u32 + // 1
           *img.at(row - 2, col - 2) as u32 + // 2
           *img.at(row + 2, col + 2) as u32 + // 3
           *img.at(row - 2, col + 2) as u32; // 4

          let horiz: u32 = *img.at(row, col - 2) as u32 + *img.at(row, col + 2) as u32;

          // But this is not just averaging, we bias towards the horizontal pixels.
          *img.at_mut(row, col) = (diags as f32 * 0.0732233 + horiz as f32 * 0.3535534).round() as u16;
        }
        _ => {
          panic!("Other colors should not appear here");
        }
      }
    }
  }

  /// Extract white balance parameters
  fn get_wb(&self, file: &RawSource, wb_offset: u64) -> std::io::Result<[f32; 4]> {
    let buffer = file.subview(wb_offset, 3 * 4)?;
    Ok([LEf32(buffer, 0), LEf32(buffer, 4), LEf32(buffer, 8), f32::NAN])
  }

  /// Extract the matrix from white balanced camera values to ROMM (ProPhoto) RGB
  fn romm_matrix(&self, file: &RawSource) -> std::io::Result<Option<[[f32; 3]; 3]>> {
    match self.makernotes.get(&IiqTag::RommMatrix.into()) {
      Some((byte_count, value)) if *byte_count == 9 * 4 => {
        let buffer = file.subview(value.force_u64(0) + 8, 9 * 4)?;
        Ok(Some(std::array::from_fn(|row| std::array::from_fn(|col| LEf32(buffer, (row * 3 + col) * 4)))))
      }
      _ => Ok(None),
    }
  }

  /// Get lens description by analyzing TIFF tags and makernotes
  fn get_lens_description(&self) -> Result<Option<&'static LensDescription>> {
    match self.lens_model()? {
      Some(model) => {
        let resolver = LensResolver::new()
          .with_mounts(&[String::from("phaseone")])
          .with_lens_keyname(Some(model))
          .with_camera(self.camera.metadata());
        Ok(resolver.resolve())
      }
      None => Ok(None),
    }
  }

  /// Decode IIQ S format
  /// Same as IIQ L, but requires a linearization curve
  fn decode_nonlinear(buffer: &[u8], strips: &[u8], width: usize, height: usize, bits: u8, dummy: bool) -> std::result::Result<PixU16, String> {
    // We fake the bit count as 16, then no shift happens in decoding.
    // Then we can linearize back the data.
    let mut img = Self::decode_compressed(buffer, strips, width, height, 16, dummy)?;

    let value_shift: u32 = MAX_BITDEPTH - (bits as u32);

    if !dummy {
      let mut curve: [u16; 256] = [0; 256];

      for (i, out) in curve.iter_mut().enumerate() {
        *out = (i as f32 * i as f32 / 3.969 + 0.5) as u16;
      }

      // For IIQ_S format, data is 8 bit and non-linear.
      // Use the linearization curve to get back original data.
      img.for_each(|pix| if pix < 256 { curve[pix as usize] << value_shift } else { pix });
    }
    Ok(img)
  }

  /// Decoder for IIQ uncompressed
  fn decode_uncompressed(buffer: &[u8], width: usize, height: usize, bits: u8, dummy: bool) -> std::result::Result<PixU16, String> {
    let need = width * height * 2;
    if buffer.len() < need {
      return Err(format!("IIQ: buffer too short ({} < {})", buffer.len(), need));
    }
    let value_shift: u32 = MAX_BITDEPTH - (bits as u32);
    decompress_lines_fn(
      width,
      height,
      dummy,
      &(|out: &mut [u16], row| {
        for (i, word) in out.iter_mut().enumerate() {
          *word = LEu16(buffer, row * (width * 2) + i * 2) << value_shift;
        }
        Ok(())
      }),
    )
  }

  /// Data of a row of compressed IIQ files, at the offset from the strip table
  fn row_data<'b>(buffer: &'b [u8], strips: &[u8], row: usize) -> std::result::Result<&'b [u8], String> {
    let offset = strips
      .get(row * 4..row * 4 + 4)
      .map(|bytes| LEu32(bytes, 0) as usize)
      .ok_or_else(|| format!("IIQ: strip table has no offset for row {}", row))?;
    buffer
      .get(offset..)
      .ok_or_else(|| format!("IIQ: offset {} of row {} is behind the end of the data", offset, row))
  }

  /// Decoder for IIQ L / S data
  pub(crate) fn decode_compressed(buffer: &[u8], strips: &[u8], width: usize, height: usize, bits: u8, dummy: bool) -> std::result::Result<PixU16, String> {
    let value_shift: u32 = MAX_BITDEPTH - (bits as u32);
    let lens: [u32; 10] = [8, 7, 6, 9, 11, 10, 5, 12, 14, 13];
    decompress_lines_fn(
      width,
      height,
      dummy,
      &(|out: &mut [u16], row| {
        let mut pump = BitPumpMSB32::new(Self::row_data(buffer, strips, row)?);
        let mut pred = [0_u32; 2];
        let mut len = [0_u32; 2];
        for (col, pixout) in out.chunks_exact_mut(1).enumerate() {
          if col >= (width & 0xfffffff8) {
            len[0] = 14;
            len[1] = 14;
          } else if col & 7 == 0 {
            for i in 0..2 {
              let mut j: usize = 0;
              while j < 5 && pump.get_bits(1)? == 0 {
                j += 1
              }
              if j > 0 {
                len[i] = lens[(j - 1) * 2 + pump.get_bits(1)? as usize];
              }
            }
          }
          let i = len[col & 1];
          pred[col & 1] = match i {
            14 => pump.get_bits(16)?,
            0 => return Err(format!("IIQ: no code length for column {} of row {}", col, row)),
            // The prediction wraps around like the int of dcraw, only the low 16 bits are used
            _ => pred[col & 1].wrapping_add(pump.get_bits(i)?).wrapping_add(1).wrapping_sub(1 << (i - 1)),
          };
          pixout[0] = (pred[col & 1] as u16) << value_shift;
        }
        Ok(())
      }),
    )
  }

  /// Decoder for IIQ S v2
  /// This compression stores row pixels in clusters of 8 pixels.
  fn decode_compressed_sv2(buffer: &[u8], strips: &[u8], width: usize, height: usize, bits: u8, dummy: bool) -> std::result::Result<PixU16, String> {
    // Correction shift for pixel values
    let value_shift: u32 = MAX_BITDEPTH - (bits as u32);
    // We can decode each row independently
    decompress_lines_fn(
      width,
      height,
      dummy,
      &(|out: &mut [u16], row| {
        let mut pump = BitPumpMSB32::new(Self::row_data(buffer, strips, row)?);
        let base_bits = pump.get_bits(16)? & 7; // first 3 bits
        let mut bit_check: [u32; 2] = [0, 0]; // for even and odd pixels
        let mut prev_pix_value = [0_u32, 0]; // for even and odd pixels

        let pix_sub_init = 17 - base_bits;

        for cluster in out.chunks_mut(8) {
          // When we have a full cluster with 8 possible pixels
          if cluster.len() == 8 {
            // Initialize the bit_check values for even and odd.
            for i in 0..bit_check.len() {
              let idx = pump.peek_bits(7)? as usize;
              pump.consume_bits(2);
              if idx >= 32 {
                // If idx is larger then lookup array, the value
                // is calculated by re-using the previous value.
                bit_check[i] = ((idx as u32 >> 5) + bit_check[i])
                  .checked_sub(2)
                  .ok_or_else(|| format!("IIQ Sv2: invalid bit count in row {}", row))?;
              } else {
                // Otherwise, just do a lookup.
                debug_assert!(idx < 32);
                bit_check[i] = SV2_BIT_INDICATOR[idx];
                pump.consume_bits(SV2_SKIP_BITS[idx] as u32);
              }
            }

            let pump_savepoint = pump.clone(); // savepoint for error recovery
            let x = pump.peek_bits(3)? as usize; // 3 bits are max 7, so it's safe as array index
            pump.consume_bits(SV2_USED_CORR[x]);

            let nbits = base_bits + SV2_EXTRA_BITS[x];
            let corr = [
              bit_check[0].saturating_sub(SV2_EXTRA_BITS[x]), // even
              bit_check[1].saturating_sub(SV2_EXTRA_BITS[x]), // odd
            ];
            let diff_shift = |bit_check: u32| {
              pix_sub_init
                .checked_sub(bit_check)
                .ok_or_else(|| format!("IIQ Sv2: bit count {} in row {} exceeds {}", bit_check, row, pix_sub_init))
            };
            let diff = [
              0xFFFF_u32 >> diff_shift(bit_check[0])?, // even
              0xFFFF_u32 >> diff_shift(bit_check[1])?, // odd
            ];
            // Detector for invalid decompression result
            let mut check_val = 0;
            // decompress cluster
            for i in 0..8 {
              let value = if bit_check[i & 1] == 9 {
                // Just get the value straight out the bit pump
                pump.get_bits(14)?
              } else {
                // Otherwise reconstruct the value, invalid values wrap
                // around and the checker finds them
                let tmp = prev_pix_value[i & 1]
                  .wrapping_add(pump.get_bits(nbits)? << corr[i & 1])
                  .wrapping_sub(diff[i & 1]);
                check_val |= tmp; // Apply to checker
                tmp
              };
              cluster[i] = (value as u16) << value_shift;
              prev_pix_value[i & 1] = value;
            }

            // Check for decompressor errors
            if (check_val & ((1 << 14) - 1)) != check_val {
              warn!("Error in IIQ Sv2 decompressor, run error recovery");
              // restore pump
              pump = pump_savepoint;
              for i in 0..8 {
                let value = if bit_check[i & 1] == 9 {
                  // Just get the value straight out the bit pump
                  pump.get_bits(14)?
                } else {
                  // Otherwise reconstruct the value
                  let tmp = (prev_pix_value[i & 1] as i32)
                    .wrapping_add((pump.get_bits(nbits)? << corr[i & 1]) as i32)
                    .wrapping_sub(diff[i & 1] as i32);
                  clamp(tmp as i32, 0, 0x3FFF) as u32
                };
                cluster[i] = (value as u16) << value_shift;
                prev_pix_value[i & 1] = value;
              }
            }
          } else {
            // Not a full cluster, just take the full values from the bit pump
            for pix in cluster.iter_mut() {
              *pix = (pump.get_bits(14)? as u16) << value_shift;
            }
          }
        }
        Ok(())
      }),
    )
  }
}

/// Apply one of four lookup curves to each pixel, selected by the
/// sensor quadrant (split at `split_row` and `split_col`) the pixel is in.
fn apply_quadrant_curves(img: &mut PixU16, split_row: usize, split_col: usize, curves: &[[Vec<u16>; 2]; 2]) {
  img.par_pixel_rows_mut().enumerate().for_each(|(row, line)| {
    let qcurves = if row >= split_row { &curves[1] } else { &curves[0] };
    let (left, right) = line.split_at_mut(split_col.min(line.len()));
    left.iter_mut().for_each(|pix| *pix = qcurves[0][*pix as usize]);
    right.iter_mut().for_each(|pix| *pix = qcurves[1][*pix as usize]);
  });
}

#[allow(unused)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Ord, PartialOrd)]
enum IiqTag {
  Orientation = 0x100,
  /// Matrix from white balanced camera values to ROMM (ProPhoto) RGB
  RommMatrix = 0x106,
  WhiteBalance = 0x107,
  Width = 0x108,
  Height = 0x109,
  MarginLeft = 0x10a,
  MarginTop = 0x10b,
  ImageWidth = 0x10c,
  ImageHeight = 0x10d,
  Format = 0x10e,
  DataOffset = 0x10f,
  SensorCorrection = 0x110,
  SensorTemperature1 = 0x210,
  SensorTemperature2 = 0x211,
  StripOffset = 0x21c,
  BlackLevel = 0x21d,
  SplitCol = 0x222,
  BlackCol = 0x223,
  SplitRow = 0x224,
  BlackRow = 0x225,
  CameraModel = 0x410,
  LensModel = 0x412,
}

impl From<IiqTag> for u32 {
  fn from(v: IiqTag) -> Self {
    v as u32
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// IIQ file with a makernote of `count` entries, of which `entries` are
  /// given as (tag, type, byte count, data)
  fn iiq_makernote(count: u32, entries: &[(u32, u32, u32, u32)]) -> Vec<u8> {
    // TIFF header, makernote header, IFD offset 12 relative to the makernote
    let mut buf = vec![0; 16];
    buf.extend_from_slice(&12_u32.to_le_bytes());
    buf.extend_from_slice(&count.to_le_bytes());
    buf.extend_from_slice(&[0; 4]);
    for (tag, typ, byte_count, data) in entries {
      for value in [tag, typ, byte_count, data] {
        buf.extend_from_slice(&value.to_le_bytes());
      }
    }
    buf
  }

  #[test]
  fn compressed_without_code_length_is_an_error() {
    // The first bits are 1, the code lengths of the first 8 columns aren't set
    assert!(IiqDecoder::decode_compressed(&[0xff; 64], &[0; 4], 8, 1, 14, false).is_err());
  }

  #[test]
  fn compressed_prediction_wraps_around() -> std::result::Result<(), String> {
    // Code lengths of 8 for both columns (bits 010 twice), then a difference
    // of 0 that subtracts 127 from the prediction of 0
    let mut buf = vec![0; 64];
    buf[3] = 0x48;
    let image = IiqDecoder::decode_compressed(&buf, &[0; 4], 8, 1, 14, false)?;
    assert_eq!(image.pixels()[0], (1_u32.wrapping_sub(128) as u16) << 2);
    Ok(())
  }

  #[test]
  fn sv2_bit_count_underflow_is_an_error() {
    // A base of 0 bits, then an index of 32 for the first cluster, which
    // takes 2 from a bit count of 0
    let mut buf = vec![0; 64];
    buf[1] = 0x40;
    assert!(IiqDecoder::decode_compressed_sv2(&buf, &[0; 4], 8, 1, 14, false).is_err());
    // Zero data has no invalid bit counts
    assert!(IiqDecoder::decode_compressed_sv2(&[0; 64], &[0; 4], 8, 1, 14, false).is_ok());
  }

  #[test]
  fn flat_field_with_too_few_values_is_skipped() {
    // The cells need 4 values, the flat field has 3
    let flat = FlatField {
      typ: 0x40b,
      head: [1, 1, 8, 8, 4, 4, 0, 0],
      data: vec![2.0; 3],
    };
    let mut img = PixU16::new_with(vec![1000; 12 * 12], 12, 12);
    IiqDecoder::correct_raw_flatfield(&mut img, &cfa::CFA::new("GBRG"), &[flat]);
    assert!(img.pixels().iter().all(|pix| *pix == 1000));
  }

  #[test]
  fn bad_columns_at_the_edges_are_skipped() {
    let mut camera = Camera::new();
    camera.cfa = cfa::CFA::new("RGGB");
    let defect = |col| Defect {
      row: 0,
      col,
      typ: 131,
      reserved: 0,
    };
    let pixels: Vec<u16> = (0..8 * 8).map(|i| i as u16 * 10).collect();
    for col in [0, 1, 6, 7] {
      let mut img = PixU16::new_with(pixels.clone(), 8, 8);
      IiqDecoder::fix_bad_column(&camera, &mut img, &defect(col));
      assert_eq!(img.pixels(), &pixels[..], "column {col}");
    }
    let mut img = PixU16::new_with(pixels.clone(), 8, 8);
    IiqDecoder::fix_bad_column(&camera, &mut img, &defect(4));
    assert_ne!(img.pixels(), &pixels[..]);
    // An image of one row
    let mut img = PixU16::new_with(vec![0; 8], 8, 1);
    IiqDecoder::fix_bad_column(&camera, &mut img, &defect(4));
  }

  #[test]
  fn makernote_entries_beyond_the_file_are_an_error() {
    let file = RawSource::new_from_slice(&iiq_makernote(u32::MAX, &[]));
    assert!(new_makernote(&file, 8).is_err());
    // An ASCII value longer than the file
    let file = RawSource::new_from_slice(&iiq_makernote(1, &[(0x412, 1, u32::MAX, 0)]));
    assert!(new_makernote(&file, 8).is_err());
  }

  #[test]
  fn makernote_skips_sensor_correction_offsets_out_of_range() -> std::io::Result<()> {
    let sensor_correction = IiqTag::SensorCorrection as u32;
    let width = IiqTag::Width as u32;
    let file = RawSource::new_from_slice(&iiq_makernote(2, &[(sensor_correction, 1, 4, u32::MAX - 3), (width, 4, 4, 100)]));
    let entries = new_makernote(&file, 8)?;
    assert!(!entries.contains_key(&sensor_correction));
    assert_eq!(entries.get(&width).map(|(_, value)| value.force_u32(0)), Some(100));
    Ok(())
  }

  #[test]
  fn red_blue_flat_field_corrects_red_and_blue_pixels() {
    // One cell of 4x4 pixels at an odd row and column, red multiplier 2.0 and blue multiplier 0.5
    let flat = FlatField {
      typ: 0x40b,
      head: [1, 1, 8, 8, 4, 4, 0, 0],
      data: [2.0, 0.5].repeat(4),
    };
    let mut img = PixU16::new_with(vec![1000; 12 * 12], 12, 12);

    IiqDecoder::correct_raw_flatfield(&mut img, &cfa::CFA::new("GBRG"), &[flat]);

    for row in 0..12 {
      for col in 0..12 {
        let inside = (1..5).contains(&row) && (1..5).contains(&col);
        let expected = match (row % 2, col % 2) {
          (0, 1) if inside => 500,  // blue
          (1, 0) if inside => 2000, // red
          _ => 1000,
        };
        assert_eq!(*img.at(row, col), expected, "row {row}, col {col}");
      }
    }
  }
}
