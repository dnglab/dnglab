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
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::f32::NAN;
use std::io::SeekFrom;
use std::mem::size_of;

use crate::analyze::FormatDump;
use crate::bits::*;
use crate::cfa;
use crate::decoders::decode_threaded;
use crate::decoders::ok_image_with_blacklevels;
use crate::exif::Exif;
use crate::formats::tiff;
use crate::formats::tiff::reader::TiffReader;
use crate::formats::tiff::GenericTiffReader;
use crate::imgop::spline::Spline;
use crate::imgop::Point;
use crate::lens::LensDescription;
use crate::lens::LensResolver;
use crate::pixarray::PixU16;
use crate::pumps::BitPump;
use crate::pumps::BitPumpMSB32;
use crate::RawFile;
use crate::RawImage;
use crate::RawLoader;
use crate::RawlerError;
use crate::Result;

use super::Camera;
use super::Decoder;
use super::RawDecodeParams;
use super::RawMetadata;

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
  sensor_margins: (usize, usize), // left, top
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

impl From<usize> for IiqCompression {
  fn from(v: usize) -> Self {
    match v {
      0 => Self::Uncompressed,
      1 => Self::Raw1,
      2 => Self::Raw2,
      3 => Self::IIQ_L,
      5 => Self::IIQ_S,
      6 => Self::IIQ_Sv2,
      8 => Self::IIQ_L16,
      _ => panic!("Unsupported IIQ format: {}", v),
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
  camera: Camera,
  #[allow(unused)]
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
  makernotes: IiqMakernotes,
}

impl<'a> IiqDecoder<'a> {
  pub fn new(file: &mut RawFile, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<IiqDecoder<'a>> {
    debug!("IIQ decoder choosen");
    let camera = rawloader.check_supported(tiff.root_ifd())?;
    let makernotes = new_makernote(file, 8).map_err(|ioerr| RawlerError::with_io_error("load IIQ makernotes", &file.path, ioerr))?;
    Ok(IiqDecoder {
      camera,
      tiff,
      rawloader,
      makernotes,
    })
  }
}

fn new_makernote(file: &mut RawFile, moffset: u64) -> std::io::Result<HashMap<u32, (usize, tiff::Value)>> {
  // All makernote offsets are not absolute to file, but to start of makernote data.
  // The makernote data starts straight after the TIFF header (8 bytes).
  // If this may change in future, offsets must be revalidated.
  assert_eq!(moffset, 8);
  let stream = file.inner();
  stream.seek(SeekFrom::Start(moffset + 8))?; // Skip first 8 bytes of makernote
  let ifd = stream.read_u32::<LittleEndian>()? as u64;
  stream.seek(SeekFrom::Start(moffset + ifd))?;
  let entry_count = stream.read_u32::<LittleEndian>()? as usize; // Skip 8 bytes in IFD

  let _ = stream.read_u32::<LittleEndian>()?; // Skip it
  let mut buf = vec![0; entry_count * 16];
  stream.read_exact(&mut buf)?;

  let mut entries: HashMap<u32, (usize, tiff::Value)> = HashMap::with_capacity(entry_count);
  let mut buf = &buf[..];

  for _ in 0..entry_count {
    let tag = LEu32(buf, 0);
    let typ = LEu32(buf, 4);
    let byte_count = LEu32(buf, 8) as usize;
    let data = LEu32(buf, 12) as usize;
    //println!("Tag 0x{:x}, typ: {}, count: {}, data: {}", tag, typ, byte_count, data);
    match typ {
      // Sensor calibration file, it's like a TIFF file and stored with ASCII type
      1 if tag == IiqTag::SensorCorrection.into() => {
        entries.insert(tag, (byte_count, tiff::Value::Long(vec![data as u32 + moffset as u32])));
      }
      // Others should be just ASCII strings
      1 => {
        let mut v = vec![0; byte_count];
        stream.seek(SeekFrom::Start(moffset + data as u64))?; // skip header
        stream.read_exact(&mut v)?;
        let value = tiff::Value::Ascii(tiff::TiffAscii::new_from_raw(&v));
        entries.insert(tag, (byte_count, value));
      }
      // Short values
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
    buf = &buf[16..];
  }
  Ok(entries)
}

impl<'a> Decoder for IiqDecoder<'a> {
  fn raw_image(&self, file: &mut RawFile, _params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let fmt = self.compression_mode()?;

    let wb_offset = self.wb_offset()?;
    let (width, height) = self.dimension()?;
    let (data_offset, data_len) = self.data_offset()?;

    let (strips_offset, strips_len) = if fmt != IiqCompression::Uncompressed { self.strip_offset()? } else { (0, 0) };

    debug!("Strips offset: {}", strips_offset);
    debug!("data offset: {}", data_offset);
    debug!("RAW IIQ Format: {:?}", fmt);

    if width == 0 || height == 0 {
      return Err(RawlerError::General("IIQ: couldn't find width and height".to_string()));
    }

    let data = file
      .subview(data_offset, data_len as u64)
      .map_err(|ioerr| RawlerError::with_io_error("load IIQ data", &file.path, ioerr))?;
    let strips = file
      .subview(strips_offset, strips_len as u64)
      .map_err(|ioerr| RawlerError::with_io_error("load IIQ strips", &file.path, ioerr))?;

    let mut image = match fmt {
      IiqCompression::Raw1 => todo!(),
      IiqCompression::Raw2 => todo!(),
      IiqCompression::Uncompressed => Self::decode_uncompressed(&data, width, height, 14, dummy),
      IiqCompression::IIQ_L => Self::decode_compressed(&data, &strips, width, height, 14, dummy),
      IiqCompression::IIQ_L16 => Self::decode_compressed(&data, &strips, width, height, 16, dummy),
      IiqCompression::IIQ_S => Self::decode_nonlinear(&data, &strips, width, height, 14, dummy),
      IiqCompression::IIQ_Sv2 => Self::decode_compressed_sv2(&data, &strips, width, height, 14, dummy),
    };

    let black = self.blacklevel().unwrap_or(0);

    if !dummy {
      let senscorr = self
        .new_sensor_correction(file, black)
        .map_err(|ioerr| RawlerError::with_io_error("read IIQ sensor calibration data", &file.path, ioerr))?;
      self.correct_raw(&mut image, &senscorr)?;
    }

    let blacklevel = [0, 0, 0, 0];
    let cpp = 1;
    let wb = self
      .get_wb(file, wb_offset)
      .map_err(|ioerr| RawlerError::with_io_error("read IIQ white balance", &file.path, ioerr))?;

    ok_image_with_blacklevels(self.camera.clone(), width, height, cpp, wb, blacklevel, image.into_inner())
  }

  fn format_dump(&self) -> FormatDump {
    FormatDump::Iiq(IiqFormat {
      //tiff: self.tiff.clone(),
      makernotes: self.makernotes.clone(),
    })
  }

  fn raw_metadata(&self, _file: &mut RawFile, _params: RawDecodeParams) -> Result<RawMetadata> {
    let exif = Exif::new(self.tiff.root_ifd())?;
    let mdata = RawMetadata::new_with_lens(&self.camera, exif, self.get_lens_description()?.cloned());
    Ok(mdata)
  }
}

impl<'a> IiqDecoder<'a> {
  /// Apply correction information for sensor defects
  /// Only bad columns are corrected for now.
  fn correct_raw_defects(&self, img: &mut PixU16, senscorr: &SensorCalibration) -> Result<()> {
    if let Some(defects) = &senscorr.defects {
      debug!("Apply defect correction");

      for def in defects.iter() {
        if def.col >= img.width {
          continue;
        };
        match def.typ {
          131 | 137 => {
            debug!("Correct bad colum: {}", def.col);
            self.fix_bad_column(img, def);
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
      let split_col = self.split_column()?.expect("Must have split column");

      let mut poly = *poly;
      let sensor_temp = self.sensor_temp()?.expect("Half polynom curve correction requires the sensor temp.");
      poly[3] += (sensor_temp - poly[7]) * poly[6] + 1.0;

      let mut curve = vec![0_u16; 0x10000];
      for (i, x) in curve.iter_mut().enumerate() {
        let num = (poly[5] * i as f32 + poly[3]) * i as f32 + poly[1];
        *x = clamp(num as i32, 0, 0xffff) as u16;
      }
      // Only apply for right side of image
      for row in img.pixel_rows_mut() {
        for pix in row.iter_mut().skip(split_col) {
          *pix = curve[*pix as usize];
        }
      }
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
      let (split_col, split_row) = self.split_offsets()?.expect("Must have split values");

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

      for qr in 0..2 {
        for qc in 0..2 {
          let cp = &mut control_points[qr][qc];
          // Some images may overflow 65535 here, but it's also in dcraw...
          // We clamp here to be <= then the final control point.
          cp[17].x = clamp(((cp[16].y * 65535) / cp[16].x) as i32, 0, 65535) as usize;
          cp[17].y = cp[17].x;
          cp[18] = Point::new(65535, 65535);
          let curve = Spline::new(cp).calculate_curve();

          let (start_row, end_row) = if qr > 0 { (split_row, img.height) } else { (0, split_row) };
          let (start_col, end_col) = if qc > 0 { (split_col, img.width) } else { (0, split_col) };

          for row in start_row..end_row {
            for col in start_col..end_col {
              let pix = img.at_mut(row, col);
              *pix = clamp(curve[*pix as usize] as i32, 0, 0xffff) as u16;
            }
          }
        }
      }
    }
    Ok(())
  }

  /// Apply quadrant multipliers correction
  fn correct_raw_q_mul(&self, img: &mut PixU16, senscorr: &SensorCalibration) -> Result<()> {
    if let Some(multipliers) = &senscorr.quadrant_multipliers {
      debug!("Apply quadrant multiplier correction");
      let (split_col, split_row) = self.split_offsets()?.expect("Must have split values");
      debug!("Split col: {}, row: {}", split_col, split_row);

      let mut qmul = [[NAN; 2]; 2];
      qmul[0][0] = 1.0 + multipliers[4];
      qmul[0][1] = 1.0 + multipliers[10];
      qmul[1][0] = 1.0 + multipliers[14];
      qmul[1][1] = 1.0 + multipliers[18];

      img.for_each_index(|pix, row, col| {
        let qr = if row >= split_row { 1 } else { 0 };
        let qc = if col >= split_col { 1 } else { 0 };
        let x = qmul[qr][qc] * (pix as f32);
        clamp(x as i32, 0, 0xffff) as u16
      });
    }
    Ok(())
  }

  /// Apply quadrant combined spline curve correction
  fn correct_raw_q_combined(&self, img: &mut PixU16, senscorr: &SensorCalibration) -> Result<()> {
    if let Some((coord_x, coord_y)) = &senscorr.quadrant_combined {
      debug!("Apply quadrant combined correction");
      let (split_col, split_row) = self.split_offsets()?.expect("Must have split values");

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

      for qr in 0..2 {
        for qc in 0..2 {
          let cp = &mut control_points[qr][qc];

          //for i in 0..9 {
          //  println!("X: {}", cp[i].x);
          //  //println!("Y: {}", cp[i].y);
          //}

          let curve = Spline::new(cp).calculate_curve();

          let (start_row, end_row) = if qr > 0 { (split_row, img.height) } else { (0, split_row) };
          let (start_col, end_col) = if qc > 0 { (split_col, img.width) } else { (0, split_col) };

          for row in start_row..end_row {
            for col in start_col..end_col {
              let pix = img.at_mut(row, col);
              *pix = curve[*pix as usize];
            }
          }
        }
      }
    }
    Ok(())
  }

  /// Apply flat field correction
  fn correct_raw_flatfield(&self, img: &mut PixU16, senscorr: &SensorCalibration) -> Result<()> {
    // Map the second green (on odd rows) to color 3
    let cfa = self
      .camera
      .cfa
      .map_colors(|row, _col, color| if color == cfa::CFA_COLOR_G && (row & 1 == 1) { 3 } else { color });

    for flat in senscorr.flats.iter() {
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
      let mut mrow = vec![0.0; nc * wide];
      let mut pump = flat.data.iter();
      let mut mult = [0.0; 4]; // One multiplier per color (R=0, G1=1, ...)

      for y in 0..high {
        for x in 0..wide {
          for c in (0..nc).step_by(2) {
            let num = *pump.next().expect("flat field correction data consumed but tried to get more");
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
          for x in 1..wide {
            for c in (0..nc).step_by(2) {
              mult[c] = mrow[c * wide + x - 1];
              mult[c + 1] = (mrow[c * wide + x] - mult[c]) / head[4] as f32;
            }
            let cend = head[0] + x * head[4];
            let mut col = cend - head[4];
            while col < img.width && col < cend && col < head[0] + head[2] - head[4] {
              let color = if nc > 2 {
                debug_assert!(row >= senscorr.sensor_margins.1);
                debug_assert!(col >= senscorr.sensor_margins.0);
                cfa.color_at(row - senscorr.sensor_margins.1, col - senscorr.sensor_margins.0)
              } else {
                0 // match all colors
              };
              // This matches for R (0) and B (2), not for G1 (1) and G2 (3)
              if (color & 1) == 0 {
                let pix = *img.at(row, col) as f32 * mult[color];
                *img.at_mut(row, col) = clamp(pix as i32, 0, 65535) as u16;
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
    Ok(())
  }

  /// Apply raw sensor corrections
  fn correct_raw_blacklevel(&self, img: &mut PixU16, calib: &SensorCalibration) -> Result<()> {
    // Remove black level and ajdust individual row/col blacklevel
    if let Some(q_blacklevel) = &calib.q_blacklevel {
      let black = calib.blacklevel as i32;
      let (split_col, split_row) = self.split_offsets()?.expect("Must have split values");
      let (cblack, rblack) = q_blacklevel;
      debug_assert_eq!(cblack.len(), img.height * 2);
      debug_assert_eq!(rblack.len(), img.width * 2);
      img.for_each_index(|pix, row, col| {
        let qr = if row >= split_row { 1 } else { 0 };
        let qc = if col >= split_col { 1 } else { 0 };
        let x = pix as i32 - black + (cblack[row * 2 + qc] as i32) + (rblack[col * 2 + qr] as i32);
        clamp(x, 0, 0xffff) as u16
      });
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
    self.correct_raw_flatfield(img, calib)?;
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
      _ => Err(RawlerError::General("Unable to find width/height in IIQ makernotes".to_string())),
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

  fn split_offsets(&self) -> Result<Option<(usize, usize)>> {
    let split_col = self.makernotes.get(&IiqTag::SplitCol.into());
    let split_row = self.makernotes.get(&IiqTag::SplitRow.into());

    match (split_col, split_row) {
      (Some(col), Some(row)) => Ok(Some((col.1.force_usize(0), row.1.force_usize(0)))),
      _ => Ok(None),
    }
  }

  fn split_blacks(&self, file: &mut RawFile) -> std::io::Result<Option<(Vec<i16>, Vec<i16>)>> {
    let black_col = self.makernotes.get(&IiqTag::BlackCol.into());
    let black_row = self.makernotes.get(&IiqTag::BlackRow.into());

    match (black_col, black_row) {
      (Some(black_col), Some(black_row)) => {
        let stream = file.inner();
        let (len, offset) = (black_col.0, black_col.1.force_u64(0));
        stream.seek(SeekFrom::Start(offset + 8))?;
        let mut cols = vec![0; len / 2]; // u16 size
        for entry in cols.iter_mut() {
          *entry = stream.read_i16::<LittleEndian>()?;
        }
        let (len, offset) = (black_row.0, black_row.1.force_u64(0));
        stream.seek(SeekFrom::Start(offset + 8))?;
        let mut rows = vec![0; len / 2]; // u16 size
        for entry in rows.iter_mut() {
          *entry = stream.read_i16::<LittleEndian>()?;
        }
        Ok(Some((cols, rows)))
      }
      _ => Ok(None),
    }
  }

  fn compression_mode(&self) -> Result<IiqCompression> {
    match self.makernotes.get(&IiqTag::Format.into()) {
      Some(mode) => {
        let code = mode.1.force_u32(0);
        Ok(IiqCompression::from(code as usize))
      }
      _ => Err(RawlerError::General("Unable to find compression mode in IIQ makernotes".to_string())),
    }
  }

  fn data_offset(&self) -> Result<(u64, usize)> {
    match self.makernotes.get(&IiqTag::DataOffset.into()) {
      Some(mode) => Ok((
        (mode.1.force_u64(0) + 8), //
        mode.0,
      )),
      _ => Err(RawlerError::General("Unable to find data offset in IIQ makernotes".to_string())),
    }
  }

  fn strip_offset(&self) -> Result<(u64, usize)> {
    match self.makernotes.get(&IiqTag::StripOffset.into()) {
      Some(mode) => Ok((
        (mode.1.force_u64(0) + 8), //
        mode.0,
      )),
      _ => Err(RawlerError::General("Unable to find strip offset in IIQ makernotes".to_string())),
    }
  }

  fn wb_offset(&self) -> Result<u64> {
    match self.makernotes.get(&IiqTag::WhiteBalance.into()) {
      Some(mode) => Ok((mode.1.force_u64(0) + 8) as u64),
      _ => Err(RawlerError::General("Unable to find whitebalance offset in IIQ makernotes".to_string())),
    }
  }

  fn blacklevel(&self) -> Result<u16> {
    match self.makernotes.get(&IiqTag::BlackLevel.into()) {
      Some(mode) => Ok(mode.1.force_u16(0)),
      _ => Err(RawlerError::General("Unable to find lacklevel in IIQ makernotes".to_string())),
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

  fn new_sensor_correction(&self, file: &mut RawFile, blacklevel: u16) -> std::io::Result<SensorCalibration> {
    let q_blacklevel = self.split_blacks(file)?;
    let sensor_margins = self.sensor_margins().unwrap_or((0, 0));

    match self.makernotes.get(&IiqTag::SensorCorrection.into()) {
      Some((_len, offset)) => {
        let offset = offset.force_u64(0);
        debug!("Sensor correction data offset: {}", offset);
        let stream = file.inner();
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
              assert_eq!(len % 4, 0);
              let mut defect_list = Vec::new();
              for _ in 0..(len / 4) {
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

            0x0419 => {
              stream.seek(SeekFrom::Start(offset + tag_offset))?;
              assert!(len as usize >= 8 * size_of::<f32>());
              let mut data = [NAN; 8];
              for entry in data.iter_mut() {
                *entry = stream.read_f32::<LittleEndian>()?;
              }
              poly_curve_half = Some(data);
            }
            0x041a => {
              stream.seek(SeekFrom::Start(offset + tag_offset))?;
              assert_eq!(len as usize, 4 * size_of::<f32>());
              let mut data = [NAN; 4];
              for entry in data.iter_mut() {
                *entry = stream.read_f32::<LittleEndian>()?;
              }
              poly_curve_full = Some(data);
            }
            0x041f => {
              stream.seek(SeekFrom::Start(offset + tag_offset))?;
              assert_eq!(len as usize, 2 * 2 * 16 * size_of::<u32>() + 4); // there is an extra value...
              let mut data = vec![0_u16; 2 * 2 * 16];
              for entry in data.iter_mut() {
                *entry = stream.read_u32::<LittleEndian>()? as u16;
              }
              quadrant_linearization = Some(data);
            }
            0x041e => {
              stream.seek(SeekFrom::Start(offset + tag_offset))?;
              let mut data = [NAN; 19];
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
          sensor_margins,
        })
      }
      _ => panic!("No sensor calibration data found."),
    }
  }

  fn fix_bad_column(&self, img: &mut PixU16, defect: &Defect) {
    let col = defect.col;
    for row in 2..img.height - 2 {
      match self.camera.cfa.color_at(row, col) {
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
  fn get_wb(&self, file: &mut RawFile, wb_offset: u64) -> std::io::Result<[f32; 4]> {
    let mut buffer = vec![0; 3 * 4];
    file.inner().seek(SeekFrom::Start(wb_offset))?;
    file.inner().read_exact(&mut buffer)?;
    Ok([LEf32(&buffer, 0), LEf32(&buffer, 4), LEf32(&buffer, 8), NAN])
  }

  /// Get lens description by analyzing TIFF tags and makernotes
  fn get_lens_description(&self) -> Result<Option<&'static LensDescription>> {
    match self.lens_model()? {
      Some(model) => {
        let resolver = LensResolver::new().with_camera(&self.camera).with_lens_keyname(Some(model));
        return Ok(resolver.resolve());
      }
      None => Ok(None),
    }
  }

  /// Decode IIQ S format
  /// Same as IIQ L, but requires a linearization curve
  fn decode_nonlinear(buffer: &[u8], strips: &[u8], width: usize, height: usize, bits: u8, dummy: bool) -> PixU16 {
    // We fake the bit count as 16, then no shift happens in decoding.
    // Then we can linearize back the data.
    let mut img = Self::decode_compressed(buffer, strips, width, height, 16, dummy);

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
    img
  }

  /// Decoder for IIQ uncompressed
  fn decode_uncompressed(buffer: &[u8], width: usize, height: usize, bits: u8, dummy: bool) -> PixU16 {
    let value_shift: u32 = MAX_BITDEPTH - (bits as u32);
    decode_threaded(
      width,
      height,
      dummy,
      &(|out: &mut [u16], row| {
        for (i, word) in out.iter_mut().enumerate() {
          *word = LEu16(buffer, row * (width * 2) + i * 2) << value_shift;
        }
      }),
    )
  }

  /// Decoder for IIQ L / S data
  pub(crate) fn decode_compressed(buffer: &[u8], strips: &[u8], width: usize, height: usize, bits: u8, dummy: bool) -> PixU16 {
    let value_shift: u32 = MAX_BITDEPTH - (bits as u32);
    let lens: [u32; 10] = [8, 7, 6, 9, 11, 10, 5, 12, 14, 13];
    decode_threaded(
      width,
      height,
      dummy,
      &(|out: &mut [u16], row| {
        let offset = LEu32(strips, row * 4) as usize;
        let mut pump = BitPumpMSB32::new(&buffer[offset..]);
        let mut pred = [0_u32; 2];
        let mut len = [0_u32; 2];
        for (col, pixout) in out.chunks_exact_mut(1).enumerate() {
          if col >= (width & 0xfffffff8) {
            len[0] = 14;
            len[1] = 14;
          } else if col & 7 == 0 {
            for i in 0..2 {
              let mut j: usize = 0;
              while j < 5 && pump.get_bits(1) == 0 {
                j += 1
              }
              if j > 0 {
                len[i] = lens[(j - 1) * 2 + pump.get_bits(1) as usize];
              }
            }
          }
          let i = len[col & 1];
          pred[col & 1] = if i == 14 {
            pump.get_bits(16)
          } else {
            pred[col & 1] + pump.get_bits(i) + 1 - (1 << (i - 1))
          };
          pixout[0] = (pred[col & 1] as u16) << value_shift;
        }
      }),
    )
  }

  /// Decoder for IIQ S v2
  /// This compression stores row pixels in clusters of 8 pixels.
  fn decode_compressed_sv2(buffer: &[u8], strips: &[u8], width: usize, height: usize, bits: u8, dummy: bool) -> PixU16 {
    // Correction shift for pixel values
    let value_shift: u32 = MAX_BITDEPTH - (bits as u32);
    // We can decode each row independently
    decode_threaded(
      width,
      height,
      dummy,
      &(|out: &mut [u16], row| {
        let offset = LEu32(strips, row * 4) as usize;
        let mut pump = BitPumpMSB32::new(&buffer[offset..]);
        let base_bits = pump.get_bits(16) & 7; // first 3 bits
        let mut bit_check: [u32; 2] = [0, 0]; // for even and odd pixels
        let mut prev_pix_value = [0, 0]; // for even and odd pixels

        let pix_sub_init = 17 - base_bits;

        for cluster in out.chunks_mut(8) {
          // When we have a full cluster with 8 possible pixels
          if cluster.len() == 8 {
            // Initialize the bit_check values for even and odd.
            for i in 0..bit_check.len() {
              let idx = pump.peek_bits(7) as usize;
              pump.consume_bits(2);
              if idx >= 32 {
                // If idx is larger then lookup array, the value
                // is calculated by re-using the previous value.
                bit_check[i] = (idx as u32 >> 5) + bit_check[i] - 2;
              } else {
                // Otherwise, just do a lookup.
                debug_assert!(idx < 32);
                bit_check[i] = SV2_BIT_INDICATOR[idx];
                pump.consume_bits(SV2_SKIP_BITS[idx] as u32);
              }
            }

            let pump_savepoint = pump; // savepoint for error recovery
            let x = pump.peek_bits(3) as usize; // 3 bits are max 7, so it's safe as array index
            pump.consume_bits(SV2_USED_CORR[x]);

            let nbits = base_bits + SV2_EXTRA_BITS[x];
            let corr = [
              bit_check[0].saturating_sub(SV2_EXTRA_BITS[x]), // even
              bit_check[1].saturating_sub(SV2_EXTRA_BITS[x]), // odd
            ];
            let diff = [
              0xFFFF_u32 >> (pix_sub_init - bit_check[0]), // even
              0xFFFF_u32 >> (pix_sub_init - bit_check[1]), // odd
            ];
            // Detector for invalid decompression result
            let mut check_val = 0;
            // decompress cluster
            for i in 0..8 {
              let value = if bit_check[i & 1] == 9 {
                // Just get the value straight out the bit pump
                pump.get_bits(14)
              } else {
                // Otherwise reconstruct the value
                let tmp = prev_pix_value[i & 1] + (pump.get_bits(nbits) << corr[i & 1]) - diff[i & 1];
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
                  pump.get_bits(14)
                } else {
                  // Otherwise reconstruct the value
                  let tmp = prev_pix_value[i & 1] as i32 + (pump.get_bits(nbits) << corr[i & 1]) as i32 - diff[i & 1] as i32;
                  clamp(tmp as i32, 0, 0x3FFF) as u32
                };
                cluster[i] = (value as u16) << value_shift;
                prev_pix_value[i & 1] = value;
              }
            }
          } else {
            // Not a full cluster, just take the full values from the bit pump
            for pix in cluster.iter_mut() {
              *pix = (pump.get_bits(14) as u16) << value_shift;
            }
          }
        }
      }),
    )
  }
}

#[allow(unused)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Ord, PartialOrd)]
enum IiqTag {
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
