// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Masks of DNG raw images: transparency masks and semantic masks

use crate::{
  RawImageData,
  imgop::{Dim2, Point, Rect},
  pixarray::ImageRegion,
};

/// Pixel data of a DNG mask with one sample per pixel
///
/// Zero is transparent (or not part of the semantic attribute), the maximum
/// value is opaque (part of it): 2^bps-1 for integer masks, 1.0 for floating
/// point masks. Values in between are semi-transparent.
#[derive(Debug, Clone, PartialEq)]
pub struct MaskImage {
  pub width: usize,
  pub height: usize,
  /// BitsPerSample of the stored mask: 8 or 16 for integer masks, 16, 24 or
  /// 32 for floating point masks
  pub bps: u32,
  /// `width`\*`height` samples
  pub data: RawImageData,
}

impl MaskImage {
  pub fn new(width: usize, height: usize, bps: u32, data: RawImageData) -> Self {
    let len = match &data {
      RawImageData::Integer(samples) => samples.len(),
      RawImageData::Float(samples) => samples.len(),
    };
    assert_eq!(len, width * height, "mask of {}x{} pixels has {} samples", width, height, len);
    Self { width, height, bps, data }
  }

  pub fn dim(&self) -> Dim2 {
    Dim2::new(self.width, self.height)
  }

  /// Value of fully opaque pixels
  pub fn max_value(&self) -> f32 {
    match self.data {
      RawImageData::Integer(_) => ((1_u32 << self.bps) - 1) as f32,
      RawImageData::Float(_) => 1.0,
    }
  }

  /// Opacity of the pixels, 0.0 (transparent) to 1.0 (opaque)
  pub fn alpha(&self) -> Vec<f32> {
    let max = self.max_value();
    match &self.data {
      RawImageData::Integer(samples) => samples.iter().map(|v| (*v as f32 / max).min(1.0)).collect(),
      RawImageData::Float(samples) => samples.iter().map(|v| v.clamp(0.0, 1.0)).collect(),
    }
  }

  /// Crop the mask to `area`
  pub fn crop(&mut self, area: Rect) -> crate::Result<()> {
    if area.p.x + area.d.w > self.width || area.p.y + area.d.h > self.height {
      return Err(input_error!("Crop {:?} exceeds the mask of {}x{} pixels", area, self.width, self.height));
    }
    if area == Rect::new(Point::zero(), self.dim()) {
      return Ok(());
    }
    let (width, height) = (self.width, self.height);
    self.data = match &self.data {
      RawImageData::Integer(samples) => RawImageData::Integer(ImageRegion::new(samples, width, height, 1, area).to_vec()),
      RawImageData::Float(samples) => RawImageData::Float(ImageRegion::new(samples, width, height, 1, area).to_vec()),
    };
    (self.width, self.height) = (area.d.w, area.d.h);
    Ok(())
  }

  /// Reduce the mask to half the width and height, each pixel is the mean of a
  /// 2x2 block. An odd last row or column is dropped, as by the superpixel demosaic.
  pub fn half_size(&mut self) {
    let (width, height) = (self.width / 2, self.height / 2);
    let src_width = self.width;
    // Combines the 2x2 block of each output pixel by `mean`
    fn blocks<T: Copy>(samples: &[T], src_width: usize, width: usize, height: usize, mean: impl Fn([T; 4]) -> T) -> Vec<T> {
      (0..height)
        .flat_map(|y| {
          let (r1, r2) = (&samples[2 * y * src_width..], &samples[(2 * y + 1) * src_width..]);
          (0..width).map(move |x| (r1[2 * x], r1[2 * x + 1], r2[2 * x], r2[2 * x + 1]))
        })
        .map(|(a, b, c, d)| mean([a, b, c, d]))
        .collect()
    }
    self.data = match &self.data {
      RawImageData::Integer(samples) => RawImageData::Integer(blocks(samples, src_width, width, height, |block| {
        ((block.iter().map(|v| *v as u32).sum::<u32>() + 2) / 4) as u16
      })),
      RawImageData::Float(samples) => RawImageData::Float(blocks(samples, src_width, width, height, |block| block.iter().sum::<f32>() / 4.0)),
    };
    (self.width, self.height) = (width, height);
  }

