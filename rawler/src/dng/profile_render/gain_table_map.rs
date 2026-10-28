// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! ProfileGainTableMap and ProfileGainTableMap2

use crate::camera_profile::GainTableMap;

/// Gain table map for rendering
///
/// The map holds a table of gains for each point of a grid over the image.
/// The table is indexed by a weighted sum of the RGB values (MapInputWeights
/// for red, green, blue, minimum and maximum), the gain is interpolated
/// linearly between the grid points and the table entries.
#[derive(Debug, Clone, PartialEq)]
pub struct GainTable {
  /// MapPointsV, MapPointsH, MapPointsN
  points: [usize; 3],
  /// MapSpacingV, MapSpacingH, relative to the image size
  spacing: [f32; 2],
  /// MapOriginV, MapOriginH, relative to the image size
  origin: [f32; 2],
  input_weights: [f32; 5],
  gamma: f32,
  gains: Vec<f32>,
}

impl GainTable {
  pub fn new(map: &GainTableMap) -> Option<Self> {
    let points = map.points.map(|value| value as usize);
    let gains = map.gain_values();
    let valid = points.iter().all(|value| *value > 0)
      && gains.len() == points.iter().product::<usize>()
      && map.spacing.iter().all(|value| value.is_finite() && *value > 0.0)
      && map.origin.iter().all(|value| value.is_finite())
      && map.gamma.is_finite()
      && map.gamma > 0.0;
    if !valid {
      log::warn!("Invalid gain table map with points {:?} and spacing {:?}, ignoring it", map.points, map.spacing);
      return None;
    }
    Some(Self {
      points,
      spacing: map.spacing.map(|value| value as f32),
      origin: map.origin.map(|value| value as f32),
      input_weights: map.input_weights,
      gamma: map.gamma,
      gains,
    })
  }

  /// Gain for `rgb` at the image relative position `u` (horizontal) and `v` (vertical)
  pub fn gain(&self, rgb: [f32; 3], u: f32, v: f32) -> f32 {
    let [points_v, points_h, points_n] = self.points;
    // Grid position, limited to the grid
    let x = ((u - self.origin[1]) / self.spacing[1]).clamp(0.0, (points_h - 1) as f32);
    let y = ((v - self.origin[0]) / self.spacing[0]).clamp(0.0, (points_v - 1) as f32);
    let (x0, y0) = (x as usize, y as usize);
    let (x1, y1) = ((x0 + 1).min(points_h - 1), (y0 + 1).min(points_v - 1));
    let (xf, yf) = (x - x0 as f32, y - y0 as f32);

    // Table position from the weighted input
    let [r, g, b] = rgb;
    let weights = self.input_weights;
    let weight = weights[0] * r + weights[1] * g + weights[2] * b + weights[3] * r.min(g).min(b) + weights[4] * r.max(g).max(b);
    let weight = if weight.is_finite() { weight.clamp(0.0, 1.0) } else { 0.0 };
    let weight = if self.gamma != 1.0 { weight.powf(self.gamma) } else { weight };
    let scaled = weight * points_n as f32;
    let w0 = (scaled as usize).min(points_n - 1);
    let w1 = (w0 + 1).min(points_n - 1);
    let wf = scaled - w0 as f32;

    let entry = |y: usize, x: usize| {
      let index = (y * points_h + x) * points_n;
      lerp(self.gains[index + w0], self.gains[index + w1], wf)
    };
    lerp(lerp(entry(y0, x0), entry(y0, x1), xf), lerp(entry(y1, x0), entry(y1, x1), xf), yf)
  }
}

fn lerp(a: f32, b: f32, fract: f32) -> f32 {
  a + (b - a) * fract
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::camera_profile::GainData;

  fn map(points: [u32; 3], gains: Vec<f32>) -> GainTableMap {
    GainTableMap {
      points,
      spacing: [1.0 / (points[0].max(2) - 1) as f64, 1.0 / (points[1].max(2) - 1) as f64],
      origin: [0.0, 0.0],
      input_weights: [0.0, 0.0, 0.0, 0.0, 1.0],
      gamma: 1.0,
      gain_min: 1.0,
      gain_max: 1.0,
      gains: GainData::F32(gains),
    }
  }

  #[test]
  fn gain_along_table() {
    // Two table entries: gain 2.0 for dark and 1.0 for bright values
    let table = GainTable::new(&map([1, 1, 2], vec![2.0, 1.0])).expect("table");
    assert_eq!(table.gain([0.0, 0.0, 0.0], 0.5, 0.5), 2.0);
    // The table index is weight * MapPointsN
    assert_eq!(table.gain([0.25, 0.1, 0.0], 0.5, 0.5), 1.5);
    assert_eq!(table.gain([0.6, 0.1, 0.0], 0.5, 0.5), 1.0);
    // Weights are limited to 0.0 ..= 1.0
    assert_eq!(table.gain([4.0, 0.0, 0.0], 0.5, 0.5), 1.0);
  }

  #[test]
  fn gain_across_image() {
    // 2x2 grid with one table entry each: 1.0, 2.0 (top), 3.0, 4.0 (bottom)
    let table = GainTable::new(&map([2, 2, 1], vec![1.0, 2.0, 3.0, 4.0])).expect("table");
    assert_eq!(table.gain([0.5; 3], 0.0, 0.0), 1.0);
    assert_eq!(table.gain([0.5; 3], 1.0, 0.0), 2.0);
    assert_eq!(table.gain([0.5; 3], 0.5, 0.5), 2.5);
    // Positions outside of the grid use the nearest grid point
    assert_eq!(table.gain([0.5; 3], 2.0, 2.0), 4.0);
    assert_eq!(table.gain([0.5; 3], -1.0, 1.0), 3.0);
  }

  #[test]
  fn gamma_of_weight() {
    let mut gamma_map = map([1, 1, 2], vec![2.0, 1.0]);
    gamma_map.gamma = 2.0;
    let table = GainTable::new(&gamma_map).expect("table");
    // 0.5^2 = 0.25, half way to the second entry
    assert_eq!(table.gain([0.5, 0.0, 0.0], 0.5, 0.5), 1.5);
  }

  #[test]
  fn invalid_maps() {
    assert!(GainTable::new(&map([1, 1, 2], vec![1.0])).is_none());
    let mut zero_spacing = map([2, 2, 1], vec![1.0; 4]);
    zero_spacing.spacing = [0.0, 1.0];
    assert!(GainTable::new(&zero_spacing).is_none());
  }
}
