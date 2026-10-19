// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! DNG opcode FixBadPixelsList (DNG 1.3)

use serde::{Deserialize, Serialize};

use crate::{
  RawImage, RawImageData, RawlerError, Result,
  imgop::{Dim2, crop},
};

use super::{
  OpcodeStage,
  helpers::{ParamReader, bayer_is_green, put_u32, same_phase_index, stage_bounds},
};

/// Patches bad pixels and bad rectangles of a Bayer CFA image given by their position.
#[derive(Debug, Clone, Eq, PartialEq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FixBadPixelsList {
  pub version: u32,
  pub flags: u32,
  /// Color of the top-left pixel, see [`FixBadPixelsConstant`]
  pub bayer_phase: u32,
  /// Bad pixels as (row, column)
  pub points: Vec<(i32, i32)>,
  /// Bad rectangles as (top, left, bottom, right)
  pub rects: Vec<(i32, i32, i32, i32)>,
}

impl FixBadPixelsList {
  pub(super) fn parse(reader: &mut ParamReader, version: u32, flags: u32) -> Result<Self> {
    let bayer_phase = reader.u32()?;
    let point_count = reader.u32()? as usize;
    let rect_count = reader.u32()? as usize;
    reader.check_count(point_count, 8, "FixBadPixelsList")?;
    let mut points = Vec::with_capacity(point_count);
    for _ in 0..point_count {
      points.push((reader.i32()?, reader.i32()?));
    }
    reader.check_count(rect_count, 16, "FixBadPixelsList")?;
    let mut rects = Vec::with_capacity(rect_count);
    for _ in 0..rect_count {
      rects.push((reader.i32()?, reader.i32()?, reader.i32()?, reader.i32()?));
    }
    if bayer_phase > 3 {
      return Err(RawlerError::DecoderFailed(format!(
        "DNG opcode FixBadPixelsList: invalid BayerPhase {}",
        bayer_phase
      )));
    }
    Ok(Self {
      version,
      flags,
      bayer_phase,
      points,
      rects,
    })
  }

  pub(super) fn write_params(&self, buf: &mut Vec<u8>) {
    put_u32(buf, self.bayer_phase);
    put_u32(buf, self.points.len() as u32);
    put_u32(buf, self.rects.len() as u32);
    for (row, col) in &self.points {
      buf.extend_from_slice(&row.to_be_bytes());
      buf.extend_from_slice(&col.to_be_bytes());
    }
    for (top, left, bottom, right) in &self.rects {
      for value in [top, left, bottom, right] {
        buf.extend_from_slice(&value.to_be_bytes());
      }
    }
  }

  pub fn apply(&self, rawimage: &mut RawImage, stage: OpcodeStage) -> Result<()> {
    let bounds = stage_bounds(rawimage, stage);
    let (width, height, cpp) = (rawimage.width, rawimage.height, rawimage.cpp);
    let samples = match &mut rawimage.data {
      RawImageData::Integer(samples) if cpp == 1 => samples,
      _ => {
        return Err(RawlerError::DecoderFailed(format!(
          "DNG opcode FixBadPixelsList in {} requires 16 bit raw data with a single plane",
          stage
        )));
      }
    };
    if bounds.width() == 0 || bounds.height() == 0 {
      return Ok(());
    }
    // Bad pixels are fixed in place in sorted order
    let mut points: Vec<(i64, i64)> = self.points.iter().map(|&(row, col)| (row as i64, col as i64)).collect();
    points.sort_unstable();
    let mut rects: Vec<(i64, i64, i64, i64)> = self.rects.iter().map(|&(t, l, b, r)| (t as i64, l as i64, b as i64, r as i64)).collect();
    rects.sort_unstable();
    let mut fixer = BadPixelFixer {
      image: BayerImage {
        data: crop(samples, Dim2::new(width, height), bounds),
        width: bounds.width() as i64,
        height: bounds.height() as i64,
        bayer_phase: self.bayer_phase,
      },
      points,
      rects,
    };
    fixer.fix();
    let window = bounds.width();
    for (line, fixed) in samples
      .chunks_exact_mut(width)
      .skip(bounds.y())
      .take(bounds.height())
      .zip(fixer.image.data.chunks_exact(window))
    {
      line[bounds.x()..bounds.x() + window].copy_from_slice(fixed);
    }
    Ok(())
  }
}

