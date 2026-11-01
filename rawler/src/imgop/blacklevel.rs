// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Black levels of raw image data (DNG spec, chapter 5 "Mapping Raw Values to
//! Linear Reference Values")

use itertools::Itertools;
use multiversion::multiversion;
use num::integer::lcm;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
  formats::tiff::{Rational, SRational},
  imgop::{
    Dim2, Point, Rect,
    raw::{WhiteBalance, cfa_planes},
  },
  pixarray::{ImageView, ImageViewMut},
  rawimage::{CFAConfig, WhiteLevel},
};

/// Minimum number of samples of the blocks of black levels and scales that
/// repeat along the rows of an image, see [`RescaleRows`]
const BLOCK_SAMPLES: usize = 64;

/// Maximum number of masked areas of a DNG raw image
pub const MAX_MASKED_AREAS: usize = 4;

/// Check the masked areas `areas` of an image of `dim` pixels with the
/// ActiveArea `active_area`
///
/// Masked areas are non-overlapping rectangles of fully masked pixels (DNG
/// spec, MaskedAreas). Each area must not be empty, must be inside of the
/// image and outside of the ActiveArea, and must not overlap other areas. At
/// most [`MAX_MASKED_AREAS`] areas are valid. The error describes the first
/// invalid area.
pub fn check_masked_areas(areas: &[Rect], dim: Dim2, active_area: Rect) -> Result<(), String> {
  if areas.len() > MAX_MASKED_AREAS {
    return Err(format!("{} masked areas, at most {} are allowed", areas.len(), MAX_MASKED_AREAS));
  }
  let image = Rect::new(Point::zero(), dim);
  for (i, area) in areas.iter().enumerate() {
    if area.intersect(&image) != Some(*area) {
      return Err(format!("masked area {:?} is empty or exceeds the image of {}x{} pixels", area, dim.w, dim.h));
    }
    if area.intersect(&active_area).is_some() {
      return Err(format!("masked area {:?} overlaps the ActiveArea {:?}", area, active_area));
    }
    if let Some(other) = areas[..i].iter().find(|other| area.intersect(other).is_some()) {
      return Err(format!("masked areas {:?} and {:?} overlap", other, area));
    }
  }
  Ok(())
}

/// Black levels of raw image data: a repeating pattern (DNG BlackLevel and
/// BlackLevelRepeatDim) and differences for each column and row of the
/// ActiveArea (BlackLevelDeltaH and BlackLevelDeltaV)
///
/// The black level of a sample is the sum of its value in the pattern and the
/// differences of its column and row. Like the CFA pattern of a raw image, the
/// pattern starts at the top-left sample of the image data, see
/// [`shift_pattern`](Self::shift_pattern) to align it to other positions. The
/// differences start at the top-left sample of the ActiveArea.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct BlackLevel {
  /// Repeating pattern, in row-column-sample order
  pub levels: Vec<Rational>,
  /// Samples per pixel
  pub cpp: usize,
  /// Columns of the pattern
  pub width: usize,
  /// Rows of the pattern
  pub height: usize,
  /// Difference of each column of the ActiveArea (DNG BlackLevelDeltaH)
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub delta_h: Vec<SRational>,
  /// Difference of each row of the ActiveArea (DNG BlackLevelDeltaV)
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub delta_v: Vec<SRational>,
}

impl Default for BlackLevel {
  fn default() -> Self {
    Self::zero(1, 1, 1)
  }
}

impl std::fmt::Debug for BlackLevel {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let levels: Vec<f32> = self.levels.iter().map(|x| x.as_f32()).collect();
    f.write_fmt(format_args!("RepeatDim: {}:{}, cpp: {}, {:?}", self.height, self.width, self.cpp, levels))?;
    if !self.delta_h.is_empty() || !self.delta_v.is_empty() {
      f.write_fmt(format_args!(", DeltaH: {}, DeltaV: {}", self.delta_h.len(), self.delta_v.len()))?;
    }
    Ok(())
  }
}

impl BlackLevel {
  /// Pattern of `width` x `height` pixels with `cpp` samples each, without differences
  pub fn new<T>(levels: &[T], width: usize, height: usize, cpp: usize) -> Self
  where
    T: Copy + Into<Rational>,
  {
    assert_eq!(levels.len(), width * height * cpp);
    Self {
      levels: levels.iter().map(|x| (*x).into()).collect(),
      width,
      height,
      cpp,
      delta_h: Vec::new(),
      delta_v: Vec::new(),
    }
  }

