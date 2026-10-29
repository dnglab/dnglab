// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Helpers shared by the DNG opcodes: reading and writing of opcode parameters,
//! image areas and planes, Bayer phases, warp geometry and resampling, and the
//! per-row and per-column tables of DeltaPerRow, DeltaPerColumn, ScalePerRow
//! and ScalePerColumn.

use std::ops::Range;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
  RawlerError, Result,
  imgop::{Dim2, Point, Rect, crop},
  pixarray::SubPixel,
};

use super::{OpcodeData, OpcodeImage, OpcodeStage};

/// Reader for big-endian opcode data
pub(super) struct ParamReader<'a> {
  data: &'a [u8],
  pos: usize,
}

impl<'a> ParamReader<'a> {
  pub(super) fn new(data: &'a [u8]) -> Self {
    Self { data, pos: 0 }
  }

  pub(super) fn remaining(&self) -> usize {
    self.data.len() - self.pos
  }

  pub(super) fn take(&mut self, n: usize) -> Result<&'a [u8]> {
    if n > self.remaining() {
      return Err(RawlerError::DecoderFailed(format!(
        "DNG opcode data truncated: {} bytes required at offset {}, but only {} available",
        n,
        self.pos,
        self.remaining()
      )));
    }
    let bytes = &self.data[self.pos..self.pos + n];
    self.pos += n;
    Ok(bytes)
  }

  fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
    let mut buf = [0; N];
    buf.copy_from_slice(self.take(N)?);
    Ok(buf)
  }

  pub(super) fn u16(&mut self) -> Result<u16> {
    self.array().map(u16::from_be_bytes)
  }

  pub(super) fn u32(&mut self) -> Result<u32> {
    self.array().map(u32::from_be_bytes)
  }

  pub(super) fn i32(&mut self) -> Result<i32> {
    self.array().map(i32::from_be_bytes)
  }

  pub(super) fn f32(&mut self) -> Result<f32> {
    self.array().map(f32::from_be_bytes)
  }

  pub(super) fn f64(&mut self) -> Result<f64> {
    self.array().map(f64::from_be_bytes)
  }

  /// Check that `count` items of `item_size` bytes are available, before allocating them
  pub(super) fn check_count(&self, count: usize, item_size: usize, what: &str) -> Result<()> {
    match count.checked_mul(item_size) {
      Some(bytes) if bytes <= self.remaining() => Ok(()),
      _ => Err(RawlerError::DecoderFailed(format!(
        "DNG opcode {}: {} entries exceed parameter area of {} bytes",
        what,
        count,
        self.remaining()
      ))),
    }
  }
}

pub(super) fn put_u32(buf: &mut Vec<u8>, value: u32) {
  buf.extend_from_slice(&value.to_be_bytes());
}

pub(super) fn put_f32(buf: &mut Vec<u8>, value: f32) {
  buf.extend_from_slice(&value.to_be_bytes());
}

pub(super) fn put_f64(buf: &mut Vec<u8>, value: f64) {
  buf.extend_from_slice(&value.to_be_bytes());
}

/// Image area and planes affected by an opcode
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AreaSpec {
  pub top: u32,
  pub left: u32,
  pub bottom: u32,
  pub right: u32,
  pub plane: u32,
  pub planes: u32,
  pub row_pitch: u32,
  pub col_pitch: u32,
}

impl AreaSpec {
  pub(super) fn parse(reader: &mut ParamReader) -> Result<Self> {
    let area = Self {
      top: reader.u32()?,
      left: reader.u32()?,
      bottom: reader.u32()?,
      right: reader.u32()?,
      plane: reader.u32()?,
      planes: reader.u32()?,
      row_pitch: reader.u32()?,
      col_pitch: reader.u32()?,
    };
    area.validate()?;
    Ok(area)
  }