/// Bad points closer than this to other bad pixels are clustered
const BAD_POINT_PADDING: i64 = 2;

/// Bad rectangles closer than this to other bad rectangles are clustered
const BAD_RECT_PADDING: i64 = 4;

/// Single plane Bayer image for bad pixel interpolation
struct BayerImage {
  data: Vec<u16>,
  width: i64,
  height: i64,
  bayer_phase: u32,
}

impl BayerImage {
  fn contains(&self, row: i64, col: i64) -> bool {
    (0..self.height).contains(&row) && (0..self.width).contains(&col)
  }

  /// Pixel value, positions outside of the image are replicated from pixels of the same color
  fn get(&self, row: i64, col: i64) -> i64 {
    let row = same_phase_index(row as isize, self.height as usize);
    let col = same_phase_index(col as isize, self.width as usize);
    self.data[row * self.width as usize + col] as i64
  }

  fn set(&mut self, row: i64, col: i64, value: i64) {
    self.data[(row * self.width + col) as usize] = value.clamp(0, u16::MAX as i64) as u16;
  }

  fn is_green(&self, row: i64, col: i64) -> bool {
    bayer_is_green(self.bayer_phase, row as usize, col as usize)
  }
}

/// Interpolation of FixBadPixelsList
struct BadPixelFixer {
  image: BayerImage,
  /// Sorted bad pixels as (row, column)
  points: Vec<(i64, i64)>,
  /// Sorted bad rectangles as (top, left, bottom, right)
  rects: Vec<(i64, i64, i64, i64)>,
}

impl BadPixelFixer {
  /// Fix all bad pixels, then all bad rectangles, which can use the fixed pixels
  fn fix(&mut self) {
    let (width, height) = (self.image.width, self.image.height);
    for index in 0..self.points.len() {
      let (row, col) = self.points[index];
      if !self.image.contains(row, col) {
        continue;
      }
      let inside = (BAD_POINT_PADDING..height - BAD_POINT_PADDING).contains(&row) && (BAD_POINT_PADDING..width - BAD_POINT_PADDING).contains(&col);
      if inside && self.is_point_isolated(index) {
        self.fix_isolated_pixel(row, col);
      } else {
        self.fix_clustered_pixel(row, col);
      }
    }
    for index in 0..self.rects.len() {
      let (top, left, bottom, right) = self.rects[index];
      let (t, l, b, r) = (top.max(0), left.max(0), bottom.min(height), right.min(width));
      if t >= b || l >= r {
        continue;
      }
      let isolated = self.is_rect_isolated(index);
      if isolated && right == left + 1 && left >= 2 && right <= width - 2 {
        for row in t..b {
          let value = self.line_estimate(row, l, false);
          self.image.set(row, l, value);
        }
      } else if isolated && bottom == top + 1 && top >= 2 && bottom <= height - 2 {
        for col in l..r {
          let value = self.line_estimate(t, col, true);
          self.image.set(t, col, value);
        }
      } else {
        self.fix_clustered_rect(t, l, b, r);
      }
    }
  }

  fn is_bad_point(&self, row: i64, col: i64) -> bool {
    self.points.binary_search(&(row, col)).is_ok()
  }

  fn in_bad_rect(&self, row: i64, col: i64) -> bool {
    self.rects.iter().any(|&(t, l, b, r)| (t..b).contains(&row) && (l..r).contains(&col))
  }

  /// Check if a pixel can be used for interpolation: inside of the image, not in a
  /// bad rectangle and, if `skip_points` is set, no bad pixel
  fn is_valid(&self, row: i64, col: i64, skip_points: bool) -> bool {
    self.image.contains(row, col) && !(skip_points && self.is_bad_point(row, col)) && !self.in_bad_rect(row, col)
  }

