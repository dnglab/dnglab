// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

use multiversion::multiversion;
use rayon::prelude::*;
use std::time::Instant;

use crate::{
  cfa::{CFA, PlaneColor},
  imgop::{Rect, sensor::Demosaic},
  pixarray::{Color2D, PixF32, RgbF32},
};

/// Bilinear demosaicing implementation for Fujifilm X-Trans sensor data.
///
/// X-Trans sensors use a 6x6 color filter array (CFA) pattern instead of the
/// more common 2x2 Bayer pattern. This demosaicing algorithm uses simple bilinear
/// interpolation over a 5x5 neighborhood to reconstruct missing color channels
/// at each pixel.
///
/// # Quality
///
/// Bilinear interpolation is the simplest demosaicing approach and produces
/// lower quality results compared to more advanced algorithms (e.g. directional
/// or frequency-domain methods). It tends to produce color fringing and
/// zipper artifacts at edges. However, it is fast and suitable for previews
/// or when speed is more important than quality.
#[derive(Default)]
pub struct XTransBilinearDemosaic {}

impl XTransBilinearDemosaic {
  pub fn new() -> Self {
    Self {}
  }
}

impl Demosaic<f32, 3> for XTransBilinearDemosaic {
  /// Demosaic X-Trans sensor data using bilinear interpolation.
  ///
  /// # Parameters
  /// - `pixels`: Single-channel mosaic pixel data (f32).
  /// - `cfa`: The color filter array describing the X-Trans pattern.
  /// - `colors`: Plane-to-color mapping (unused, assumed RGB).
  /// - `roi`: Region of interest within `pixels` to demosaic.
  ///
  /// # Panics
  /// Panics if the CFA pattern is not an RGB pattern.
  #[allow(unused)]
  fn demosaic(&self, pixels: &PixF32, cfa: &CFA, colors: &PlaneColor, roi: Rect) -> Color2D<f32, 3> {
    if !cfa.is_rgb() {
      panic!("CFA pattern '{}' is not a RGB pattern, can not demosaic", cfa);
    }
    let now = Instant::now();
    let rgb = interpolate_bilinear(pixels, cfa, roi);
    log::debug!("X-Trans bilinear demosaic total time: {:.5}s", now.elapsed().as_secs_f32());
    rgb
  }
}

/// Perform bilinear interpolation on X-Trans mosaic data.
///
/// For each pixel in the output, this function averages all same-color samples
/// within a 5x5 window (±2 pixels in each direction) centered on the target
/// pixel. The window is clamped at image boundaries.
///
/// The CFA pattern is shifted according to the ROI origin so that the correct
/// color channel is assigned regardless of where the ROI falls within the
/// full-frame CFA pattern.
///
/// Uses SIMD acceleration via `multiversion` when available (AVX2, SSE, NEON).
#[multiversion(targets("x86_64+avx+avx2+fma", "x86+sse", "aarch64+neon"))]
fn interpolate_bilinear(input: &PixF32, cfa: &CFA, roi: Rect) -> Color2D<f32, 3> {
  let cfa_roi = cfa.shift(roi.p.x, roi.p.y);
  let mut output = RgbF32::new_with_default(roi.width(), roi.height(), f32::NAN);
  let width = output.width;
  let height = output.height;

  output.pixels_mut().par_chunks_exact_mut(width).enumerate().for_each(|(y, line)| {
    line.iter_mut().enumerate().for_each(|(x, pixel)| {
      let mut rgb = [0.0f32; 3];
      let mut count = [0u32; 3];

      let y_lo = y.saturating_sub(2);
      let y_hi = (y + 2).min(height - 1);
      let x_lo = x.saturating_sub(2);
      let x_hi = (x + 2).min(width - 1);

      for row in y_lo..=y_hi {
        for col in x_lo..=x_hi {
          let ch = cfa_roi.color_at(row, col) as usize;
          rgb[ch] += input.at(roi.p.y + row, roi.p.x + col);
          count[ch] += 1;
        }
      }

      for c in 0..3 {
        pixel[c] = if count[c] > 0 { rgb[c] / count[c] as f32 } else { 0.0 };
      }
    });
  });
  output
}