  /// Validate the area
  fn validate(&self) -> Result<()> {
    let invalid = |what: &str| Err(RawlerError::DecoderFailed(format!("DNG opcode area {:?} is invalid: {}", self, what)));
    if self.planes < 1 {
      return invalid("planes must be at least 1");
    }
    if self.row_pitch < 1 || self.col_pitch < 1 {
      return invalid("row and column pitch must be at least 1");
    }
    if self.is_empty() {
      if self.row_pitch != 1 || self.col_pitch != 1 {
        return invalid("row and column pitch must be 1 for an empty area");
      }
    } else if self.row_pitch > self.bottom - self.top || self.col_pitch > self.right - self.left {
      return invalid("pitch exceeds area size");
    }
    Ok(())
  }

  pub(super) fn write(&self, buf: &mut Vec<u8>) {
    for value in [
      self.top,
      self.left,
      self.bottom,
      self.right,
      self.plane,
      self.planes,
      self.row_pitch,
      self.col_pitch,
    ] {
      put_u32(buf, value);
    }
  }

  /// An empty area covers the whole image
  pub fn is_empty(&self) -> bool {
    self.top >= self.bottom || self.left >= self.right
  }

  /// Affected rectangle within an image of dimension `dim`, or `None` if
  /// no pixel is affected. The rectangle always starts at the area's top/left,
  /// so row and column pitch can be applied relative to its origin.
  pub fn overlap(&self, dim: Dim2) -> Option<Rect> {
    let rect = if self.is_empty() {
      Rect::new(Point::zero(), dim)
    } else {
      let (top, left) = (self.top as usize, self.left as usize);
      let bottom = (self.bottom as usize).min(dim.h);
      let right = (self.right as usize).min(dim.w);
      if top >= bottom || left >= right {
        return None;
      }
      Rect::new_with_points(Point::new(left, top), Point::new(right, bottom))
    };
    (rect.width() > 0 && rect.height() > 0).then_some(rect)
  }

  /// Affected planes, clipped to the planes of the image
  pub fn plane_range(&self, cpp: usize) -> Range<usize> {
    let start = (self.plane as usize).min(cpp);
    let end = (self.plane as usize).saturating_add(self.planes as usize).min(cpp);
    start..end
  }

  /// Affected rectangle in coordinates of the image buffer for the given stage
  pub(super) fn buffer_rect(&self, image: &OpcodeImage<'_>, stage: OpcodeStage) -> Option<Rect> {
    let bounds = stage_bounds(image, stage);
    self
      .overlap(bounds.d)
      .map(|rect| Rect::new(Point::new(bounds.p.x + rect.p.x, bounds.p.y + rect.p.y), rect.d))
  }
}

/// Part of the image buffer an opcode list operates on.
/// OpcodeList1 operates on the raw image as stored in the file. The stage 2 and
/// stage 3 images are cropped to the ActiveArea, so opcode
/// coordinates of OpcodeList2 and OpcodeList3 are relative to the ActiveArea.
pub(super) fn stage_bounds(image: &OpcodeImage<'_>, stage: OpcodeStage) -> Rect {
  let full = Rect::new(Point::zero(), Dim2::new(image.width(), image.height()));
  match stage {
    OpcodeStage::List1 => full,
    OpcodeStage::List2 | OpcodeStage::List3 => image.active_area.unwrap_or(full),
  }
}

/// Map the planes of all pixels in `rect` (stepping by the row and column pitch
/// of `area`) through function `f`.
pub(super) fn map_area<T: SubPixel, F>(samples: &mut [T], width: usize, cpp: usize, rect: Rect, area: &AreaSpec, f: &F)
where
  F: Fn(T) -> T + Sync,
{
  let planes = area.plane_range(cpp);
  let col_pitch = area.col_pitch as usize;
  apply_roi_threaded(samples, width, cpp, rect, area.row_pitch as usize, &|line, _row| {
    for pixel in line.chunks_exact_mut(cpp).step_by(col_pitch) {
      for c in planes.clone() {
        pixel[c] = f(pixel[c]);
      }
    }
  });
}