  /// No other bad pixel or bad rectangle is close to the bad pixel
  fn is_point_isolated(&self, index: usize) -> bool {
    let (row, col) = self.points[index];
    let close = |&(_, c): &(i64, i64)| (c - col).abs() <= BAD_POINT_PADDING;
    let before = self.points[..index].iter().rev().take_while(|(r, _)| *r >= row - BAD_POINT_PADDING).any(close);
    let after = self.points[index + 1..].iter().take_while(|(r, _)| *r <= row + BAD_POINT_PADDING).any(close);
    let area = (
      row - BAD_POINT_PADDING,
      col - BAD_POINT_PADDING,
      row + BAD_POINT_PADDING + 1,
      col + BAD_POINT_PADDING + 1,
    );
    !(before || after || self.rects.iter().any(|rect| rects_intersect(area, *rect)))
  }

  /// No other bad rectangle is close to the bad rectangle
  fn is_rect_isolated(&self, index: usize) -> bool {
    let (t, l, b, r) = self.rects[index];
    let area = (t - BAD_RECT_PADDING, l - BAD_RECT_PADDING, b + BAD_RECT_PADDING, r + BAD_RECT_PADDING);
    !self.rects.iter().enumerate().any(|(i, rect)| i != index && rects_intersect(area, *rect))
  }

  /// Interpolate a bad pixel without bad pixels in its 5x5 neighbourhood along the
  /// direction with the smallest gradient
  fn fix_isolated_pixel(&mut self, row: i64, col: i64) {
    // Pixel (k, j) of the 5x5 neighbourhood, the bad pixel is at (2, 2)
    let v = |k: i64, j: i64| self.image.get(row + k - 2, col + j - 2);
    let estimates = if self.image.is_green(row, col) {
      // g00 b01 g02 b03 g04
      // r10 g11 r12 g13 r14
      // g20 b21 g22 b23 g24
      // r30 g31 r32 g33 r34
      // g40 b41 g42 b43 g44
      let (b01, g02, b03) = (v(0, 1), v(0, 2), v(0, 3));
      let (r10, g11, r12, g13, r14) = (v(1, 0), v(1, 1), v(1, 2), v(1, 3), v(1, 4));
      let (g20, b21, b23, g24) = (v(2, 0), v(2, 1), v(2, 3), v(2, 4));
      let (r30, g31, r32, g33, r34) = (v(3, 0), v(3, 1), v(3, 2), v(3, 3), v(3, 4));
      let (b41, g42, b43) = (v(4, 1), v(4, 2), v(4, 3));
      [
        (
          g02 + g42,
          ad(g02, g42) + ad(g11, g31) + ad(g13, g33) + ad(b01, b21) + ad(b03, b23) + ad(b21, b41) + ad(b23, b43),
        ),
        (
          g11 + g33,
          ad(g11, g33) + ad(g02, g24) + ad(g20, g42) + ad(b01, b23) + ad(r10, r32) + ad(r12, r34) + ad(b21, b43),
        ),
        (
          g20 + g24,
          ad(g20, g24) + ad(g11, g13) + ad(g31, g33) + ad(r10, r12) + ad(r30, r32) + ad(r12, r14) + ad(r32, r34),
        ),
        (
          g13 + g31,
          ad(g13, g31) + ad(g02, g20) + ad(g24, g42) + ad(b03, b21) + ad(r14, r32) + ad(r12, r30) + ad(b23, b41),
        ),
      ]
    } else {
      // b00 g01 b02 g03 b04
      // g10 r11 g12 r13 g14
      // b20 g21 b22 g23 b24
      // g30 r31 g32 r33 g34
      // b40 g41 b42 g43 b44
      let (b00, g01, b02, g03, b04) = (v(0, 0), v(0, 1), v(0, 2), v(0, 3), v(0, 4));
      let (g10, r11, g12, r13, g14) = (v(1, 0), v(1, 1), v(1, 2), v(1, 3), v(1, 4));
      let (b20, g21, g23, b24) = (v(2, 0), v(2, 1), v(2, 3), v(2, 4));
      let (g30, r31, g32, r33, g34) = (v(3, 0), v(3, 1), v(3, 2), v(3, 3), v(3, 4));
      let (b40, g41, b42, g43, b44) = (v(4, 0), v(4, 1), v(4, 2), v(4, 3), v(4, 4));
      [
        (
          b02 + b42,
          ad(b02, b42) + ad(g12, g32) + ad(g01, g21) + ad(g21, g41) + ad(g03, g23) + ad(g23, g43) + ad(r11, r31) + ad(r13, r33),
        ),
        (
          b00 + b44,
          ad(b00, b44) + ad(r11, r33) + ad(g01, g23) + ad(g10, g32) + ad(g12, g34) + ad(g21, g43) + ad(b02, b24) + ad(b20, b42),
        ),
        (
          b20 + b24,
          ad(b20, b24) + ad(g21, g23) + ad(g10, g12) + ad(g12, g14) + ad(g30, g32) + ad(g32, g34) + ad(r11, r13) + ad(r31, r33),
        ),
        (
          b04 + b40,
          ad(b04, b40) + ad(r13, r31) + ad(g03, g21) + ad(g14, g32) + ad(g12, g30) + ad(g23, g41) + ad(b02, b20) + ad(b24, b42),
        ),
      ]
    };
    let value = select_estimate(&estimates);
    self.image.set(row, col, value);
  }

