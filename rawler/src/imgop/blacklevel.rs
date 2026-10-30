// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Black levels of raw image data (DNG spec, chapter 5 "Mapping Raw Values to
//! Linear Reference Values")

use itertools::Itertools;
use serde::{Deserialize, Serialize};

use crate::{
  formats::tiff::{Rational, SRational},
  imgop::Rect,
};

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

  pub fn as_vec(&self) -> Vec<f32> {
    self.levels.iter().map(Rational::as_f32).collect_vec()
  }

  pub fn as_bayer_array(&self) -> [f32; 4] {
    if self.levels.len() == 4 {
      [
        self.levels[0].as_f32(),
        self.levels[1].as_f32(),
        self.levels[2].as_f32(),
        self.levels[3].as_f32(),
      ]
    } else {
      [
        self.levels[0].as_f32(),
        self.levels[0].as_f32(),
        self.levels[0].as_f32(),
        self.levels[0].as_f32(),
      ]
    }
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
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::imgop::{Dim2, Point};

  type Result = std::result::Result<(), Box<dyn std::error::Error>>;

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