/// Call `closure` for every `row_pitch` row of `roi` with the pixels of that
/// row inside `roi` and the absolute row index.
pub(super) fn apply_roi_threaded<T: SubPixel, F>(samples: &mut [T], width: usize, cpp: usize, roi: Rect, row_pitch: usize, closure: &F)
where
  F: Fn(&mut [T], usize) + Sync,
{
  let col_start = roi.x() * cpp;
  let col_end = col_start + roi.width() * cpp;
  samples
    .par_chunks_exact_mut(width * cpp)
    .skip(roi.y())
    .take(roi.height())
    .step_by(row_pitch)
    .enumerate()
    .for_each(|(i, line)| closure(&mut line[col_start..col_end], roi.y() + i * row_pitch));
}

/// Check for a green pixel of a Bayer image. BayerPhase is the color of the top-left
/// pixel as in the DNG specification.
pub(super) fn bayer_is_green(bayer_phase: u32, row: usize, col: usize) -> bool {
  let top_left_green = matches!(bayer_phase, 1 | 2);
  (row + col).is_multiple_of(2) == top_left_green
}

/// Index of a pixel on an axis of `size` pixels. Positions outside of the image are
/// replicated from the pixels of the same color phase at the image edge.
pub(super) fn same_phase_index(index: isize, size: usize) -> usize {
  let size = size as isize;
  let index = if index < 0 {
    index.rem_euclid(2)
  } else if index >= size {
    size - 2 + (index - size).rem_euclid(2)
  } else {
    index
  };
  index.clamp(0, size - 1) as usize
}

/// Maximum number of WarpRectilinear coefficient sets
pub(super) const WARP_MAX_PLANES: usize = 4;

/// Number of radial polynomial terms of WarpRectilinear2
pub(super) const WARP_RADIAL_TERMS: usize = 15;

/// Warp of a plane, the common model of WarpRectilinear, WarpRectilinear2 and
/// WarpFisheye
pub(super) struct WarpPlane {
  pub(super) radial: RadialModel,
  /// Tangential coefficients kt0 and kt1
  pub(super) tangential: [f64; 2],
}

/// Radial warp model
pub(super) enum RadialModel {
  /// Polynomial in r
  Polynomial {
    /// Coefficients for r^0 to r^14
    coeffs: [f64; WARP_RADIAL_TERMS],
    /// Valid radius range of the polynomial
    valid_radius: [f64; 2],
    /// Divide by the polynomial instead of multiplying
    reciprocal: bool,
  },
  /// Polynomial in arctan(r)
  Fisheye {
    /// Coefficients for t, t^3, t^5 and t^7 with t = arctan(r)
    coeffs: [f64; 4],
  },
}

impl WarpPlane {
  /// Warp without radial and tangential distortion
  fn is_identity(&self) -> bool {
    match &self.radial {
      RadialModel::Polynomial { coeffs, .. } => coeffs[0] == 1.0 && coeffs[1..].iter().all(|k| *k == 0.0) && self.tangential == [0.0, 0.0],
      // A fisheye warp is never an identity
      RadialModel::Fisheye { .. } => false,
    }
  }

  /// Ratio of real to ideal radius for the squared ideal radius `rr`
  pub(super) fn ratio(&self, rr: f64) -> f64 {
    match &self.radial {
      RadialModel::Polynomial {
        coeffs,
        valid_radius: [min, max],
        reciprocal,
      } => {
        let r = rr.clamp(min * min, max * max).sqrt();
        let poly = coeffs.iter().rev().fold(0.0, |acc, k| acc * r + k);
        if *reciprocal { 1.0 / poly } else { poly }
      }
      RadialModel::Fisheye { coeffs: [k0, k1, k2, k3] } => {
        if rr < 1.0e-12 {
          return 1.0;
        }
        let r = rr.sqrt();
        let t = r.atan();
        let t2 = t * t;
        t * (k0 + t2 * (k1 + t2 * (k2 + t2 * k3))) / r
      }
    }
  }
}

