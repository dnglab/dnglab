// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

pub mod chromatic_adaption;
pub mod develop;
pub mod gamma;
pub mod matrix;
pub mod raw;
pub mod sensor;
pub mod spline;
pub mod srgb;
pub mod xyz;
pub mod yuv;

use multiversion::multiversion;
use serde::{Deserialize, Serialize};
use std::cmp::{max, min};

use crate::{formats::tiff::IFD, tags::DngTag};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Dim2 {
  pub w: usize,
  pub h: usize,
}

impl Dim2 {
  pub fn new(w: usize, h: usize) -> Self {
    Self { w, h }
  }

  pub fn is_empty(&self) -> bool {
    self.w == 0 && self.h == 0
  }
}

/// Clip a value with min/max value
#[allow(clippy::if_same_then_else)]
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

/// A simple x/y point
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Point {
  pub x: usize,
  pub y: usize,
}

impl Point {
  pub fn new(x: usize, y: usize) -> Self {
    Self { x, y }
  }

  pub fn zero() -> Self {
    Self { x: 0, y: 0 }
  }
}

/// Rectangle by a point and dimension
#[derive(Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Rect {
  pub p: Point,
  pub d: Dim2,
}

impl std::fmt::Debug for Rect {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    //f.debug_struct("Rect").field("p", &self.p).field("d", &self.d).finish()?;
    f.write_fmt(format_args!(
      "Rect{{{}:{}, {}x{}, LTRB=[{}, {}, {}, {}]}}",
      self.p.x,
      self.p.y,
      self.d.w,
      self.d.h,
      self.p.x,
      self.p.y,
      self.p.x + self.d.w,
      self.p.y + self.d.h
    ))
  }
}

impl Rect {
  pub fn new(p: Point, d: Dim2) -> Self {
    Self { p, d }
  }

  // left, top, right, bottom
  pub fn new_with_points(p1: Point, p2: Point) -> Self {
    assert!(p1.x <= p2.x);
    assert!(p1.y <= p2.y);
    Self {
      p: p1,
      d: Dim2 {
        w: p2.x - p1.x,
        h: p2.y - p1.y,
      },
    }
  }

  pub fn new_with_borders(dim: Dim2, borders: &[usize; 4]) -> Self {
    Self::new_with_points(Point::new(borders[0], borders[1]), Point::new(dim.w - borders[2], dim.h - borders[3]))
  }

  /// DNG used top-left-bottom-right for all rectangles
  pub fn new_with_dng(rect: &[usize; 4]) -> Rect {
    Self::new_with_points(Point::new(rect[1], rect[0]), Point::new(rect[3], rect[2]))
  }

  pub fn is_empty(&self) -> bool {
    self.d.is_empty()
  }

  /// Return in LTRB coordinates
  pub fn as_ltrb(&self) -> [usize; 4] {
    [self.p.x, self.p.y, self.p.x + self.d.w, self.p.y + self.d.h]
  }

  /// Return in TLBR
  pub fn as_tlbr(&self) -> [usize; 4] {
    [self.p.y, self.p.x, self.p.y + self.d.h, self.p.x + self.d.w]
  }

  /// Return as offsets from each side (LTRB)
  pub fn as_ltrb_offsets(&self, width: usize, height: usize) -> [usize; 4] {
    [self.p.x, self.p.y, width - (self.p.x + self.d.w), height - (self.p.y + self.d.h)]
  }

  /// Return as offsets from each side (TLBR)
  pub fn as_tlbr_offsets(&self, width: usize, height: usize) -> [usize; 4] {
    [self.p.y, self.p.x, height - (self.p.y + self.d.h), width - (self.p.x + self.d.w)]
  }

  // Read Crop params from IFD
  pub fn from_tiff(ifd: &IFD) -> Option<Self> {
    if let Some(crop) = ifd.get_entry(DngTag::DefaultCropOrigin) {
      if let Some(dim) = ifd.get_entry(DngTag::DefaultCropSize) {
        let p = Point::new(crop.force_usize(0), crop.force_usize(1));
        let d = Dim2::new(dim.force_usize(0), dim.force_usize(1));
        return Some(Self::new(p, d));
      }
    }
    None
  }