  pub fn zero(width: usize, height: usize, cpp: usize) -> Self {
    if width == 0 || height == 0 || cpp == 0 {
      panic!(
        "Blacklevel::zero() must not be called with zero value arguments: width: {}, height: {}, cpp: {}",
        width, height, cpp
      );
    }
    Self::new(&vec![0_u32; cpp * width * height], width, height, cpp)
  }

  /// Black levels with the differences of each column (`delta_h`) and row
  /// (`delta_v`) of the ActiveArea
  pub fn with_deltas(self, delta_h: Vec<SRational>, delta_v: Vec<SRational>) -> Self {
    Self { delta_h, delta_v, ..self }
  }

  /// Black levels with the pattern rotated to start at position (`x`, `y`) of
  /// the current pattern, the differences are unchanged
  ///
  /// Rotating by `((width - x % width) % width, (height - y % height) % height)`
  /// reverses it.
  ///
  /// # Example
  /// ```
  /// use rawler::rawimage::BlackLevel;
  /// let black = BlackLevel::new(&[1_u32, 2, 3, 4, 5, 6], 3, 2, 1);
  /// let shifted = black.shift_pattern(1, 1);
  /// assert_eq!(shifted, BlackLevel::new(&[5_u32, 6, 4, 2, 3, 1], 3, 2, 1));
  /// assert_eq!(shifted.shift_pattern(2, 1), black);
  /// ```
  pub fn shift_pattern(&self, x: usize, y: usize) -> Self {
    let (w, h, cpp) = (self.width, self.height, self.cpp);
    let mut levels = Vec::with_capacity(self.levels.len());
    for row in 0..h {
      for col in 0..w {
        let start = (((row + y) % h) * w + (col + x) % w) * cpp;
        levels.extend_from_slice(&self.levels[start..start + cpp]);
      }
    }
    Self { levels, ..self.clone() }
  }

  /// Value of the pattern for `sample` of the pixel at `row` and `col`
  fn pattern_level(&self, row: usize, col: usize, sample: usize) -> f32 {
    self.levels[((row % self.height) * self.width + col % self.width) * self.cpp + sample].as_f32()
  }

  /// Black level of `sample` of the pixel at `row` and `col`, with the
  /// differences of the ActiveArea at `active_area`
  ///
  /// The differences of the columns apply to the columns of the ActiveArea,
  /// those of the rows to its rows.
  pub fn level(&self, row: usize, col: usize, sample: usize, active_area: Rect) -> f32 {
    let delta = |deltas: &[SRational], pos: usize, start: usize, len: usize| {
      pos
        .checked_sub(start)
        .filter(|i| *i < len)
        .and_then(|i| deltas.get(i))
        .map_or(0.0, SRational::as_f32)
    };
    self.pattern_level(row, col, sample)
      + delta(&self.delta_h, col, active_area.p.x, active_area.d.w)
      + delta(&self.delta_v, row, active_area.p.y, active_area.d.h)
  }

  /// Maximum black level of `sample` in the ActiveArea at `active_area`
  ///
  /// It is the maximum of the pattern values plus the largest differences of
  /// the columns and rows with the same position in the pattern. It sets the
  /// scale of the linear values.
  pub fn max_level(&self, sample: usize, active_area: Rect) -> f32 {
    // Largest difference for each position of the pattern, relative to the ActiveArea
    let max_deltas = |deltas: &[SRational], period: usize| {
      let mut max: Vec<Option<f32>> = vec![None; period];
      for (i, delta) in deltas.iter().enumerate() {
        let max = &mut max[i % period];
        *max = Some(max.map_or(delta.as_f32(), |max| max.max(delta.as_f32())));
      }
      max.into_iter().map(|max| max.unwrap_or(0.0)).collect_vec()
    };
    let max_h = max_deltas(&self.delta_h, self.width);
    let max_v = max_deltas(&self.delta_v, self.height);
    let (x, y) = (active_area.p.x, active_area.p.y);
    (0..self.height)
      .cartesian_product(0..self.width)
      .map(|(row, col)| self.pattern_level(y + row, x + col, sample) + max_v[row] + max_h[col])
      .fold(f32::NEG_INFINITY, f32::max)
  }

  /// All black levels and differences are zero
  pub fn is_zero(&self) -> bool {
    self.levels.iter().all(|x| x.n == 0) && self.delta_h.iter().chain(&self.delta_v).all(|x| x.n == 0)
  }