/// Apply a warp
pub(super) fn apply_warp(name: &str, planes: &[WarpPlane], center: (f64, f64), image: &mut OpcodeImage<'_>, stage: OpcodeStage) -> Result<()> {
  if planes.iter().all(WarpPlane::is_identity) {
    return Ok(());
  }
  let cpp = image.channels();
  if planes.len() != 1 && planes.len() != cpp {
    return Err(RawlerError::DecoderFailed(format!(
      "DNG opcode {}: {} coefficient sets for an image with {} planes",
      name,
      planes.len(),
      cpp
    )));
  }
  let bounds = stage_bounds(image, stage);
  if bounds.width() == 0 || bounds.height() == 0 {
    return Ok(());
  }
  let geometry = WarpGeometry::new(center, bounds.d, image.aspect_ratio);
  // The image being warped is copied, the warped image is written in place
  let width = image.width();
  let dim = Dim2::new(width * cpp, image.height());
  let window = Rect::new(Point::new(bounds.x() * cpp, bounds.y()), Dim2::new(bounds.width() * cpp, bounds.height()));
  match (&mut image.data, stage) {
    // Logical range of floating point data is 0.0 to 1.0
    (OpcodeData::Float(view), _) => {
      let samples = view.as_mut_slice();
      let src = crop(samples, dim, window);
      warp(samples, width, cpp, bounds, &src, &geometry, planes, &|v: f32| v.clamp(0.0, 1.0));
    }
    // Logical range of 16 bit data in OpcodeList1 is 0 to 65535
    (OpcodeData::Integer(view), OpcodeStage::List1) => {
      let samples = view.as_mut_slice();
      let src: Vec<f32> = crop(samples, dim, window).into_iter().map(|v| v as f32 / 65535.0).collect();
      warp(samples, width, cpp, bounds, &src, &geometry, planes, &|v: f32| {
        (v * 65535.0).round().clamp(0.0, 65535.0) as u16
      });
    }
    (OpcodeData::Integer(_), _) => {
      return Err(RawlerError::DecoderFailed(format!(
        "DNG opcode {} in {} requires image data mapped to linear reference values",
        name, stage
      )));
    }
  }
  Ok(())
}

/// Warp the `bounds` window of `samples`. `src` holds the samples of the window
/// before warping, `store` converts the resampled values into the sample type.
#[allow(clippy::too_many_arguments)]
fn warp<T: SubPixel, F>(samples: &mut [T], width: usize, cpp: usize, bounds: Rect, src: &[f32], geometry: &WarpGeometry, planes: &[WarpPlane], store: &F)
where
  F: Fn(f32) -> T + Sync,
{
  let dim = bounds.d;
  apply_roi_threaded(samples, width, cpp, bounds, 1, &|line, row| {
    let y = (row - bounds.y()) as f64;
    for (x, pixel) in line.chunks_exact_mut(cpp).enumerate() {
      if let [plane_warp] = planes {
        // Same source position for all planes
        let (sx, sy) = geometry.source(plane_warp, x as f64, y);
        let taps = BicubicTaps::new(sx, sy, dim);
        for (plane, value) in pixel.iter_mut().enumerate() {
          *value = store(taps.sample(src, dim.w, cpp, plane));
        }
      } else {
        for (plane, (value, plane_warp)) in pixel.iter_mut().zip(planes).enumerate() {
          let (sx, sy) = geometry.source(plane_warp, x as f64, y);
          *value = store(BicubicTaps::new(sx, sy, dim).sample(src, dim.w, cpp, plane));
        }
      }
    }
  });
}

/// Geometry of a rectilinear warp for an image. Pixel positions are integer pixel
/// coordinates, the optical center is relative to the outer edges of the image.
pub(super) struct WarpGeometry {
  /// Optical center in pixel coordinates
  pub(super) center: (f64, f64),
  /// Distance from the optical center to the farthest image corner, for square pixels
  pub(super) radius: f64,
  /// Pixel aspect ratio
  aspect: f64,
}

