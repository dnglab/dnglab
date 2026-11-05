// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Scaling of image data

use std::borrow::Cow;

use rayon::prelude::*;

use crate::{imgop::Dim2, pixarray::ImageView};

/// Radius of [`kernel`] in pixels
const KERNEL_RADIUS: f64 = 2.0;

/// Catmull-Rom interpolation
fn kernel(x: f64) -> f64 {
  match x.abs() {
    x if x < 1.0 => (1.5 * x - 2.5) * x * x + 1.0,
    x if x < 2.0 => ((-0.5 * x + 2.5) * x - 4.0) * x + 2.0,
    _ => 0.0,
  }
}

/// Source pixels of one destination pixel and their weights
struct Contribution {
  start: usize,
  weights: Vec<f32>,
}

/// Contributions for scaling `src` pixels to `dst` pixels. The kernel is
/// widened for a smaller size, so all source pixels are taken into account.
fn contributions(src: usize, dst: usize) -> Vec<Contribution> {
  let ratio = src as f64 / dst as f64;
  let width = ratio.max(1.0);
  (0..dst)
    .map(|i| {
      let center = (i as f64 + 0.5) * ratio - 0.5;
      let start = (center - KERNEL_RADIUS * width).ceil().max(0.0) as usize;
      let end = ((center + KERNEL_RADIUS * width).floor() as usize).min(src - 1);
      let weights: Vec<f64> = (start..=end).map(|j| kernel((j as f64 - center) / width)).collect();
      let sum: f64 = weights.iter().sum();
      Contribution {
        start,
        weights: weights.iter().map(|weight| (weight / sum) as f32).collect(),
      }
    })
    .collect()
}

/// Scale `image` to `dim`, the positions of the pixel centers are preserved
pub fn scale_image(image: ImageView<'_, f32>, dim: Dim2) -> Vec<f32> {
  let channels = image.channels();
  let data = if image.width() == dim.w {
    Cow::Borrowed(image.as_slice())
  } else {
    let contributions = contributions(image.width(), dim.w);
    let mut scaled = vec![0.0; dim.w * image.height() * channels];
    scaled
      .par_chunks_exact_mut(dim.w * channels)
      .zip(image.as_slice().par_chunks_exact(image.width() * channels))
      .for_each(|(row, src)| {
        for (pixel, contribution) in row.chunks_exact_mut(channels).zip(&contributions) {
          for (i, weight) in contribution.weights.iter().enumerate() {
            let samples = &src[(contribution.start + i) * channels..][..channels];
            pixel.iter_mut().zip(samples).for_each(|(value, sample)| *value += weight * sample);
          }
        }
      });
    Cow::Owned(scaled)
  };
  if image.height() == dim.h {
    return data.into_owned();
  }
  let stride = dim.w * channels;
  let contributions = contributions(image.height(), dim.h);
  let mut scaled = vec![0.0; stride * dim.h];
  scaled.par_chunks_exact_mut(stride).zip(&contributions).for_each(|(row, contribution)| {
    for (i, weight) in contribution.weights.iter().enumerate() {
      let samples = &data[(contribution.start + i) * stride..][..stride];
      row.iter_mut().zip(samples).for_each(|(value, sample)| *value += weight * sample);
    }
  });
  scaled
}
