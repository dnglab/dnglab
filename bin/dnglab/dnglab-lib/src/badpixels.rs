// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Bad pixel lists in the format of dcraw (`.badpixels`, `dcraw -P`)
//!
//! Each line holds the column, the row and the UNIX time of death of a bad
//! pixel. Text after `#` is a comment:
//!
//! ```text
//!  962   91 1028350000  # died between August 1 and 4, 2002
//! 1285 1067 1031000000  # don't know when this pixel died
//! ```
//!
//! Like in dcraw, the coordinates are relative to the ActiveArea, and a pixel
//! that died after the image was captured is not fixed. The pixels become a
//! DNG FixBadPixelsList in OpcodeList1, which replaces an existing one.

use std::{
  collections::BTreeSet,
  path::{Path, PathBuf},
  time::{SystemTime, UNIX_EPOCH},
};

use clap::ArgMatches;
use rawler::{
  RawImage, RawImageData,
  decoders::RawDecodeParams,
  dng::opcodes::{FixBadPixelsList, Opcode, OpcodeID, OpcodeSelector, OpcodeStage},
  imgop::{Dim2, Point, Rect},
  rawimage::RawPhotometricInterpretation,
  rawsource::RawSource,
};

use crate::{AppError, Result};

/// Version of FixBadPixelsList, DNG 1.3.0.0
const FIX_BAD_PIXELS_LIST_VERSION: u32 = 0x0103_0000;

/// Bad pixel of a bad pixel list
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BadPixel {
  pub column: u32,
  pub row: u32,
  /// UNIX time of death
  pub time: i64,
}

/// Bad pixel list in the format of dcraw
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BadPixelFile {
  pub pixels: Vec<BadPixel>,
}

impl BadPixelFile {
  /// Bad pixels of `text`. Like in dcraw, the first three values of a line
  /// are the column, the row and the time. Lines without them are skipped with
  /// a warning, empty and comment lines are skipped silently.
  pub fn parse(text: &str) -> Self {
    let mut pixels = Vec::new();
    for (number, line) in text.lines().enumerate() {
      let line = line.split('#').next().unwrap_or_default();
      let fields: Vec<&str> = line.split_whitespace().collect();
      if fields.is_empty() {
        continue;
      }
      let pixel = match fields.as_slice() {
        [column, row, time, ..] => match (column.parse(), row.parse(), time.parse()) {
          (Ok(column), Ok(row), Ok(time)) => Some(BadPixel { column, row, time }),
          _ => None,
        },
        _ => None,
      };
      match pixel {
        Some(pixel) => pixels.push(pixel),
        None => log::warn!(
          "Bad pixel list, line {}: expected column, row and time, '{}' is skipped",
          number + 1,
          line.trim()
        ),
      }
    }
    Self { pixels }
  }

  /// Read the bad pixel list of `path`
  pub fn open(path: &Path) -> std::io::Result<Self> {
    Ok(Self::parse(&String::from_utf8_lossy(&std::fs::read(path)?)))
  }
}

/// Read the bad pixel list of option `id`
pub fn bad_pixels_option(options: &ArgMatches, id: &str) -> Result<Option<BadPixelFile>> {
  options
    .get_one::<PathBuf>(id)
    .map(|path| {
      if !path.exists() {
        return Err(AppError::NotFound(path.clone()));
      }
      BadPixelFile::open(path).map_err(|err| AppError::General(format!("{}: {}", path.display(), err)))
    })
    .transpose()
}

/// FixBadPixelsList with the pixels of `file` for `rawimage`, `capture_time`
/// in seconds since the UNIX epoch
///
/// Pixels outside of the ActiveArea are skipped, like pixels that died after
/// `capture_time`. Returns `None` if `rawimage` has no Bayer CFA data, which
/// FixBadPixelsList requires.
pub fn fix_bad_pixels_list(file: &BadPixelFile, rawimage: &RawImage, capture_time: Option<i64>) -> Option<FixBadPixelsList> {
  let cfa = match &rawimage.photometric {
    RawPhotometricInterpretation::Cfa(config) if config.cfa.width == 2 && config.cfa.height == 2 && config.cfa.is_rgb() => &config.cfa,
    _ => {
      log::warn!("Bad pixel list is ignored: it needs Bayer CFA raw data");
      return None;
    }
  };
  if rawimage.cpp != 1 || !matches!(rawimage.data, RawImageData::Integer(_)) {
    log::warn!("Bad pixel list is ignored: it needs raw data with integer samples");
    return None;
  }
  let area = rawimage
    .active_area
    .unwrap_or_else(|| Rect::new(Point::zero(), Dim2::new(rawimage.width, rawimage.height)));
  let (mut outside, mut later) = (0, 0);
  let mut points = BTreeSet::new();
  for pixel in &file.pixels {
    if pixel.column as usize >= area.d.w || pixel.row as usize >= area.d.h {
      outside += 1;
    } else if capture_time.is_some_and(|time| pixel.time > time) {
      later += 1;
    } else {
      // FixBadPixelsList in OpcodeList1 refers to the whole raw image
      points.insert(((area.p.y + pixel.row as usize) as i32, (area.p.x + pixel.column as usize) as i32));
    }
  }
  log::info!(
    "Bad pixel list: {} pixels are fixed, {} are outside of the ActiveArea, {} died after the capture",
    points.len(),
    outside,
    later
  );
  // Color of the top-left pixel: red, green next to red, green next to blue or blue
  let bayer_phase = match (cfa.color_at(0, 0), cfa.color_at(0, 1)) {
    (0, _) => 0,
    (1, 0) => 1,
    (1, _) => 2,
    _ => 3,
  };
  Some(FixBadPixelsList {
    version: FIX_BAD_PIXELS_LIST_VERSION,
    flags: 0,
    bayer_phase,
    points: points.into_iter().collect(),
    rects: Vec::new(),
  })
}