impl WarpGeometry {
  /// Geometry for the optical center `center` (relative to the image size) of an image of `dim`
  pub(super) fn new(center: (f64, f64), dim: Dim2, aspect: f64) -> Self {
    let (w, h) = (dim.w as f64, dim.h as f64);
    let (c_x, c_y) = center;
    let center = (c_x * w, c_y * h);
    let square_h = (h / aspect).round();
    let square_cy = c_y * square_h;
    let radius = center.0.max(w - center.0).hypot(square_cy.max(square_h - square_cy));
    Self { center, radius, aspect }
  }

  /// Position in the unwarped image for pixel (x, y) of the warped image
  pub(super) fn source(&self, plane: &WarpPlane, x: f64, y: f64) -> (f64, f64) {
    let nx = (x - self.center.0) / self.radius;
    let ny = (y - self.center.1) / self.radius;
    // Radial and tangential warp are evaluated for square pixels
    let sy = ny / self.aspect;
    let rr = (nx * nx + sy * sy).min(1.0);
    let ratio = plane.ratio(rr);
    let [k_t0, k_t1] = plane.tangential;
    let tan_x = k_t1 * (rr + 2.0 * nx * nx) + 2.0 * k_t0 * nx * sy;
    let tan_y = k_t0 * (rr + 2.0 * sy * sy) + 2.0 * k_t1 * nx * sy;
    (
      self.center.0 + self.radius * (nx * ratio + tan_x),
      self.center.1 + self.radius * (ny * ratio + tan_y * self.aspect),
    )
  }
}

/// Keys cubic convolution kernel with A = -0.75
fn cubic_kernel(x: f64) -> f64 {
  const A: f64 = -0.75;
  let x = x.abs();
  if x >= 2.0 {
    0.0
  } else if x >= 1.0 {
    ((A * x - 5.0 * A) * x + 8.0 * A) * x - 4.0 * A
  } else {
    ((A + 2.0) * x - (A + 3.0)) * x * x + 1.0
  }
}

/// Source pixels and weights of a bicubic sample
struct BicubicTaps {
  cols: [usize; 4],
  rows: [usize; 4],
  col_weights: [f32; 4],
  row_weights: [f32; 4],
}

impl BicubicTaps {
  /// Taps for position (x, y) of an image of dimension `dim`. The position is
  /// limited to the image, taps outside of the image are replicated from its edges.
  fn new(x: f64, y: f64, dim: Dim2) -> Self {
    let (cols, col_weights) = Self::axis(x, dim.w);
    let (rows, row_weights) = Self::axis(y, dim.h);
    Self {
      cols,
      rows,
      col_weights,
      row_weights,
    }
  }

  fn axis(pos: f64, size: usize) -> ([usize; 4], [f32; 4]) {
    let last = size - 1;
    // A NaN position (e.g. a pole of a reciprocal radial model) is mapped to the image edge
    let pos = if pos.is_nan() { 0.0 } else { pos.clamp(0.0, last as f64) };
    let base = pos.floor();
    let fract = pos - base;
    let base = base as usize;
    // Taps at -1, 0, +1, +2 relative to the base pixel
    let indices = [0, 1, 2, 3].map(|i| (base + i).saturating_sub(1).min(last));
    let weights = [0, 1, 2, 3].map(|i| cubic_kernel(i as f64 - 1.0 - fract) as f32);
    (indices, weights)
  }

  /// Resampled value of a plane of `src` (interleaved samples, `width` pixels per row)
  fn sample(&self, src: &[f32], width: usize, cpp: usize, plane: usize) -> f32 {
    let mut sum = 0.0;
    for (row, row_weight) in self.rows.iter().zip(self.row_weights) {
      let line = &src[row * width * cpp..];
      let mut acc = 0.0;
      for (col, col_weight) in self.cols.iter().zip(self.col_weights) {
        acc += col_weight * line[col * cpp + plane];
      }
      sum += row_weight * acc;
    }
    sum
  }
}