  /// Interpolate a bad pixel close to other bad pixels from the nearest pixels of the
  /// same color that can be used
  fn fix_clustered_pixel(&mut self, row: i64, col: i64) {
    const SETS: [&[(i64, i64)]; 3] = [
      &[(-1, 1), (-1, -1), (1, -1), (1, 1)],
      &[(-2, 0), (2, 0), (0, -2), (0, 2)],
      &[(-2, -2), (-2, 2), (2, -2), (2, 2)],
    ];
    if let Some(value) = self.nearest_average(row, col, &SETS, true) {
      self.image.set(row, col, value);
    }
  }

  /// Interpolate the pixels of a bad rectangle from the nearest pixels of the same
  /// color outside of bad rectangles
  fn fix_clustered_rect(&mut self, top: i64, left: i64, bottom: i64, right: i64) {
    const SETS: [&[(i64, i64)]; 8] = [
      &[(-1, 1), (-1, -1), (1, -1), (1, 1)],
      &[(-2, 0), (2, 0), (0, -2), (0, 2)],
      &[(-2, -2), (-2, 2), (2, -2), (2, 2)],
      &[(-1, -3), (-3, -1), (1, -3), (3, -1), (-1, 3), (-3, 1), (1, 3), (3, 1)],
      &[(-4, 0), (4, 0), (0, -4), (0, 4)],
      &[(-3, -3), (-3, 3), (3, -3), (3, 3)],
      &[(-2, -4), (-4, -2), (2, -4), (4, -2), (-2, 4), (-4, 2), (2, 4), (4, 2)],
      &[(-4, -4), (-4, 4), (4, -4), (4, 4)],
    ];
    for row in top..bottom {
      for col in left..right {
        if let Some(value) = self.nearest_average(row, col, &SETS, false) {
          self.image.set(row, col, value);
        }
      }
    }
  }

  /// Rounded average of the pixels of the first offset set with pixels that can be
  /// used. Sets with odd offsets reach other colors and are skipped for red and blue.
  fn nearest_average(&self, row: i64, col: i64, sets: &[&[(i64, i64)]], skip_points: bool) -> Option<i64> {
    let green = self.image.is_green(row, col);
    sets.iter().filter(|set| green || set[0].0 % 2 == 0).find_map(|set| {
      let (total, count) = set
        .iter()
        .map(|(dr, dc)| (row + dr, col + dc))
        .filter(|&(r, c)| self.is_valid(r, c, skip_points))
        .fold((0, 0), |(total, count), (r, c)| (total + self.image.get(r, c), count + 1));
      (count > 0).then(|| (total + count / 2) / count)
    })
  }