  /// Black levels measured in the masked areas `areas` of `image`
  ///
  /// The level of each sample at each position of a pattern of `width` x
  /// `height` pixels is the median of its values in the areas, so a few hot
  /// pixels don't raise it. The pattern starts at the top-left sample of the
  /// image data. `None` without areas, if an area is empty or exceeds the
  /// image, or if the areas don't cover all positions of the pattern.
  pub fn from_masked_areas<'a>(image: impl Into<ImageView<'a, u16>>, areas: &[Rect], width: usize, height: usize) -> Option<Self> {
    let image = image.into();
    let cpp = image.channels();
    if areas.is_empty() || width == 0 || height == 0 || cpp == 0 {
      return None;
    }
    if let Some(area) = areas.iter().find(|area| area.intersect(&image.rect()) != Some(**area)) {
      log::debug!(
        "Masked area {:?} is empty or exceeds the image of {}x{} pixels",
        area,
        image.width(),
        image.height()
      );
      return None;
    }
    let mut values: Vec<Vec<u16>> = vec![Vec::new(); width * height * cpp];
    for area in areas {
      for (row, samples) in image.rows().enumerate().skip(area.p.y).take(area.d.h) {
        for col in area.p.x..area.p.x + area.d.w {
          let pos = ((row % height) * width + col % width) * cpp;
          for (sample, value) in samples[col * cpp..(col + 1) * cpp].iter().enumerate() {
            values[pos + sample].push(*value);
          }
        }
      }
    }
    let levels: Option<Vec<u32>> = values
      .into_iter()
      .map(|mut values| {
        let mid = values.len() / 2;
        (!values.is_empty()).then(|| u32::from(*values.select_nth_unstable(mid).1))
      })
      .collect();
    match levels {
      Some(levels) => {
        log::debug!("Black levels of the masked areas: {:?}", levels);
        Some(Self::new(&levels, width, height, cpp))
      }
      None => {
        log::debug!("Masked areas don't cover all positions of the {}x{} pattern", width, height);
        None
      }
    }
  }
}

/// White balance of CFA data with one channel, applied with the mapping to linear values
pub type MosaicWhiteBalance<'a> = (&'a CFAConfig, &'a WhiteBalance);

/// Black levels and scales to map the rows of an image to linear reference
/// values, see [`rescale`]
///
/// The black levels of the pattern and of the columns are precomputed for each
/// row of the pattern, the differences of the rows are an offset of each image
/// row. Without differences of the columns, the tables hold a block of samples
/// that repeats along the rows, so they stay in the cache.
struct RescaleRows {
  /// Samples of a table row: a block that repeats along the image rows, or a
  /// whole image row
  len: usize,
  /// The image rows repeat the table rows with this period
  period: usize,
  /// Black level of each sample, `period` rows of `len` samples
  black: Vec<f32>,
  /// Scale of each sample, with the gain of the white balance
  scale: Vec<f32>,
  /// Difference of the black level of each image row, empty without
  /// differences of the rows
  offset: Vec<f32>,
  /// Upper limit of the linear values
  limit: f32,
}

impl RescaleRows {
  fn new(black: &BlackLevel, white: &WhiteLevel, active_area: Rect, dim: Dim2, cpp: usize, wb: Option<MosaicWhiteBalance>) -> crate::Result<Self> {
    if black.cpp != cpp && black.cpp != 1 {
      return Err(format!("Black level has {} samples per pixel, image has {}", black.cpp, cpp).into());
    }
    if white.0.len() != cpp && white.0.len() != 1 {
      return Err(format!("White level has {} values, image has {} samples per pixel", white.0.len(), cpp).into());
    }
    if !black.delta_h.is_empty() && black.delta_h.len() != active_area.d.w {
      return Err(
        format!(
          "BlackLevelDeltaH has {} values, ActiveArea is {} pixels wide",
          black.delta_h.len(),
          active_area.d.w
        )
        .into(),
      );
    }
    if !black.delta_v.is_empty() && black.delta_v.len() != active_area.d.h {
      return Err(
        format!(
          "BlackLevelDeltaV has {} values, ActiveArea is {} pixels high",
          black.delta_v.len(),
          active_area.d.h
        )
        .into(),
      );
    }
    let mosaic = match wb {
      Some((config, wb)) if cpp == 1 => Some((&config.cfa, cfa_planes(&config.cfa, &config.colors)?, wb)),
      Some(_) => return Err(format!("Mosaic data must have one channel, image has {}", cpp).into()),
      None => None,
    };
    // The linear value 1.0 is the white level minus the maximum black level of each sample
    let range: Vec<f32> = (0..cpp)
      .map(|sample| white.0[sample.min(white.0.len() - 1)] as f32 - black.max_level(sample.min(black.cpp - 1), active_area))
      .collect();
    if let Some(range) = range.iter().find(|range| !range.is_finite() || **range <= 0.0) {
      return Err(format!("White level must be above the maximum black level, the difference is {}", range).into());
    }

    let (cfa_width, cfa_height) = mosaic.as_ref().map_or((1, 1), |(cfa, _, _)| (cfa.width, cfa.height));
    let period = lcm(black.height, cfa_height);
    let len = if black.delta_h.is_empty() {
      let block = lcm(black.width, cfa_width) * cpp;
      block * BLOCK_SAMPLES.div_ceil(block)
    } else {
      dim.w * cpp
    };
    let mut black_table = Vec::with_capacity(period * len);
    let mut scale_table = Vec::with_capacity(period * len);
    for row in 0..period {
      for i in 0..len {
        let (col, sample) = (i / cpp, i % cpp);
        let delta = col
          .checked_sub(active_area.p.x)
          .and_then(|col| black.delta_h.get(col))
          .map_or(0.0, SRational::as_f32);
        black_table.push(black.pattern_level(row, col, sample.min(black.cpp - 1)) + delta);
        let gain = mosaic
          .as_ref()
          .map_or(1.0, |(cfa, planes, wb)| wb.coeffs[planes[(row % cfa.height) * cfa.width + col % cfa.width]]);
        scale_table.push(gain / range[sample]);
      }
    }
    let offset = if black.delta_v.is_empty() {
      Vec::new()
    } else {
      (0..dim.h)
        .map(|row| {
          row
            .checked_sub(active_area.p.y)
            .and_then(|row| black.delta_v.get(row))
            .map_or(0.0, SRational::as_f32)
        })
        .collect()
    };
    Ok(Self {
      len,
      period,
      black: black_table,
      scale: scale_table,
      offset,
      limit: mosaic.and_then(|(_, _, wb)| wb.clip).unwrap_or(f32::INFINITY),
    })
  }