/// Image axis the table values of DeltaPerRow, DeltaPerColumn, ScalePerRow
/// and ScalePerColumn belong to
#[derive(Debug, Clone, Copy)]
pub(super) enum TableAxis {
  Row,
  Column,
}

impl TableAxis {
  /// Number of table values for `area`, one for each affected row or column
  fn count(self, area: &AreaSpec) -> u32 {
    match self {
      Self::Row => area.bottom.saturating_sub(area.top).div_ceil(area.row_pitch),
      Self::Column => area.right.saturating_sub(area.left).div_ceil(area.col_pitch),
    }
  }
}

/// Parse the table of DeltaPerRow, DeltaPerColumn, ScalePerRow and ScalePerColumn
pub(super) fn parse_line_table(reader: &mut ParamReader, area: &AreaSpec, axis: TableAxis, name: &str) -> Result<Vec<f32>> {
  let count = reader.u32()?;
  let expected = axis.count(area);
  if count != expected {
    return Err(RawlerError::DecoderFailed(format!(
      "DNG opcode {}: count {} doesn't match the {} affected {}",
      name,
      count,
      expected,
      match axis {
        TableAxis::Row => "rows",
        TableAxis::Column => "columns",
      }
    )));
  }
  reader.check_count(count as usize, 4, name)?;
  (0..count)
    .map(|_| {
      let value = reader.f32()?;
      if !value.is_finite() {
        return Err(RawlerError::DecoderFailed(format!("DNG opcode {}: value {} is not finite", name, value)));
      }
      Ok(value)
    })
    .collect()
}

pub(super) fn write_line_table(buf: &mut Vec<u8>, area: &AreaSpec, values: &[f32]) {
  area.write(buf);
  put_u32(buf, values.len() as u32);
  for value in values {
    put_f32(buf, *value);
  }
}

/// Per-row or per-column table of DeltaPerRow, DeltaPerColumn, ScalePerRow
/// and ScalePerColumn
pub(super) struct LineTable<'a> {
  pub(super) name: &'a str,
  pub(super) area: &'a AreaSpec,
  pub(super) axis: TableAxis,
  pub(super) values: &'a [f32],
}

