// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

pub mod gamma;
pub mod matrix;
pub mod raw;
pub mod sensor;
pub mod srgb;
pub mod xyz;

use rayon::prelude::*;

pub type Result<T> = std::result::Result<T, String>;

/*
macro_rules! max {
  ($x: expr) => ($x);
  ($x: expr, $($z: expr),+) => {{
      let y = max!($($z),*);
      if $x > y {
          $x
      } else {
          y
      }
  }}
}

macro_rules! min {
  ($x: expr) => ($x);
  ($x: expr, $($z: expr),+) => {{
      let y = min!($($z),*);
      if $x < y {
          $x
      } else {
          y
      }
  }}
}
 */

/// Descriptor of a two-dimensional area
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Dim2 {
  pub w: usize,
  pub h: usize,
}

impl Dim2 {
  pub fn new(w: usize, h: usize) -> Self {
    Self { w, h }
  }
}

/// Rescale to u16 value
pub fn rescale_f32_to_u16(input: &[f32], black: u16, white: u16) -> Vec<u16> {
  if black == 0 {
    input.par_iter().map(|p| (p * white as f32) as u16).collect()
  } else {
    input.par_iter().map(|p| (p * (white - black) as f32) as u16 + black).collect()
  }
}

/// Rescale to u8 value
pub fn rescale_f32_to_u8(input: &[f32], black: u8, white: u8) -> Vec<u8> {
  if black == 0 {
    input.par_iter().map(|p| (p * white as f32) as u8).collect()
  } else {
    input.par_iter().map(|p| (p * (white - black) as f32) as u8 + black).collect()
  }
}

/// Clip a value with min/max value
pub fn clip(p: f32, min: f32, max: f32) -> f32 {
  if p > max {
    max
  } else if p < min {
    min
  } else if p.is_nan() {
    min
  } else {
    p
  }
}

/// Clip a value into the range 0.0 - 1.0
pub fn clip01(p: f32) -> f32 {
  clip(p, 0.0, 1.0)
}

/// A simple x/y point
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
  pub x: usize,
  pub y: usize,
}

impl Point {
  pub fn new(x: usize, y: usize) -> Self {
    Self { x, y }
  }
}

/// Rectangle by a point and dimension
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
  pub p: Point,
  pub d: Dim2,
}

impl Rect {
  pub fn new(p: Point, d: Dim2) -> Self {
    Self { p, d }
  }
}

/// Crop image to specific area
pub fn crop<T: Clone>(input: &[T], dim: Dim2, area: Rect) -> Vec<T> {
  let mut output = Vec::with_capacity(area.d.h * area.d.w);
  output.extend(
    input
      .chunks_exact(dim.w)
      .skip(area.p.y)
      .take(area.d.h)
      .flat_map(|row| row[area.p.x..area.p.x + area.d.w].iter())
      .cloned(),
  );
  output
}