  /// Black levels and scales of image row `row`
  fn tables(&self, row: usize) -> (&[f32], &[f32]) {
    let range = (row % self.period) * self.len..(row % self.period + 1) * self.len;
    (&self.black[range.clone()], &self.scale[range])
  }
}

/// Map the samples of the rows of `src` to `dst`, see [`rescale`]
///
/// The rows are processed in blocks of the length of the tables. `offset` is
/// the difference of the black level of the row, if any.
#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn rescale_row(src: &[u16], dst: &mut [f32], black: &[f32], offset: Option<f32>, scale: &[f32], limit: f32) {
  for (src, dst) in src.chunks(black.len()).zip(dst.chunks_mut(black.len())) {
    for (((dst, src), black), scale) in dst.iter_mut().zip(src).zip(black).zip(scale) {
      let v = match offset {
        Some(offset) => f32::from(*src) - black - offset,
        None => f32::from(*src) - black,
      };
      *dst = (v.max(0.0) * scale).min(limit);
    }
  }
}

/// Map the samples of a row in place, see [`rescale_row`]
#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn rescale_row_in_place(row: &mut [f32], black: &[f32], offset: Option<f32>, scale: &[f32], limit: f32) {
  for row in row.chunks_mut(black.len()) {
    for ((v, black), scale) in row.iter_mut().zip(black).zip(scale) {
      let x = match offset {
        Some(offset) => *v - black - offset,
        None => *v - black,
      };
      *v = (x.max(0.0) * scale).min(limit);
    }
  }
}

/// Map the integer samples of `src` to linear reference values (DNG spec,
/// chapter 5)
///
/// The black level of each sample is subtracted, values below black are 0.0.
/// The values are scaled by the white level minus the maximum black level of
/// their sample plane, see [`BlackLevel::max_level`]. The pattern of `black`
/// and the CFA of `wb` are aligned to the top-left sample of `src`,
/// `active_area` is in its coordinates and places the differences of the
/// columns and rows. With `wb`, CFA data is white balanced in the same pass.
pub fn rescale<'a>(
  src: impl Into<ImageView<'a, u16>>,
  black: &BlackLevel,
  white: &WhiteLevel,
  active_area: Rect,
  wb: Option<MosaicWhiteBalance>,
) -> crate::Result<Vec<f32>> {
  let src = src.into();
  let rows = RescaleRows::new(black, white, active_area, src.dim(), src.channels(), wb)?;
  let row_len = (src.width() * src.channels()).max(1);
  let mut dst = vec![0.0; src.as_slice().len()];
  dst
    .par_chunks_mut(row_len)
    .zip(src.as_slice().par_chunks(row_len))
    .enumerate()
    .for_each(|(row, (dst, src))| {
      let (black, scale) = rows.tables(row);
      rescale_row(src, dst, black, rows.offset.get(row).copied(), scale, rows.limit);
    });
  Ok(dst)
}