impl LineTable<'_> {
  /// Add the deltas, clipped to -1..1 for floating point data. Deltas are in pixel units of
  /// the image data, so for data scaled from 16 bit integers in 1/65535.
  pub(super) fn apply_delta(&self, image: &mut OpcodeImage<'_>, stage: OpcodeStage) -> Result<()> {
    let unit = if image.scaled_from_integer { 1.0 / 65535.0 } else { 1.0 };
    self.apply(image, stage, &|x: f32, delta| (x + delta * unit).clamp(-1.0, 1.0), &|x: u16, delta| {
      (x as f32 + delta).clamp(0.0, 65535.0).round() as u16
    })
  }

  /// Multiply by the scales, clipped to -1..1 for floating point data
  pub(super) fn apply_scale(&self, image: &mut OpcodeImage<'_>, stage: OpcodeStage) -> Result<()> {
    self.apply(image, stage, &|x: f32, scale| (x * scale).clamp(-1.0, 1.0), &|x: u16, scale| {
      (x as f32 * scale).clamp(0.0, 65535.0).round() as u16
    })
  }

  /// Apply the table to floating point data with `float_fn`, or to the integer
  /// data of OpcodeList1 with `int_fn`.
  fn apply<F, I>(&self, image: &mut OpcodeImage<'_>, stage: OpcodeStage, float_fn: &F, int_fn: &I) -> Result<()>
  where
    F: Fn(f32, f32) -> f32 + Sync,
    I: Fn(u16, f32) -> u16 + Sync,
  {
    let Some(rect) = self.area.buffer_rect(image, stage) else {
      return Ok(());
    };
    let bounds = stage_bounds(image, stage);
    let (width, cpp) = (image.width(), image.channels());
    match (&mut image.data, stage) {
      (OpcodeData::Float(view), _) => self.map(view.as_mut_slice(), width, cpp, rect, bounds, float_fn),
      (OpcodeData::Integer(view), OpcodeStage::List1) => self.map(view.as_mut_slice(), width, cpp, rect, bounds, int_fn),
      (OpcodeData::Integer(_), _) => {
        return Err(RawlerError::DecoderFailed(format!(
          "DNG opcode {} in {} requires image data mapped to linear reference values",
          self.name, stage
        )));
      }
    }
    Ok(())
  }

  /// Map the planes of all pixels in `rect` (stepping by the row and column
  /// pitch) through `f` with the table value of their row or column. Table
  /// positions are relative to `bounds`, the image being modified.
  fn map<T: SubPixel, F>(&self, samples: &mut [T], width: usize, cpp: usize, rect: Rect, bounds: Rect, f: &F)
  where
    F: Fn(T, f32) -> T + Sync,
  {
    let area = self.area;
    let planes = area.plane_range(cpp);
    let col_pitch = area.col_pitch as usize;
    let map_planes = |pixel: &mut [T], value: f32| {
      for c in planes.clone() {
        pixel[c] = f(pixel[c], value);
      }
    };
    let columns: Vec<Option<f32>> = match self.axis {
      TableAxis::Row => Vec::new(),
      TableAxis::Column => (rect.x()..rect.x() + rect.width())
        .step_by(col_pitch)
        .map(|col| self.value(col - bounds.p.x, area.left, area.col_pitch))
        .collect(),
    };
    apply_roi_threaded(samples, width, cpp, rect, area.row_pitch as usize, &|line, row| {
      let pixels = line.chunks_exact_mut(cpp).step_by(col_pitch);
      match self.axis {
        TableAxis::Row => {
          if let Some(value) = self.value(row - bounds.p.y, area.top, area.row_pitch) {
            pixels.for_each(|pixel| map_planes(pixel, value));
          }
        }
        TableAxis::Column => {
          for (pixel, value) in pixels.zip(&columns) {
            if let Some(value) = value {
              map_planes(pixel, *value);
            }
          }
        }
      }
    });
  }

  /// Table value of the row or column at `pos`, counted from the area's
  /// `origin` edge. `None` if the table has no value for it, which happens for
  /// an empty area (covering the whole image) only.
  fn value(&self, pos: usize, origin: u32, pitch: u32) -> Option<f32> {
    let offset = pos.checked_sub(origin as usize)?;
    self.values.get(offset / pitch as usize).copied()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::RawImageData;
  use crate::dng::opcodes::{Opcode, OpcodeList, testutil::*};

  #[test]
  fn line_table_parse() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Rows 1 and 3, columns 0, 2 and 4
    let area = [1, 0, 4, 6, 0, 1, 2, 2];
    let data = opcode_list(&[
      opcode(10, V1_3, 0, &line_table(area, &[1.5, -2.0])),
      opcode(11, V1_3, 0, &line_table(area, &[0.5, 1.0, 2.0])),
      opcode(12, V1_3, 1, &line_table(area, &[0.25, 4.0])),
      opcode(13, V1_3, 1, &line_table(area, &[3.0, 2.0, 1.0])),
    ]);
    let list = OpcodeList::parse(&data)?;
    assert!(matches!(&list.opcodes()[0], Opcode::DeltaPerRow(op) if op.deltas == [1.5, -2.0]));
    assert!(matches!(&list.opcodes()[1], Opcode::DeltaPerColumn(op) if op.deltas == [0.5, 1.0, 2.0]));
    assert!(matches!(&list.opcodes()[2], Opcode::ScalePerRow(op) if op.scales == [0.25, 4.0] && op.area.row_pitch == 2));
    assert!(matches!(&list.opcodes()[3], Opcode::ScalePerColumn(op) if op.scales == [3.0, 2.0, 1.0] && op.flags == 1));
    assert_eq!(list.to_bytes()?, data);
    // Count must match the affected rows or columns
    assert!(single_line_table(10, area, &[1.0, 2.0, 3.0]).is_err());
    assert!(single_line_table(11, area, &[1.0, 2.0]).is_err());
    assert!(single_line_table(12, area, &[1.0]).is_err());
    assert!(single_line_table(13, area, &[1.0, 2.0, 3.0, 4.0]).is_err());
    let list = OpcodeList::parse(&opcode_list(&[opcode(10, V1_3, 1, &line_table(area, &[1.0]))]))?;
    assert!(matches!(&list.opcodes()[0], Opcode::Unknown(op) if op.id == 10));
    // Values must be finite
    assert!(single_line_table(10, area, &[f32::NAN, 1.0]).is_err());
    assert!(single_line_table(13, area, &[1.0, f32::INFINITY, 1.0]).is_err());
    // Size must match the count, invalid area
    let mut params = line_table(area, &[1.0, 2.0]);
    params.truncate(params.len() - 4);
    assert!(OpcodeList::parse(&opcode_list(&[opcode(12, V1_3, 0, &params)])).is_err());
    let mut params = line_table(area, &[1.0, 2.0]);
    params.push(0);
    assert!(OpcodeList::parse(&opcode_list(&[opcode(12, V1_3, 0, &params)])).is_err());
    assert!(single_line_table(10, [0, 0, 4, 4, 0, 0, 1, 1], &[1.0; 4]).is_err());
    Ok(())
  }

  #[test]
  fn scale_per_row_and_column() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Floating point data is clipped to -1..1
    let mut image = float_image(2, 3, 1, 0.5);
    single_line_table(12, [0, 0, 3, 2, 0, 1, 1, 1], &[0.5, 3.0, -4.0])?.apply(&mut image, OpcodeStage::List3)?;
    assert_eq!(floats(&image), [0.25, 0.25, 1.0, 1.0, -1.0, -1.0]);
    // Integer data is rounded and clipped to the 16 bit range
    let mut image = float_image(3, 2, 1, 0.0);
    image.data = RawImageData::Integer(vec![100, 200, 300, 400, 500, 600]);
    single_line_table(13, [0, 0, 2, 3, 0, 1, 1, 1], &[-1.5, 0.29, 1000.0])?.apply(&mut image, OpcodeStage::List1)?;
    assert_eq!(integers(&image), [0, 58, 65535, 0, 145, 65535]);
    // Row pitch
    let mut image = float_image(1, 4, 1, 0.5);
    single_line_table(12, [1, 0, 4, 1, 0, 1, 2, 1], &[2.0, 0.0])?.apply(&mut image, OpcodeStage::List3)?;
    assert_eq!(floats(&image), [0.5, 1.0, 0.5, 0.0]);
    Ok(())
  }

  #[test]
  fn line_table_empty_area() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // An empty area covers the whole image, but the table only has values for
    // the rows or columns of the area. Others are unchanged.
    let mut image = float_image(2, 5, 1, 0.0);
    single_line_table(10, [0, 2, 3, 2, 0, 1, 1, 1], &[0.25, 0.5, 0.75])?.apply(&mut image, OpcodeStage::List3)?;
    assert_eq!(floats(&image), [0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 0.0, 0.0, 0.0, 0.0]);
    let mut image = float_image(4, 1, 1, 1.0);
    single_line_table(13, [0, 1, 0, 3, 0, 1, 1, 1], &[0.25, 0.5])?.apply(&mut image, OpcodeStage::List3)?;
    assert_eq!(floats(&image), [1.0, 0.25, 0.5, 1.0]);
    Ok(())
  }
}