  /// Extend the mask to `dim` with transparent pixels on the right and bottom
  pub fn pad(&mut self, dim: Dim2) -> crate::Result<()> {
    if dim.w < self.width || dim.h < self.height {
      return Err(input_error!(
        "Can't pad the mask of {}x{} pixels to {}x{}",
        self.width,
        self.height,
        dim.w,
        dim.h
      ));
    }
    if dim == self.dim() {
      return Ok(());
    }
    fn pad<T: Copy + Default>(samples: &[T], width: usize, dim: Dim2) -> Vec<T> {
      let mut padded = vec![T::default(); dim.w * dim.h];
      if width > 0 {
        for (dst, src) in padded.chunks_exact_mut(dim.w).zip(samples.chunks_exact(width)) {
          dst[..width].copy_from_slice(src);
        }
      }
      padded
    }
    self.data = match &self.data {
      RawImageData::Integer(samples) => RawImageData::Integer(pad(samples, self.width, dim)),
      RawImageData::Float(samples) => RawImageData::Float(pad(samples, self.width, dim)),
    };
    (self.width, self.height) = (dim.w, dim.h);
    Ok(())
  }

  /// Rectangle of the largest area within `within` that contains only fully
  /// opaque pixels, `None` if there is no opaque pixel
  pub fn largest_opaque_rect(&self, within: Rect) -> Option<Rect> {
    let within = within.intersect(&Rect::new(Point::zero(), self.dim()))?;
    match &self.data {
      RawImageData::Integer(samples) => {
        let max = self.max_value() as u16;
        largest_rect(within, self.width, |i| samples[i] >= max)
      }
      RawImageData::Float(samples) => largest_rect(within, self.width, |i| samples[i] >= 1.0),
    }
  }
}

/// Largest rectangle within `within` of pixels for which `included` is true,
/// for pixels at index `y * width + x`
///
/// Each row is the base of a histogram of the included pixels above it, the
/// largest rectangle of each histogram is found with a stack of increasing bars.
fn largest_rect(within: Rect, width: usize, included: impl Fn(usize) -> bool) -> Option<Rect> {
  let w = within.d.w;
  let mut heights = vec![0_usize; w];
  let mut stack: Vec<usize> = Vec::with_capacity(w + 1);
  let mut best: Option<Rect> = None;
  let mut best_area = 0;
  for y in within.p.y..within.p.y + within.d.h {
    let row = y * width + within.p.x;
    for (x, height) in heights.iter_mut().enumerate() {
      *height = if included(row + x) { *height + 1 } else { 0 };
    }
    stack.clear();
    for x in 0..=w {
      let height = if x < w { heights[x] } else { 0 };
      while let Some(&top) = stack.last()
        && heights[top] >= height
      {
        stack.pop();
        let left = stack.last().map_or(0, |left| left + 1);
        let area = heights[top] * (x - left);
        if area > best_area {
          best_area = area;
          best = Some(Rect::new(
            Point::new(within.p.x + left, y + 1 - heights[top]),
            Dim2::new(x - left, heights[top]),
          ));
        }
      }
      stack.push(x);
    }
  }
  best
}

/// DNG semantic mask (NewSubFileType 65540)
///
/// The mask covers the field of view of the ActiveArea of the raw image, it
/// may have another resolution than the image data.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticMask {
  /// SemanticName, identifies the purpose of the mask
  pub name: String,
  /// SemanticInstanceID, relates the content of multiple masks
  pub instance_id: Option<String>,
  /// MaskSubArea of a cropped mask: top, left, width and height of the
  /// uncropped mask
  pub sub_area: Option<[u32; 4]>,
  pub image: MaskImage,
}

#[cfg(test)]
mod tests {
  use super::*;