  /// Estimate for a pixel of an isolated single bad column along the direction with
  /// the smallest gradient. For a single bad row, rows and columns of the
  /// neighbourhood are swapped.
  fn line_estimate(&self, row: i64, col: i64, transposed: bool) -> i64 {
    // Pixel (k, j) of the 9x9 neighbourhood, the bad pixel is at (4, 4)
    let v = |k: i64, j: i64| {
      if transposed {
        self.image.get(row + j - 4, col + k - 4)
      } else {
        self.image.get(row + k - 4, col + j - 4)
      }
    };
    let (estimates, lower, upper) = if self.image.is_green(row, col) {
      // g00 b01 g02 b03 g04 b05 g06 b07 g08
      // r10 g11 r12 g13 r14 g15 r16 g17 r18
      // g20 b21 g22 b23 g24 b25 g26 b27 g28
      // r30 g31 r32 g33 r34 g35 r36 g37 r38
      // g40 b41 g42 b43 g44 b45 g46 b47 g48
      // r50 g51 r52 g53 r54 g55 r56 g57 r58
      // g60 b61 g62 b63 g64 b65 g66 b67 g68
      // r70 g71 r72 g73 r74 g75 r76 g77 r78
      // g80 b81 g82 b83 g84 b85 g86 b87 g88
      let (b03, b05) = (v(0, 3), v(0, 5));
      let (g11, g13, g15, g17) = (v(1, 1), v(1, 3), v(1, 5), v(1, 7));
      let (g22, b23, b25, g26) = (v(2, 2), v(2, 3), v(2, 5), v(2, 6));
      let (r30, g31, r32, g33, g35, r36, g37, r38) = (v(3, 0), v(3, 1), v(3, 2), v(3, 3), v(3, 5), v(3, 6), v(3, 7), v(3, 8));
      let (g40, g42, b43, b45, g46, g48) = (v(4, 0), v(4, 2), v(4, 3), v(4, 5), v(4, 6), v(4, 8));
      let (r50, g51, r52, g53, g55, r56, g57, r58) = (v(5, 0), v(5, 1), v(5, 2), v(5, 3), v(5, 5), v(5, 6), v(5, 7), v(5, 8));
      let (g62, b63, b65, g66) = (v(6, 2), v(6, 3), v(6, 5), v(6, 6));
      let (g71, g73, g75, g77) = (v(7, 1), v(7, 3), v(7, 5), v(7, 7));
      let (b83, b85) = (v(8, 3), v(8, 5));
      // Difference between the two types of green pixels, to adjust estimates across green types
      let split = ((g22 + g62 + g26 + g66) * 4 + (g42 + g46) * 8
        - (g11 + g13 + g15 + g17)
        - (g31 + g33 + g35 + g37) * 3
        - (g51 + g53 + g55 + g57) * 3
        - (g71 + g73 + g75 + g77)
        + 16)
        >> 5;
      let estimates = [
        (
          g13 + g75 + split * 2,
          ad(g13, g75) + ad(g15, g46) + ad(g22, g53) + ad(g35, g66) + ad(g42, g73) + ad(b03, b65) + ad(b23, b85),
        ),
        (
          g33 + g55 + split * 2,
          ad(g33, g55) + ad(g22, g55) + ad(g33, g66) + ad(g13, g35) + ad(g53, g75) + ad(b23, b45) + ad(b43, b65),
        ),
        (
          g31 + g57 + split * 2,
          ad(g31, g57) + ad(g33, g46) + ad(g35, g48) + ad(g40, g53) + ad(g42, g55) + ad(r30, r56) + ad(r32, r58),
        ),
        (
          g42 + g46,
          ad(g42, g46) * 2 + ad(g33, g35) + ad(g53, g55) + ad(b23, b25) + ad(b43, b45) + ad(b63, b65),
        ),
        (
          g37 + g51 + split * 2,
          ad(g37, g51) + ad(g35, g42) + ad(g33, g40) + ad(g48, g55) + ad(g46, g53) + ad(r38, r52) + ad(r36, r50),
        ),
        (
          g35 + g53 + split * 2,
          ad(g35, g53) + ad(g26, g53) + ad(g35, g62) + ad(g15, g33) + ad(g55, g73) + ad(b25, b43) + ad(b45, b63),
        ),
        (
          g15 + g73 + split * 2,
          ad(g15, g73) + ad(g13, g42) + ad(g26, g55) + ad(g33, g62) + ad(g46, g75) + ad(b05, b63) + ad(b25, b83),
        ),
      ];
      let lower = (g33.min(g35).min(g53).min(g55) + split).clamp(0, u16::MAX as i64);
      let upper = (g33.max(g35).max(g53).max(g55) + split).clamp(0, u16::MAX as i64);
      (estimates, lower, upper)
    } else {
      // b00 g01 b02 g03 b04 g05 b06 g07 b08
      // g10 r11 g12 r13 g14 r15 g16 r17 g18
      // b20 g21 b22 g23 b24 g25 b26 g27 b28
      // g30 r31 g32 r33 g34 r35 g36 r37 g38
      // b40 g41 b42 g43 b44 g45 b46 g47 b48
      // g50 r51 g52 r53 g54 r55 g56 r57 g58
      // b60 g61 b62 g63 b64 g65 b66 g67 b68
      // g70 r71 g72 r73 g74 r75 g76 r77 g78
      // b80 g81 b82 g83 b84 g85 b86 g87 b88
      let (b02, g03, g05, b06) = (v(0, 2), v(0, 3), v(0, 5), v(0, 6));
      let (r13, r15) = (v(1, 3), v(1, 5));
      let (b20, b22, g23, g25, b26, b28) = (v(2, 0), v(2, 2), v(2, 3), v(2, 5), v(2, 6), v(2, 8));
      let (r31, g32, r33, r35, g36, r37) = (v(3, 1), v(3, 2), v(3, 3), v(3, 5), v(3, 6), v(3, 7));
      let (g41, b42, g43, g45, b46, g47) = (v(4, 1), v(4, 2), v(4, 3), v(4, 5), v(4, 6), v(4, 7));
      let (r51, g52, r53, r55, g56, r57) = (v(5, 1), v(5, 2), v(5, 3), v(5, 5), v(5, 6), v(5, 7));
      let (b60, b62, g63, g65, b66, b68) = (v(6, 0), v(6, 2), v(6, 3), v(6, 5), v(6, 6), v(6, 8));
      let (r73, r75) = (v(7, 3), v(7, 5));
      let (b82, g83, g85, b86) = (v(8, 2), v(8, 3), v(8, 5), v(8, 6));
      let estimates = [
        (
          b02 + b86,
          ad(b02, b86) + ad(r13, r55) + ad(r33, r75) + ad(g03, g45) + ad(g23, g65) + ad(g43, g85),
        ),
        (
          b22 + b66,
          ad(b22, b66) + ad(r13, r35) + ad(r33, r55) + ad(r53, r75) + ad(g23, g45) + ad(g43, g65),
        ),
        (
          b20 + b68,
          ad(b20, b68) + ad(r31, r55) + ad(r33, r57) + ad(g23, g47) + ad(g32, g56) + ad(g41, g65),
        ),
        (b42 + b46, ad(b42, b46) + ad(r33, r35) + ad(r53, r55) + ad(g32, g36) + ad(g52, g56)),
        (
          b28 + b60,
          ad(b28, b60) + ad(r37, r53) + ad(r35, r51) + ad(g25, g41) + ad(g36, g52) + ad(g47, g63),
        ),
        (
          b26 + b62,
          ad(b26, b62) + ad(r15, r33) + ad(r35, r53) + ad(r55, r73) + ad(g25, g43) + ad(g45, g63),
        ),
        (
          b06 + b82,
          ad(b06, b82) + ad(r15, r53) + ad(r35, r73) + ad(g05, g43) + ad(g25, g63) + ad(g45, g83),
        ),
      ];
      (estimates, b42.min(b46), b42.max(b46))
    };
    select_estimate(&estimates).clamp(lower, upper)
  }
}