/// Map floating point samples to linear reference values in place, see [`rescale`]
pub fn rescale_in_place<'a>(
  image: impl Into<ImageViewMut<'a, f32>>,
  black: &BlackLevel,
  white: &WhiteLevel,
  active_area: Rect,
  wb: Option<MosaicWhiteBalance>,
) -> crate::Result<()> {
  let mut image = image.into();
  let rows = RescaleRows::new(black, white, active_area, image.dim(), image.channels(), wb)?;
  let row_len = (image.width() * image.channels()).max(1);
  image.as_mut_slice().par_chunks_mut(row_len).enumerate().for_each(|(row, samples)| {
    let (black, scale) = rows.tables(row);
    rescale_row_in_place(samples, black, rows.offset.get(row).copied(), scale, rows.limit);
  });
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{
    CFA,
    cfa::PlaneColor,
    imgop::{Point, raw::white_balance_mosaic},
  };

  type Result = std::result::Result<(), Box<dyn std::error::Error>>;

  const XTRANS: &str = "GGRGGBGGBGGRBRGRBGGGBGGRGGRGGBRBGBRG";

  /// Samples between 0 and 4095
  fn samples(len: usize) -> Vec<u16> {
    let mut state = 0x2545_f491_u32;
    (0..len)
      .map(|_| {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 20) as u16
      })
      .collect()
  }

  /// Linear values of `src`, computed for each sample by [`BlackLevel::level`]
  fn reference(src: &[u16], dim: Dim2, cpp: usize, black: &BlackLevel, white: &WhiteLevel, active_area: Rect) -> Vec<f32> {
    let mut out = Vec::with_capacity(src.len());
    for row in 0..dim.h {
      for col in 0..dim.w {
        for sample in 0..cpp {
          let black_sample = sample.min(black.cpp - 1);
          let range = white.0[sample.min(white.0.len() - 1)] as f32 - black.max_level(black_sample, active_area);
          let v = f32::from(src[(row * dim.w + col) * cpp + sample]) - black.level(row, col, black_sample, active_area);
          out.push(v.max(0.0) / range);
        }
      }
    }
    out
  }

  fn assert_close(a: &[f32], b: &[f32], name: &str) {
    assert_eq!(a.len(), b.len(), "{}", name);
    for (i, (a, b)) in a.iter().zip(b).enumerate() {
      assert!((a - b).abs() <= 1e-6 * b.abs().max(1.0), "{}: sample {}: {} != {}", name, i, a, b);
    }
  }

  #[test]
  fn rescale_like_the_reference() -> Result {
    let deltas = |count: usize, scale: i32| (0..count).map(|i| SRational::new((i as i32 % 7 - 3) * scale, 2)).collect_vec();
    let bayer = BlackLevel::new(&[60_u16, 64, 66, 61], 2, 2, 1);
    let xtrans = BlackLevel::new(&(0..36_u32).map(|i| 60 + i % 5).collect_vec(), 6, 6, 1);
    let rgb = BlackLevel::new(&(0..12_u32).map(|i| 50 + i).collect_vec(), 2, 2, 3);
    // ActiveArea at an odd offset, the image width is no multiple of the blocks
    let (dim, active_area) = (Dim2::new(71, 13), Rect::new(Point::new(3, 1), Dim2::new(65, 11)));
    let cases = [
      ("bayer", bayer.clone(), 1, WhiteLevel::new(vec![4095])),
      ("bayer_deltas", bayer.with_deltas(deltas(65, 3), deltas(11, 5)), 1, WhiteLevel::new(vec![4095])),
      ("xtrans", xtrans.clone(), 1, WhiteLevel::new(vec![4000])),
      ("xtrans_delta_h", xtrans.with_deltas(deltas(65, 1), Vec::new()), 1, WhiteLevel::new(vec![4000])),
      ("rgb", rgb.clone(), 3, WhiteLevel::new(vec![4095, 3900, 4000])),
      (
        "rgb_delta_v",
        rgb.with_deltas(Vec::new(), deltas(11, 2)),
        3,
        WhiteLevel::new(vec![4095, 3900, 4000]),
      ),
      ("uniform_for_rgb", BlackLevel::new(&[100_u16], 1, 1, 1), 3, WhiteLevel::new(vec![4095])),
    ];
    for (name, black, cpp, white) in cases {
      let src = samples(dim.w * dim.h * cpp);
      let expected = reference(&src, dim, cpp, &black, &white, active_area);
      let rescaled = rescale(ImageView::new(&src, dim.w, dim.h, cpp), &black, &white, active_area, None)?;
      assert_close(&rescaled, &expected, name);
      let mut in_place: Vec<f32> = src.iter().copied().map(f32::from).collect();
      rescale_in_place(ImageViewMut::new(&mut in_place, dim.w, dim.h, cpp), &black, &white, active_area, None)?;
      assert_eq!(in_place, rescaled, "{}", name);
    }
    Ok(())
  }

  #[test]
  fn rescale_with_white_balance_of_the_mosaic() -> Result {
    let (dim, active_area) = (Dim2::new(70, 12), Rect::new(Point::new(1, 1), Dim2::new(66, 10)));
    let src = samples(dim.w * dim.h);
    let white = WhiteLevel::new(vec![4095]);
    let wb = WhiteBalance {
      coeffs: [2.0, 1.0, 1.5, f32::NAN],
      clip: Some(1.0),
    };
    for (pattern, black) in [
      ("RGGB", BlackLevel::new(&[60_u16, 64, 66, 61], 2, 2, 1)),
      (XTRANS, BlackLevel::new(&(0..36_u32).map(|i| 60 + i % 5).collect_vec(), 6, 6, 1)),
    ] {
      let config = CFAConfig::new(&CFA::new(pattern), &PlaneColor::new("RGB"));
      let mut expected = rescale(ImageView::new(&src, dim.w, dim.h, 1), &black, &white, active_area, None)?;
      white_balance_mosaic(ImageViewMut::new(&mut expected, dim.w, dim.h, 1), &config.cfa, &config.colors, &wb)?;
      let balanced = rescale(ImageView::new(&src, dim.w, dim.h, 1), &black, &white, active_area, Some((&config, &wb)))?;
      assert_close(&balanced, &expected, pattern);
      assert!(balanced.iter().all(|v| (0.0..=1.0).contains(v)), "{}", pattern);
    }
    Ok(())
  }

  #[test]
  fn rescale_errors() -> Result {
    let src = [100_u16; 4 * 2 * 3];
    let rect = Rect::new(Point::zero(), Dim2::new(4, 2));
    let black = BlackLevel::new(&[60_u16, 64, 66, 61], 2, 2, 1);
    let white = WhiteLevel::new(vec![4095]);
    let view = |cpp: usize| ImageView::new(&src[..8 * cpp], 4, 2, cpp);
    // The white level must be above the maximum black level
    assert!(rescale(view(1), &black, &WhiteLevel::new(vec![66]), rect, None).is_err());
    // The differences must match the ActiveArea
    let deltas = black.clone().with_deltas(vec![SRational::new(1, 1); 3], Vec::new());
    assert!(rescale(view(1), &deltas, &white, rect, None).is_err());
    // A pattern with two samples doesn't fit three channels
    assert!(rescale(view(3), &BlackLevel::new(&[1_u16, 2], 1, 1, 2), &white, rect, None).is_err());
    // Mosaic data has one channel
    let config = CFAConfig::new(&CFA::new("RGGB"), &PlaneColor::new("RGB"));
    let wb = WhiteBalance { coeffs: [1.0; 4], clip: None };
    assert!(rescale(view(3), &black, &white, rect, Some((&config, &wb))).is_err());
    Ok(())
  }

  fn rationals(values: &[u16]) -> Vec<Rational> {
    values.iter().copied().map(Rational::from).collect()
  }

  fn srationals(values: &[i32]) -> Vec<SRational> {
    values.iter().map(|v| SRational::new(*v, 1)).collect()
  }

  #[test]
  fn shift_pattern_rotates_the_pattern() -> Result {
    let black = BlackLevel::new(&[1_u16, 2, 3, 4], 2, 2, 1).shift_pattern(1, 1);
    assert_eq!(black.levels, rationals(&[4, 3, 2, 1]));

    let black = BlackLevel::new(&[1_u16, 1, 1, 2, 2, 2, 3, 3, 3, 4, 4, 4], 2, 2, 3).shift_pattern(3, 3);
    assert_eq!(black.levels, rationals(&[4, 4, 4, 3, 3, 3, 2, 2, 2, 1, 1, 1]));
    Ok(())
  }

  #[test]
  fn shift_pattern_is_reversible_and_keeps_deltas() -> Result {
    let levels: Vec<u16> = (0..36).collect();
    let black = BlackLevel::new(&levels, 6, 6, 1).with_deltas(srationals(&[1, -2, 3]), srationals(&[4, 5]));
    for (x, y) in [(0, 0), (1, 0), (2, 5), (7, 13)] {
      let shifted = black.shift_pattern(x, y);
      assert_eq!(shifted.delta_h, black.delta_h);
      assert_eq!(shifted.delta_v, black.delta_v);
      // The shifted pattern at (0, 0) is the original one at (x, y)
      let area = Rect::new(Point::new(100, 100), Dim2::new(1, 1));
      for (row, col) in (0..6).cartesian_product(0..6) {
        assert_eq!(shifted.level(row, col, 0, area), black.level(row + y, col + x, 0, area));
      }
      assert_eq!(shifted.shift_pattern((6 - x % 6) % 6, (6 - y % 6) % 6), black, "{} {}", x, y);
    }
    Ok(())
  }

  #[test]
  fn level_adds_deltas_of_the_active_area() -> Result {
    let black = BlackLevel::new(&[10_u16, 20, 30, 40], 2, 2, 1).with_deltas(srationals(&[1, -2, 3]), srationals(&[5, -1]));
    let area = Rect::new(Point::new(1, 3), Dim2::new(3, 2));
    // Outside of the columns and rows of the ActiveArea, only the pattern
    assert_eq!(black.level(0, 0, 0, area), 10.0);
    assert_eq!(black.level(3, 0, 0, area), 30.0 + 5.0);
    assert_eq!(black.level(0, 2, 0, area), 10.0 - 2.0);
    assert_eq!(black.level(5, 4, 0, area), 30.0);
    // Inside, with the differences of the column and row
    assert_eq!(black.level(3, 1, 0, area), 40.0 + 1.0 + 5.0);
    assert_eq!(black.level(4, 3, 0, area), 20.0 + 3.0 - 1.0);
    Ok(())
  }

  #[test]
  fn max_level_of_pattern_and_deltas() -> Result {
    let black = BlackLevel::new(&[10_u16, 20, 30, 40], 2, 2, 1);
    let area = Rect::new(Point::new(1, 1), Dim2::new(4, 3));
    assert_eq!(black.max_level(0, area), 40.0);
    // Largest differences of the ActiveArea columns 0 and 2: 3, columns 1 and 3: -2,
    // rows 0 and 2: 5, row 1: -1. The ActiveArea starts at pattern position (1, 1).
    let black = black.with_deltas(srationals(&[1, -2, 3, -4]), srationals(&[5, -1, 2]));
    assert_eq!(black.max_level(0, area), (40.0 + 3.0 + 5.0_f32).max(30.0 - 2.0 + 5.0));
    // Negative differences lower the maximum
    let black = BlackLevel::new(&[10_u16, 20, 30, 40], 2, 2, 1).with_deltas(srationals(&[-1, -2]), Vec::new());
    assert_eq!(black.max_level(0, Rect::new(Point::zero(), Dim2::new(2, 2))), 40.0 - 2.0);
    // Each sample has its own maximum
    let black = BlackLevel::new(&[1_u16, 7, 3, 5, 6, 2], 2, 1, 3);
    assert_eq!([0, 1, 2].map(|s| black.max_level(s, area)), [5.0, 7.0, 3.0]);
    Ok(())
  }

  #[test]
  fn is_zero_includes_deltas() -> Result {
    assert!(BlackLevel::zero(2, 2, 1).is_zero());
    assert!(!BlackLevel::zero(2, 2, 1).with_deltas(srationals(&[0, 1]), Vec::new()).is_zero());
    assert!(!BlackLevel::new(&[0_u16, 1], 1, 1, 2).is_zero());
    Ok(())
  }

  #[test]
  fn from_masked_areas_takes_the_median() -> Result {
    let pattern = [60_u16, 64, 66, 61];
    let (w, h) = (8, 8);
    let areas = [Rect::new(Point::zero(), Dim2::new(8, 2)), Rect::new(Point::new(0, 2), Dim2::new(2, 6))];
    // Each position of the pattern has 7 values in the areas, the pixels
    // outside of them are not used
    let mut src = vec![1000_u16; w * h];
    for area in areas {
      for (row, col) in (area.p.y..area.p.y + area.d.h).cartesian_product(area.p.x..area.p.x + area.d.w) {
        src[row * w + col] = pattern[(row % 2) * 2 + col % 2];
      }
    }
    // Two hot pixels of each position don't change the median
    for (row, col) in [(0, 0), (0, 2), (0, 1), (0, 3), (1, 0), (1, 2), (1, 1), (1, 3)] {
      src[row * w + col] = 4095;
    }
    let black = BlackLevel::from_masked_areas(ImageView::new(&src, w, h, 1), &areas, 2, 2);
    assert_eq!(black, Some(BlackLevel::new(&pattern, 2, 2, 1)));
    Ok(())
  }

  #[test]
  fn from_masked_areas_aligns_the_pattern_to_the_image() -> Result {
    // X-Trans like pattern of 3x2 pixels, the area starts at an odd position
    let (w, h) = (9, 6);
    let src: Vec<u16> = (0..w * h).map(|i| ((i / w % 2) * 3 + i % w % 3) as u16 * 10).collect();
    let areas = [Rect::new(Point::new(4, 1), Dim2::new(4, 3))];
    let black = BlackLevel::from_masked_areas(ImageView::new(&src, w, h, 1), &areas, 3, 2);
    assert_eq!(black, Some(BlackLevel::new(&[0_u16, 10, 20, 30, 40, 50], 3, 2, 1)));
    Ok(())
  }

  #[test]
  fn from_masked_areas_of_each_sample() -> Result {
    // Monochrome: the upper median of an even number of values
    let src = [9_u16, 1, 4, 3, 100, 100];
    let areas = [Rect::new(Point::zero(), Dim2::new(4, 1))];
    let black = BlackLevel::from_masked_areas(ImageView::new(&src, 6, 1, 1), &areas, 1, 1);
    assert_eq!(black, Some(BlackLevel::new(&[4_u16], 1, 1, 1)));
    // Three samples per pixel, one median for each sample
    let src = [10_u16, 20, 30, 12, 22, 32, 11, 99, 31, 0, 0, 0];
    let areas = [Rect::new(Point::zero(), Dim2::new(3, 1))];
    let black = BlackLevel::from_masked_areas(ImageView::new(&src, 4, 1, 3), &areas, 1, 1);
    assert_eq!(black, Some(BlackLevel::new(&[11_u16, 22, 31], 1, 1, 3)));
    Ok(())
  }

  #[test]
  fn from_masked_areas_requires_valid_areas() -> Result {
    let src = [100_u16; 8 * 4];
    let image = ImageView::new(&src, 8, 4, 1);
    assert_eq!(BlackLevel::from_masked_areas(image, &[], 2, 2), None);
    // The area exceeds the image or is empty
    assert_eq!(
      BlackLevel::from_masked_areas(image, &[Rect::new(Point::new(6, 0), Dim2::new(3, 4))], 2, 2),
      None
    );
    assert_eq!(BlackLevel::from_masked_areas(image, &[Rect::new(Point::zero(), Dim2::new(0, 4))], 2, 2), None);
    // A single row doesn't cover the second row of the pattern
    assert_eq!(BlackLevel::from_masked_areas(image, &[Rect::new(Point::zero(), Dim2::new(8, 1))], 2, 2), None);
    Ok(())
  }

  #[test]
  fn check_masked_areas_of_the_image() {
    let (dim, active_area) = (Dim2::new(16, 16), Rect::new(Point::new(2, 2), Dim2::new(12, 12)));
    let top = Rect::new(Point::new(2, 0), Dim2::new(12, 2));
    let left = Rect::new(Point::new(0, 2), Dim2::new(2, 12));
    let corner = |x: usize, y: usize| Rect::new(Point::new(x, y), Dim2::new(1, 1));
    assert_eq!(check_masked_areas(&[], dim, active_area), Ok(()));
    assert_eq!(check_masked_areas(&[top, left], dim, active_area), Ok(()));
    assert_eq!(check_masked_areas(&[top, left, corner(0, 0), corner(15, 15)], dim, active_area), Ok(()));
    // Too many areas
    assert!(check_masked_areas(&[top, left, corner(0, 0), corner(15, 15), corner(0, 15)], dim, active_area).is_err());
    // Empty, outside of the image, overlapping the ActiveArea or each other
    assert!(check_masked_areas(&[Rect::new(Point::zero(), Dim2::new(0, 2))], dim, active_area).is_err());
    assert!(check_masked_areas(&[Rect::new(Point::new(15, 0), Dim2::new(2, 2))], dim, active_area).is_err());
    assert!(check_masked_areas(&[Rect::new(Point::new(2, 0), Dim2::new(12, 3))], dim, active_area).is_err());
    assert!(check_masked_areas(&[top, Rect::new(Point::new(0, 1), Dim2::new(3, 1))], dim, active_area).is_err());
  }

  #[test]
  fn serde_without_deltas() -> Result {
    // Black levels of snapshots without differences
    let black: BlackLevel = serde_yaml::from_str("levels: [510/1, 511/1, 511/1, 510/1]\ncpp: 1\nwidth: 2\nheight: 2\n")?;
    assert_eq!(black, BlackLevel::new(&[510_u16, 511, 511, 510], 2, 2, 1));
    assert!(!serde_yaml::to_string(&black)?.contains("delta"));
    let black = black.with_deltas(srationals(&[-1]), srationals(&[2, 3]));
    assert_eq!(serde_yaml::from_str::<BlackLevel>(&serde_yaml::to_string(&black)?)?, black);
    Ok(())
  }
}