  /// 8-bit mask from rows of '#' (opaque), '+' (semi-transparent) and '.' (transparent)
  fn mask(rows: &[&str]) -> MaskImage {
    let samples: Vec<u16> = rows
      .iter()
      .flat_map(|row| {
        row.chars().map(|c| match c {
          '#' => 255,
          '+' => 128,
          _ => 0,
        })
      })
      .collect();
    MaskImage::new(rows[0].len(), rows.len(), 8, RawImageData::Integer(samples))
  }

  fn integers(mask: &MaskImage) -> &[u16] {
    match &mask.data {
      RawImageData::Integer(samples) => samples,
      RawImageData::Float(_) => panic!("float mask"),
    }
  }

  #[test]
  fn alpha_is_normalized() {
    let mask = MaskImage::new(3, 1, 16, RawImageData::Integer(vec![0, 32768, 65535]));
    assert_eq!(mask.alpha(), vec![0.0, 32768.0 / 65535.0, 1.0]);
    let mask = MaskImage::new(2, 1, 8, RawImageData::Integer(vec![51, 255]));
    assert_eq!(mask.alpha(), vec![0.2, 1.0]);
    let mask = MaskImage::new(3, 1, 32, RawImageData::Float(vec![-0.5, 0.25, 1.5]));
    assert_eq!(mask.alpha(), vec![0.0, 0.25, 1.0]);
  }

  #[test]
  fn crop_and_pad() -> crate::Result<()> {
    let mut mask = MaskImage::new(4, 3, 16, RawImageData::Integer((0..12).collect()));
    mask.crop(Rect::new(Point::new(1, 1), Dim2::new(2, 2)))?;
    assert_eq!((mask.width, mask.height), (2, 2));
    assert_eq!(integers(&mask), &[5, 6, 9, 10]);
    mask.pad(Dim2::new(3, 3))?;
    assert_eq!(integers(&mask), &[5, 6, 0, 9, 10, 0, 0, 0, 0]);
    assert!(mask.crop(Rect::new(Point::new(1, 1), Dim2::new(3, 2))).is_err());
    assert!(mask.pad(Dim2::new(2, 3)).is_err());
    Ok(())
  }

  #[test]
  fn half_size_averages_blocks() {
    let mut mask = MaskImage::new(5, 3, 16, RawImageData::Integer(vec![0, 4, 8, 8, 99, 2, 2, 8, 9, 99, 99, 99, 99, 99, 99]));
    mask.half_size();
    assert_eq!((mask.width, mask.height), (2, 1));
    // Rounded means of (0, 4, 2, 2) and (8, 8, 8, 9), the odd column and row are dropped
    assert_eq!(integers(&mask), &[2, 8]);
    let mut mask = MaskImage::new(2, 2, 32, RawImageData::Float(vec![0.0, 1.0, 1.0, 0.5]));
    mask.half_size();
    assert!(matches!(&mask.data, RawImageData::Float(samples) if samples == &[0.625]));
  }

  #[test]
  fn largest_opaque_rect() {
    let mask = mask(&[
      "###.....", //
      "###.####", "###.####", "....####", "....##+#",
    ]);
    let all = Rect::new(Point::zero(), mask.dim());
    assert_eq!(mask.largest_opaque_rect(all), Some(Rect::new(Point::new(4, 1), Dim2::new(4, 3))));
    // Within the first six columns, the 3x3 block is larger than the 2x4 block
    let within = Rect::new(Point::zero(), Dim2::new(6, 5));
    assert_eq!(mask.largest_opaque_rect(within), Some(Rect::new(Point::zero(), Dim2::new(3, 3))));
    let within = Rect::new(Point::new(4, 4), Dim2::new(1, 1));
    assert_eq!(mask.largest_opaque_rect(within), Some(within));
    // Semi-transparent pixels are not opaque
    let within = Rect::new(Point::new(6, 4), Dim2::new(1, 1));
    assert_eq!(mask.largest_opaque_rect(within), None);
    let opaque = MaskImage::new(3, 2, 32, RawImageData::Float(vec![1.0; 6]));
    assert_eq!(opaque.largest_opaque_rect(all), Some(Rect::new(Point::zero(), Dim2::new(3, 2))));
  }
}