/// Add the FixBadPixelsList of `file` for image `index` of `input` to
/// `replace`, the DNG opcodes to insert. Without a bad pixel that applies, an
/// existing FixBadPixelsList is removed by a selector in `remove`. A raw image
/// that can't be read is logged and gets no bad pixel list.
pub fn add_bad_pixel_opcodes(file: &BadPixelFile, input: &Path, index: usize, replace: &mut Vec<(OpcodeStage, Opcode)>, remove: &mut Vec<OpcodeSelector>) {
  match bad_pixels_of_image(file, input, index) {
    Ok(Some(list)) if !list.points.is_empty() => replace.push((OpcodeStage::List1, Opcode::FixBadPixelsList(list))),
    Ok(Some(_)) => remove.push(OpcodeSelector {
      stage: Some(OpcodeStage::List1),
      id: OpcodeID::FixBadPixelsList as u32,
    }),
    Ok(None) => {}
    Err(err) => log::warn!("Bad pixel list is ignored for {}: {}", input.display(), err),
  }
}

/// FixBadPixelsList of `file` for image `index` of `input`, from the
/// metadata of the image without its pixels
fn bad_pixels_of_image(file: &BadPixelFile, input: &Path, index: usize) -> Result<Option<FixBadPixelsList>> {
  let rawfile = RawSource::new(input)?;
  let decoder = rawler::get_decoder(&rawfile)?;
  let params = RawDecodeParams { image_index: index };
  let rawimage = decoder.raw_image(&rawfile, &params, true)?;
  let capture_time = decoder.raw_metadata(&rawfile, &params)?.capture_time()?.map(unix_time);
  Ok(fix_bad_pixels_list(file, &rawimage, capture_time))
}

/// Seconds of `time` since the UNIX epoch
fn unix_time(time: SystemTime) -> i64 {
  match time.duration_since(UNIX_EPOCH) {
    Ok(duration) => duration.as_secs() as i64,
    Err(err) => -(err.duration().as_secs() as i64),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use rawler::{
    CFA,
    decoders::Camera,
    pixarray::PixU16,
    rawimage::{BlackLevel, CFAConfig, WhiteLevel},
  };

  /// 16x12 raw image with the CFA `pattern` and an ActiveArea at (4, 2)
  fn rawimage(pattern: &str) -> RawImage {
    let mut camera = Camera::new();
    camera.cfa = CFA::new(pattern);
    let photometric = RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&camera));
    let mut rawimage = RawImage::new(
      camera,
      PixU16::new_with(vec![0; 16 * 12], 16, 12),
      1,
      [2.0, 1.0, 1.5, f32::NAN],
      photometric,
      Some(BlackLevel::zero(2, 2, 1)),
      Some(WhiteLevel::new_bits(12, 1)),
      false,
    );
    rawimage.active_area = Some(Rect::new(Point::new(4, 2), Dim2::new(10, 8)));
    rawimage
  }

  #[test]
  fn parse_like_dcraw() {
    let file = BadPixelFile::parse("# camera A\n 962   91 1028350000  # died in August 2002\n\n1285\t1067 1031000000 extra\n-3 4 0\n12 x 0\n5 6\n7 8 0");
    assert_eq!(
      file.pixels,
      [
        BadPixel {
          column: 962,
          row: 91,
          time: 1028350000
        },
        BadPixel {
          column: 1285,
          row: 1067,
          time: 1031000000
        },
        BadPixel { column: 7, row: 8, time: 0 },
      ]
    );
  }

  #[test]
  fn opcode_of_bad_pixels() {
    let file = BadPixelFile::parse("0 0 0\n9 7 100\n3 1 0\n3 1 0\n10 0 0\n0 8 0\n2 2 2000");
    for (pattern, phase) in [("RGGB", 0), ("GRBG", 1), ("GBRG", 2), ("BGGR", 3)] {
      let list = fix_bad_pixels_list(&file, &rawimage(pattern), Some(1000)).expect("list");
      assert_eq!(list.bayer_phase, phase, "{}", pattern);
      // Shifted by the ActiveArea, sorted and without duplicates. Pixels outside
      // of the ActiveArea and the pixel that died after the capture are skipped.
      assert_eq!(list.points, [(2, 4), (3, 7), (9, 13)], "{}", pattern);
      assert_eq!((list.version, list.flags), (FIX_BAD_PIXELS_LIST_VERSION, 0));
    }
    // Without a capture time, all pixels in the ActiveArea are fixed
    let list = fix_bad_pixels_list(&file, &rawimage("RGGB"), None).expect("list");
    assert_eq!(list.points, [(2, 4), (3, 7), (4, 6), (9, 13)]);
  }

  #[test]
  fn opcode_needs_bayer_cfa_data() {
    let file = BadPixelFile::parse("1 1 0");
    let mut float = rawimage("RGGB");
    float.data = RawImageData::Float(vec![0.0; 16 * 12]);
    assert!(fix_bad_pixels_list(&file, &float, None).is_none());
    let xtrans = rawimage("GGRGGBGGBGGRBRGRBGGGBGGRGGRGGBRBGBRG");
    assert!(fix_bad_pixels_list(&file, &xtrans, None).is_none());
  }
}