/// Absolute difference
fn ad(a: i64, b: i64) -> i64 {
  (a - b).abs()
}

/// Average of the estimates (each the sum of two pixels) with a gradient close to the
/// smallest gradient
fn select_estimate(estimates: &[(i64, i64)]) -> i64 {
  let min_grad = estimates.iter().map(|(_, grad)| *grad).min().unwrap_or(0);
  let limit = (min_grad * 3) >> 1;
  let (total, count) = estimates
    .iter()
    .filter(|(_, grad)| *grad <= limit)
    .fold((0, 0), |(total, count), (estimate, _)| (total + estimate, count + 2));
  let count = count.max(1);
  (total + (count >> 1)) / count
}

/// Check if two rectangles (top, left, bottom, right) overlap
fn rects_intersect(a: (i64, i64, i64, i64), b: (i64, i64, i64, i64)) -> bool {
  a.0.max(b.0) < a.2.min(b.2) && a.1.max(b.1) < a.3.min(b.3)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::dng::opcodes::{Opcode, OpcodeList, testutil::*};

  fn fix_bad_pixels_list(phase: u32, points: &[(i32, i32)], rects: &[(i32, i32, i32, i32)]) -> Vec<u8> {
    let mut buf = Vec::new();
    [phase, points.len() as u32, rects.len() as u32].iter().for_each(|v| put_u32(&mut buf, *v));
    points
      .iter()
      .for_each(|(row, col)| [row, col].iter().for_each(|v| put_u32(&mut buf, **v as u32)));
    rects
      .iter()
      .for_each(|(t, l, b, r)| [t, l, b, r].iter().for_each(|v| put_u32(&mut buf, **v as u32)));
    buf
  }

  /// Linear ramp, interpolation along symmetric pixel pairs restores pixels exactly
  fn ramp(row: i32, col: i32) -> u16 {
    (1000 + 7 * row + 3 * col) as u16
  }

  /// Rounded average of ramp pixels
  fn ramp_average(pixels: &[(i32, i32)]) -> u16 {
    let total: u32 = pixels.iter().map(|(r, c)| ramp(*r, *c) as u32).sum();
    let count = pixels.len() as u32;
    ((total + count / 2) / count) as u16
  }

  /// Ramp image with BayerPhase 0, listed pixels are marked bad (0) and fixed
  fn fix_ramp(size: usize, points: &[(i32, i32)], rects: &[(i32, i32, i32, i32)]) -> Result<Vec<u16>> {
    let mut image = float_image(size, size, 1, 0.0);
    let inside_rect = |r: i32, c: i32| rects.iter().any(|&(t, l, b, rr)| (t..b).contains(&r) && (l..rr).contains(&c));
    image.data = RawImageData::Integer(
      (0..size * size)
        .map(|i| {
          let (r, c) = ((i / size) as i32, (i % size) as i32);
          if points.contains(&(r, c)) || inside_rect(r, c) { 0 } else { ramp(r, c) }
        })
        .collect(),
    );
    OpcodeList::parse(&opcode_list(&[opcode(5, V1_3, 0, &fix_bad_pixels_list(0, points, rects))]))?.apply(&mut image, OpcodeStage::List1)?;
    Ok(integers(&image).to_vec())
  }

  #[test]
  fn fix_bad_pixels_list_parse() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Unsorted lists and negative coordinates are kept as stored
    let (points, rects) = ([(5, 4), (-1, 3), (2, 3)], [(9, 1, 10, 8), (0, 0, 2, 2)]);
    let data = opcode_list(&[opcode(5, V1_3, 0, &fix_bad_pixels_list(2, &points, &rects))]);
    let list = OpcodeList::parse(&data)?;
    assert!(matches!(&list.opcodes()[0], Opcode::FixBadPixelsList(op) if op.points == points && op.rects == rects && op.bayer_phase == 2));
    assert_eq!(list.to_bytes()?, data);
    // Invalid BayerPhase
    let params = fix_bad_pixels_list(4, &points, &rects);
    assert!(OpcodeList::parse(&opcode_list(&[opcode(5, V1_3, 0, &params)])).is_err());
    let list = OpcodeList::parse(&opcode_list(&[opcode(5, V1_3, 1, &params)]))?;
    assert!(matches!(&list.opcodes()[0], Opcode::Unknown(op) if op.id == 5));
    // Size must match the counts
    let mut params = fix_bad_pixels_list(0, &points, &rects);
    params.push(0);
    assert!(OpcodeList::parse(&opcode_list(&[opcode(5, V1_3, 0, &params)])).is_err());
    let mut params = fix_bad_pixels_list(0, &points, &rects);
    params[7] = 4;
    assert!(OpcodeList::parse(&opcode_list(&[opcode(5, V1_3, 0, &params)])).is_err());
    Ok(())
  }

  #[test]
  fn fix_bad_pixels_list_points() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Isolated red (4, 4) and green (8, 9) pixels, a pixel outside of the image is ignored
    let pixels = fix_ramp(12, &[(4, 4), (8, 9), (-1, 3)], &[])?;
    for (i, value) in pixels.iter().enumerate() {
      assert_eq!(*value, ramp((i / 12) as i32, (i % 12) as i32), "pixel {}", i);
    }
    // Clustered green pixels skip each other, red pixel at the corner uses pixels inside of the image
    let pixels = fix_ramp(12, &[(4, 5), (5, 6), (0, 0)], &[])?;
    assert_eq!(pixels[4 * 12 + 5], ramp_average(&[(3, 6), (3, 4), (5, 4)]));
    assert_eq!(pixels[5 * 12 + 6], ramp_average(&[(4, 7), (6, 5), (6, 7)]));
    assert_eq!(pixels[0], ramp_average(&[(2, 0), (0, 2)]));
    Ok(())
  }

  #[test]
  fn fix_bad_pixels_list_lines() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Isolated single column and single row, the interior is restored exactly
    let column = fix_ramp(16, &[], &[(0, 7, 16, 8)])?;
    let row = fix_ramp(16, &[], &[(7, 0, 8, 16)])?;
    for i in 0..16 {
      if (4..12).contains(&i) {
        assert_eq!(column[i * 16 + 7], ramp(i as i32, 7), "column, row {}", i);
        assert_eq!(row[7 * 16 + i], ramp(7, i as i32), "row, column {}", i);
      }
      assert!(column[i * 16 + 7] > 900 && row[7 * 16 + i] > 900);
    }
    Ok(())
  }

  #[test]
  fn fix_bad_pixels_list_clustered_rect() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Two columns wide, pixels of the rectangle are not used
    let pixels = fix_ramp(16, &[], &[(4, 6, 10, 8)])?;
    assert_eq!(pixels[6 * 16 + 6], ramp_average(&[(6, 4), (6, 8)]));
    assert_eq!(pixels[6 * 16 + 7], ramp_average(&[(5, 8), (7, 8)]));
    assert_eq!(pixels[4 * 16 + 7], ramp_average(&[(3, 8), (3, 6), (5, 8)]));
    // Bad pixels are fixed before bad rectangles, which can use them
    let pixels = fix_ramp(16, &[(6, 4)], &[(6, 6, 8, 8)])?;
    let point = ramp_average(&[(4, 4), (8, 4), (6, 2)]);
    assert_eq!(pixels[6 * 16 + 4], point);
    let total = ramp(4, 6) as u32 + ramp(8, 6) as u32 + point as u32 + ramp(6, 8) as u32;
    assert_eq!(pixels[6 * 16 + 6], ((total + 2) / 4) as u16);
    Ok(())
  }

  #[test]
  fn fix_bad_pixels_list_data_checks() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let list = OpcodeList::parse(&opcode_list(&[opcode(5, V1_3, 0, &fix_bad_pixels_list(0, &[(1, 1)], &[]))]))?;
    assert!(list.apply(&mut float_image(8, 8, 1, 0.0), OpcodeStage::List1).is_err());
    let mut image = float_image(4, 4, 3, 0.0);
    image.data = RawImageData::Integer(vec![0; 48]);
    assert!(list.apply(&mut image, OpcodeStage::List1).is_err());
    Ok(())
  }
}