  pub fn intersection(&self, other: &Self) -> Self {
    let x1 = max(self.p.x, other.p.x);
    let y1 = max(self.p.y, other.p.y);
    let x2 = min(self.p.x + self.d.w, other.p.x + other.d.w);
    let y2 = min(self.p.y + self.d.h, other.p.y + other.d.h);

    if x1 >= x2 || y1 >= y2 {
      // No overlap, return an empty rectangle
      Self::new(Point::zero(), Dim2::new(0, 0))
    } else {
      Self::new_with_points(Point::new(x1, y1), Point::new(x2, y2))
    }
  }

  pub fn width(&self) -> usize {
    self.d.w
  }

  pub fn height(&self) -> usize {
    self.d.h
  }

  pub fn x(&self) -> usize {
    self.p.x
  }

  pub fn y(&self) -> usize {
    self.p.y
  }

  pub fn adapt(&self, master: &Self) -> Self {
    assert!(self.p.x >= master.p.x);
    assert!(self.p.y >= master.p.y);
    assert!(self.d.w <= master.d.w);
    assert!(self.d.h <= master.d.h);
    Self {
      p: Point::new(self.p.x - master.p.x, self.p.y - master.p.y),
      d: self.d,
    }
  }

  pub fn scale(&mut self, scale: f32) {
    self.d.w = ((self.d.w as f32) * scale) as usize;
    self.d.h = ((self.d.h as f32) * scale) as usize;
    self.p.x = ((self.p.x as f32) * scale) as usize;
    self.p.y = ((self.p.y as f32) * scale) as usize;
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

pub fn scale_u16_to_double(x: u16) -> f32 {
  x as f32 / u16::MAX as f32
}

pub fn scale_double_to_u16(x: f32) -> u16 {
  (x.abs() * u16::MAX as f32) as u16
}

pub fn scale_u8_to_double(x: u8) -> f32 {
  x as f32 / u8::MAX as f32
}

pub fn scale_double_to_u8(x: f32) -> u8 {
  (x.abs() * u8::MAX as f32) as u8
}

/*
/// Rescale to u16 value
#[multiversion]
#[clone(target = "[x86|x86_64]+avx+avx2")]
#[clone(target = "x86+sse")]
pub fn rescale_f32_to_u16(input: &[f32], black: u16, white: u16) -> Vec<u16> {
  if black == 0 {
    input.par_iter().map(|p| (p * white as f32) as u16).collect()
  } else {
    input.par_iter().map(|p| (p * (white - black) as f32) as u16 + black).collect()
  }
}

/// Rescale to u8 value
#[multiversion]
#[clone(target = "[x86|x86_64]+avx+avx2")]
#[clone(target = "x86+sse")]
pub fn rescale_f32_to_u8(input: &[f32], black: u8, white: u8) -> Vec<u8> {
  if black == 0 {
    input.par_iter().map(|p| (p * white as f32) as u8).collect()
  } else {
    input.par_iter().map(|p| (p * (white - black) as f32) as u8 + black).collect()
  }
}
 */

#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
pub fn convert_to_f32_unscaled<T>(pix: &[T]) -> Vec<f32>
where
  T: Copy,
  f32: From<T>,
{
  pix.iter().copied().map(f32::from).collect()
}

#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
pub fn convert_to_f32_scaled<T>(pix: &[T], black: T, white: T) -> Vec<f32>
where
  T: Copy + Eq + PartialEq + Default + std::ops::Sub<T, Output = T>,
  f32: From<T>,
{
  if black == T::default() {
    pix.iter().copied().map(f32::from).map(|x| x / f32::from(white)).collect()
  } else {
    pix
      .iter()
      .copied()
      .map(|x| x - black)
      .map(f32::from)
      .map(|x| x / f32::from(white - black))
      .collect()
  }
}

#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
pub fn convert_from_f32_unscaled_u16(pix: &[f32]) -> Vec<u16> {
  pix.iter().copied().map(|x| x as u16).collect()
}

/// Rescale to u8 value
#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
pub fn convert_from_f32_scaled_u16(input: &[f32], black: u16, white: u16) -> Vec<u16> {
  if black == u16::default() {
    input.iter().map(|p| (p * f32::from(white)) as u16).collect()
  } else {
    input.iter().map(|p| ((p * f32::from(white - black)) as u16) + black).collect()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn rect_from_points() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let rect1 = Rect::new(Point::new(0, 0), Dim2::new(1, 1));
    let rect2 = Rect::new_with_points(Point::new(0, 0), Point::new(1, 1));
    assert_eq!(rect1, rect2);
    Ok(())
  }
}
