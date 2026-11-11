// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Sharpening of linear image data by a Laplacian filter
//!
//! The values are sharpened while they are scene-referred, before the tone curve
//! and the transfer function, so the result doesn't depend on the output color
//! space. Linear values are not a good scale for the filter: the noise of the
//! highlights would be amplified much more than the noise of the shadows. Like
//! the noise reduction, it works on the luma stabilized by its square root.
//!
//! Each pixel gets the difference between its stabilized luma and the mean of its
//! four direct neighbors, scaled by the strength. All color planes of the pixel are
//! multiplied by the same factor to reach the new luma, which keeps its chromaticity,
//! so edges get no color fringes. The change fades out near black and white: shadows
//! and highlights have little room for it, and it would mostly amplify their noise.

use std::time::{Duration, Instant};

use multiversion::multiversion;
use rayon::prelude::*;

use super::{
  denoise::{NoiseChange, STABILIZER_OFFSET, detail_deviations},
  luma::luma_coefficients,
};
use crate::pixarray::ImageRegionMut;

/// Default strength: a slight gain of the finest detail
pub const DEFAULT_SHARPEN: f32 = 0.2;

/// Slope of the weight of the change near black and white, stabilized values from
/// 1/8 to 7/8 get the full change
const TONE_SLOPE: f32 = 8.0;

/// Statistics of a sharpening
#[derive(Clone, Copy, Debug, PartialEq)]
struct SharpenStats {
  /// Noise of the stabilized luma values
  noise: NoiseChange,
  /// Mean of the absolute changes of the stabilized luma values
  mean_change: f32,
  /// Largest absolute change of the stabilized luma values
  largest_change: f32,
  /// Time of the sharpening, without the statistics
  elapsed: Duration,
}

/// Sharpen the linear values of `region`, which has 1, 3 or 4 color planes
///
/// The strength is the amount of the Laplacian: 0.0 changes nothing and 1.0 is the
/// classic filter with a center of 2 and neighbors of -1/4. `scale` maps the values
/// of each plane to neutral colors with a white of 1.0: the white balance that is
/// not applied yet and the exposure gain. Only the first coefficients for the planes
/// of `region` are used. The values stay in their scale, they can be above white
/// afterwards. The statistics are logged at the debug level.
pub fn laplacian_sharpen(mut region: ImageRegionMut<'_, f32>, strength: f32, scale: &[f32; 4]) -> crate::Result<()> {
  let collect = log::log_enabled!(log::Level::Debug);
  if let Some(stats) = sharpen_region(region.reborrow(), strength, scale, collect)? {
    log_statistics(&region, strength, &stats);
  }
  Ok(())
}

/// Sharpen `region`, see [`laplacian_sharpen`]. The statistics are only calculated
/// with `collect`, they have no effect on the result. They are missing as well if
/// nothing is changed.
fn sharpen_region(mut region: ImageRegionMut<'_, f32>, strength: f32, scale: &[f32; 4], collect: bool) -> crate::Result<Option<SharpenStats>> {
  let (width, height, channels) = (region.width(), region.height(), region.channels());
  let Some(coefficients) = luma_coefficients(channels, scale) else {
    return Err(input_error!("Sharpening of {} color planes is not supported", channels));
  };
  // Also skips a strength that is not a number
  let active = strength > 0.0;
  if !active || width == 0 || height == 0 {
    return Ok(None);
  }
  let start = Instant::now();
  let mut luma = vec![0.0; width * height];
  read_luma(&region, &coefficients, &mut luma);
  let luma_ref = &luma;
  region.par_rows_mut().enumerate().for_each_init(
    || vec![0.0; width],
    |factors, (y, row)| {
      let line = |y: usize| &luma_ref[y * width..(y + 1) * width];
      let (above, below) = neighbors(y, height);
      row_factors(line(y), line(above), line(below), strength, factors);
      multiply_samples(row, factors, channels);
    },
  );
  let elapsed = start.elapsed();
  Ok(collect.then(|| measure(&region, &coefficients, &luma, elapsed)))
}

/// Positions before and after position `index` of `len` positions, mirrored at
/// the borders. A single position is its own neighbor.
fn neighbors(index: usize, len: usize) -> (usize, usize) {
  let before = if index > 0 { index - 1 } else { 1.min(len - 1) };
  let after = if index + 1 < len { index + 1 } else { len.saturating_sub(2) };
  (before, after)
}

/// Stabilized luma of the pixels of `region`
fn read_luma(region: &ImageRegionMut<'_, f32>, coefficients: &[f32; 4], out: &mut [f32]) {
  let channels = region.channels();
  out
    .par_chunks_exact_mut(region.width())
    .zip(region.as_region().par_rows())
    .for_each(|(out, row)| luma_row(row, channels, coefficients, out));
}

/// Stabilized luma of a row of pixels
#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn luma_row(row: &[f32], channels: usize, coefficients: &[f32; 4], out: &mut [f32]) {
  for (o, pixel) in out.iter_mut().zip(row.chunks_exact(channels)) {
    *o = pixel.iter().zip(coefficients).map(|(sample, coefficient)| sample * coefficient).sum();
  }
  for o in out.iter_mut() {
    *o = (o.max(0.0) + STABILIZER_OFFSET).sqrt();
  }
}

/// Factor for the samples of a pixel with the stabilized luma `center`, by the
/// Laplacian of its four neighbors
#[inline(always)]
fn factor(center: f32, left: f32, right: f32, above: f32, below: f32, strength: f32) -> f32 {
  let detail = center - (left + right + above + below) * 0.25;
  let weight = (TONE_SLOPE * center.min(1.0 - center)).clamp(0.0, 1.0);
  let change = (strength * weight * detail).max(-center);
  let luma = center * center - STABILIZER_OFFSET;
  // The change of the luma is (center + change)^2 - center^2, without a change
  // the factor is exactly 1.0
  if luma > 0.0 {
    (1.0 + change * (2.0 * center + change) / luma).max(0.0)
  } else {
    1.0
  }
}

/// Factors of a row of stabilized luma values and the rows above and below it
#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn row_factors(row: &[f32], above: &[f32], below: &[f32], strength: f32, out: &mut [f32]) {
  let len = row.len();
  for x in [0, len - 1] {
    let (left, right) = neighbors(x, len);
    out[x] = factor(row[x], row[left], row[right], above[x], below[x], strength);
  }
  if len > 2 {
    let samples = out[1..len - 1]
      .iter_mut()
      .zip(&row[1..])
      .zip(row)
      .zip(&row[2..])
      .zip(&above[1..])
      .zip(&below[1..]);
    for (((((o, c), l), r), a), b) in samples {
      *o = factor(*c, *l, *r, *a, *b, strength);
    }
  }
}

/// Multiply all samples of each pixel by its factor
#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn multiply_samples(row: &mut [f32], factors: &[f32], channels: usize) {
  for (pixel, factor) in row.chunks_exact_mut(channels).zip(factors) {
    for sample in pixel {
      *sample *= factor;
    }
  }
}

/// Statistics of the sharpened `region`, with the stabilized luma values `before` it
fn measure(region: &ImageRegionMut<'_, f32>, coefficients: &[f32; 4], before: &[f32], elapsed: Duration) -> SharpenStats {
  let (width, height) = (region.width(), region.height());
  let mut after = vec![0.0; before.len()];
  read_luma(region, coefficients, &mut after);
  let changes = || before.par_iter().zip(&after).map(|(a, b)| (a - b).abs());
  let sum: f64 = changes().map(f64::from).sum();
  SharpenStats {
    noise: NoiseChange::new(&detail_deviations(before, width, height), &detail_deviations(&after, width, height)),
    mean_change: (sum / before.len() as f64) as f32,
    largest_change: changes().reduce(|| 0.0, f32::max),
    elapsed,
  }
}

/// Log the statistics of a sharpening of `region`
fn log_statistics(region: &ImageRegionMut<'_, f32>, strength: f32, stats: &SharpenStats) {
  log::debug!("Sharpen: the noise is the deviation of the detail of the stabilized luma in the flattest blocks of the input");
  log::debug!(
    "Sharpen of {}x{} pixels, {} planes, strength {}: noise {:.5} -> {:.5} (x{:.2}), mean change {:.5}, largest change {:.5}, {:.3} s",
    region.width(),
    region.height(),
    region.channels(),
    strength,
    stats.noise.before,
    stats.noise.after,
    stats.noise.percent_left() / 100.0,
    stats.mean_change,
    stats.largest_change,
    stats.elapsed.as_secs_f64()
  );
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::imgop::{Dim2, Point, Rect};
  use crate::pixarray::ImageViewMut;

  const NEUTRAL: [f32; 4] = [1.0; 4];

  fn region(data: &mut [f32], width: usize, height: usize, channels: usize) -> ImageRegionMut<'_, f32> {
    ImageRegionMut::from(ImageViewMut::new(data, width, height, channels))
  }

  /// Stabilized value of a linear value
  fn stabilized(value: f32) -> f32 {
    (value.max(0.0) + STABILIZER_OFFSET).sqrt()
  }

  /// Linear value of a stabilized value
  fn linear(value: f32) -> f32 {
    value * value - STABILIZER_OFFSET
  }

  /// Image with white noise on the values of `base`, for a position and color plane
  fn noisy_image(width: usize, height: usize, channels: usize, amount: f32, base: impl Fn(usize, usize, usize) -> f32) -> Vec<f32> {
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut data = Vec::with_capacity(width * height * channels);
    for y in 0..height {
      for x in 0..width {
        for c in 0..channels {
          state ^= state << 13;
          state ^= state >> 7;
          state ^= state << 17;
          let noise = (state >> 40) as f32 / (1_u64 << 24) as f32 - 0.5;
          data.push(base(x, y, c) + amount * noise);
        }
      }
    }
    data
  }

  fn sharpened(input: &[f32], width: usize, height: usize, channels: usize, strength: f32) -> crate::Result<Vec<f32>> {
    let mut output = input.to_vec();
    laplacian_sharpen(region(&mut output, width, height, channels), strength, &NEUTRAL)?;
    Ok(output)
  }

  /// Monochrome image of the stabilized values `values`
  fn from_stabilized(values: &[f32]) -> Vec<f32> {
    values.iter().map(|v| linear(*v)).collect()
  }

  /// Image of 3x3 pixels with the value `center` at (1, 1) and the value `other` around it
  fn cross(center: f32, other: f32) -> Vec<f32> {
    let mut data = vec![other; 9];
    data[4] = center;
    data
  }

  #[test]
  fn known_values_with_mirrored_borders() -> crate::Result<()> {
    #[rustfmt::skip]
    let input = from_stabilized(&[
      0.2, 0.3, 0.4,
      0.3, 0.5, 0.6,
      0.4, 0.6, 0.8,
    ]);
    let output = sharpened(&input, 3, 3, 1, 1.0)?;
    // All pixels are within the range of the full weight
    let expected = [
      // The neighbors outside of the image are the ones on the other side
      0.2 + (0.2 - (0.3 + 0.3 + 0.3 + 0.3) / 4.0),
      0.3 + (0.3 - (0.2 + 0.4 + 0.5 + 0.5) / 4.0),
      0.4 + (0.4 - (0.3 + 0.3 + 0.6 + 0.6) / 4.0),
      0.3 + (0.3 - (0.5 + 0.5 + 0.2 + 0.4) / 4.0),
      0.5 + (0.5 - (0.3 + 0.6 + 0.3 + 0.6) / 4.0),
      0.6 + (0.6 - (0.5 + 0.5 + 0.4 + 0.8) / 4.0),
      0.4 + (0.4 - (0.6 + 0.6 + 0.3 + 0.3) / 4.0),
      0.6 + (0.6 - (0.4 + 0.8 + 0.5 + 0.5) / 4.0),
      0.8 + (0.8 - (0.6 + 0.6 + 0.6 + 0.6) / 4.0),
    ];
    for (i, (value, expected)) in output.iter().zip(expected).enumerate() {
      assert!((stabilized(*value) - expected).abs() < 1e-5, "{}: {} != {}", i, stabilized(*value), expected);
    }
    Ok(())
  }

  #[test]
  fn flat_image_is_unchanged() -> crate::Result<()> {
    let input: Vec<f32> = (0..16 * 16).flat_map(|_| [0.2, 0.5, 0.7]).collect();
    let output = sharpened(&input, 16, 16, 3, 2.0)?;
    assert!(input.iter().zip(&output).all(|(a, b)| (a - b).abs() < 1e-5));
    Ok(())
  }

  #[test]
  fn soft_edge_gets_steeper() -> crate::Result<()> {
    // Edge over four columns of stabilized values
    let edge = from_stabilized(&[0.4, 0.4, 0.4, 0.4, 0.45, 0.55, 0.65, 0.7, 0.7, 0.7, 0.7, 0.7]);
    let input: Vec<f32> = (0..12 * 12).flat_map(|i| [edge[i % 12]; 3]).collect();
    let output = sharpened(&input, 12, 12, 3, 1.0)?;
    let row: Vec<f32> = output[6 * 12 * 3..7 * 12 * 3].iter().step_by(3).map(|v| stabilized(*v)).collect();
    // The edge rises by 0.225 instead of 0.2 between columns 4 and 6, the columns
    // next to it get darker and brighter
    let expected = [0.4, 0.4, 0.4, 0.3875, 0.4375, 0.55, 0.6625, 0.7125, 0.7, 0.7, 0.7, 0.7];
    for (x, (value, expected)) in row.iter().zip(expected).enumerate() {
      assert!((value - expected).abs() < 1e-5, "{}: {} != {}", x, value, expected);
    }
    Ok(())
  }

  #[test]
  fn strength_scales_the_change_of_stabilized_values() -> crate::Result<()> {
    let input = noisy_image(32, 32, 3, 0.05, |_, _, _| 0.25);
    let difference = |output: &[f32]| -> Vec<f32> {
      input
        .chunks_exact(3)
        .zip(output.chunks_exact(3))
        .map(|(a, b)| stabilized((b[0] + 2.0 * b[1] + b[2]) / 4.0) - stabilized((a[0] + 2.0 * a[1] + a[2]) / 4.0))
        .collect()
    };
    let base = difference(&sharpened(&input, 32, 32, 3, 0.2)?);
    assert!(base.iter().any(|v| v.abs() > 1e-3));
    for factor in [0.5, 2.0, 5.0] {
      let scaled = difference(&sharpened(&input, 32, 32, 3, 0.2 * factor)?);
      for (a, b) in base.iter().zip(&scaled) {
        assert!((a * factor - b).abs() < 1e-4, "{}: {} {}", factor, a, b);
      }
    }
    Ok(())
  }

  #[test]
  fn no_strength_leaves_the_image_unchanged() -> crate::Result<()> {
    let input = noisy_image(16, 16, 3, 0.1, |_, _, _| 0.5);
    for strength in [0.0, -1.0, f32::NAN] {
      assert_eq!(sharpened(&input, 16, 16, 3, strength)?, input, "{}", strength);
    }
    Ok(())
  }

  #[test]
  fn change_fades_out_at_black_and_white() -> crate::Result<()> {
    // The full change is 0.04 below the center
    for (tone, weight) in [
      (0.5, 1.0),
      (0.125, 1.0),
      (0.875, 1.0),
      (0.0625, 0.5),
      (0.9375, 0.5),
      (0.03125, 0.25),
      (0.96875, 0.25),
    ] {
      let output = sharpened(&cross(linear(tone), linear(tone + 0.04)), 3, 3, 1, 1.0)?;
      let change = stabilized(output[4]) - tone;
      assert!((change + 0.04 * weight).abs() < 1e-4, "{}: {}", tone, change);
    }
    // Black, white and values above it are kept next to any neighbor
    for value in [0.0, 1.0, 1.5] {
      assert_eq!(sharpened(&cross(value, 0.25), 3, 3, 1, 2.0)?[4], value);
    }
    Ok(())
  }

  #[test]
  fn clipped_highlights_next_to_an_edge_stay_white() -> crate::Result<()> {
    let input: Vec<f32> = (0..8 * 8).flat_map(|i| [if i % 8 < 4 { 1.0 } else { 0.1 }; 3]).collect();
    let output = sharpened(&input, 8, 8, 3, 2.0)?;
    assert_ne!(input, output);
    assert!(output.chunks_exact(3 * 8).all(|row| row[..4 * 3].iter().all(|v| *v == 1.0)));
    Ok(())
  }

  #[test]
  fn results_are_not_negative_and_can_be_above_white() -> crate::Result<()> {
    let input = noisy_image(32, 32, 3, 0.1, |x, _, c| match (x / 4 % 2, c) {
      (0, 0) => 0.8,
      (0, _) => 0.1,
      (_, 1) => 0.7,
      _ => 0.05,
    });
    let input: Vec<f32> = input.iter().map(|v| v.max(0.0)).collect();
    let output = sharpened(&input, 32, 32, 3, 2.0)?;
    assert!(output.iter().all(|v| v.is_finite() && *v >= 0.0));
    // A dark pixel between bright ones stops at zero
    assert_eq!(sharpened(&cross(linear(0.2), linear(0.9)), 3, 3, 1, 2.0)?[4], 0.0);
    // A bright pixel between dark ones keeps its value above white, the color
    // operations limit it
    let bright = sharpened(&cross(linear(0.85), linear(0.2)), 3, 3, 1, 2.0)?[4];
    assert!((stabilized(bright) - 2.15).abs() < 1e-4, "{}", stabilized(bright));
    Ok(())
  }

  #[test]
  fn all_color_planes_get_the_same_factor() -> crate::Result<()> {
    let input = noisy_image(32, 32, 3, 0.05, |x, y, c| 0.2 + 0.1 * c as f32 + 0.005 * (x + y) as f32);
    let output = sharpened(&input, 32, 32, 3, 0.5)?;
    assert_ne!(input, output);
    for (before, after) in input.chunks_exact(3).zip(output.chunks_exact(3)) {
      let factor = after[0] / before[0];
      assert!((after[1] / before[1] - factor).abs() < 1e-4, "{:?} {:?}", before, after);
      assert!((after[2] / before[2] - factor).abs() < 1e-4, "{:?} {:?}", before, after);
    }
    Ok(())
  }

  #[test]
  fn monochrome_is_sharpened_like_neutral_pixels() -> crate::Result<()> {
    let mono = noisy_image(32, 32, 1, 0.1, |x, _, _| 0.2 + 0.01 * x as f32);
    let sharpened_mono = sharpened(&mono, 32, 32, 1, 0.5)?;
    assert_ne!(mono, sharpened_mono);
    for channels in [3, 4] {
      let color: Vec<f32> = mono.iter().flat_map(|v| vec![*v; channels]).collect();
      let sharpened_color = sharpened(&color, 32, 32, channels, 0.5)?;
      for (value, pixel) in sharpened_mono.iter().zip(sharpened_color.chunks_exact(channels)) {
        // Neutral pixels stay neutral
        assert!(pixel.iter().all(|v| (v - value).abs() < 1e-5), "{} {:?}", value, pixel);
      }
    }
    Ok(())
  }

  #[test]
  fn scale_gives_the_neutral_values_of_the_luma() -> crate::Result<()> {
    let mono = noisy_image(32, 32, 1, 0.1, |x, _, _| 0.2 + 0.01 * x as f32);
    let expected = sharpened(&mono, 32, 32, 1, 0.5)?;
    let close = |a: f32, b: f32| (a - b).abs() < 1e-4;

    // Values before the white balance
    for wb in [[2.0, 1.0, 1.5, f32::NAN], [2.0, 1.0, 1.5, 0.0], [2.0, 1.0, 1.5, 7.0]] {
      let mut color: Vec<f32> = mono.iter().flat_map(|v| [v / wb[0], v / wb[1], v / wb[2]]).collect();
      laplacian_sharpen(region(&mut color, 32, 32, 3), 0.5, &wb)?;
      for (value, pixel) in expected.iter().zip(color.chunks_exact(3)) {
        assert!((0..3).all(|c| close(pixel[c] * wb[c], *value)), "{} {:?}", value, pixel);
      }
    }
    // Values before an exposure gain, white is at 0.5 of them
    let mut darker: Vec<f32> = mono.iter().map(|v| v / 2.0).collect();
    laplacian_sharpen(region(&mut darker, 32, 32, 1), 0.5, &[2.0; 4])?;
    assert!(expected.iter().zip(&darker).all(|(a, b)| close(*a, b * 2.0)));
    // A coefficient that can't be used is ignored
    let mut output = mono.clone();
    laplacian_sharpen(region(&mut output, 32, 32, 1), 0.5, &[f32::NAN; 4])?;
    assert_eq!(output, expected);
    Ok(())
  }

  #[test]
  fn other_numbers_of_color_planes_are_rejected() {
    for channels in [2, 5] {
      assert!(laplacian_sharpen(region(&mut vec![0.5; 4 * channels], 2, 2, channels), 1.0, &NEUTRAL).is_err());
    }
    // Also without a strength
    assert!(laplacian_sharpen(region(&mut [0.5; 8], 2, 2, 2), 0.0, &NEUTRAL).is_err());
  }

  #[test]
  fn small_regions() -> crate::Result<()> {
    for (width, height) in [(1, 1), (2, 1), (1, 2), (2, 2), (3, 1), (1, 3), (3, 3), (31, 2), (2, 31)] {
      for channels in [1, 3, 4] {
        let input = noisy_image(width, height, channels, 0.1, |_, _, _| 0.5);
        for collect in [false, true] {
          let mut output = input.clone();
          sharpen_region(region(&mut output, width, height, channels), 1.0, &NEUTRAL, collect)?;
          assert!(output.iter().all(|v| v.is_finite()), "{}x{}", width, height);
        }
      }
    }
    // A single pixel has no detail
    assert!((sharpened(&[0.25], 1, 1, 1, 2.0)?[0] - 0.25).abs() < 1e-6);
    // The neighbors of a row of two pixels are the other pixel and the pixel itself
    let input = from_stabilized(&[0.4, 0.6]);
    for (width, height) in [(2, 1), (1, 2)] {
      let output = sharpened(&input, width, height, 1, 1.0)?;
      assert!(
        (stabilized(output[0]) - 0.3).abs() < 1e-5 && (stabilized(output[1]) - 0.7).abs() < 1e-5,
        "{:?}",
        output
      );
    }
    // Without pixels
    sharpen_region(region(&mut [], 0, 0, 3), 1.0, &NEUTRAL, true)?;
    Ok(())
  }

  #[test]
  fn region_is_sharpened_like_a_copy_of_it() -> crate::Result<()> {
    let (width, height) = (32, 24);
    let area = Rect::new(Point::new(8, 6), Dim2::new(16, 12));
    for channels in [1, 3] {
      let input = noisy_image(width, height, channels, 0.1, |x, y, _| 0.2 + 0.01 * x as f32 + 0.002 * y as f32);
      let mut output = input.clone();
      laplacian_sharpen(ImageRegionMut::new(&mut output, width, height, channels, area), 0.5, &NEUTRAL)?;

      let inside = |x: usize, y: usize| (area.p.x..area.p.x + area.d.w).contains(&x) && (area.p.y..area.p.y + area.d.h).contains(&y);
      let mut copy = Vec::new();
      for y in 0..height {
        for x in 0..width {
          let pixel = (y * width + x) * channels;
          if inside(x, y) {
            copy.extend_from_slice(&input[pixel..pixel + channels]);
          } else {
            assert_eq!(output[pixel..pixel + channels], input[pixel..pixel + channels], "{} {}", x, y);
          }
        }
      }
      laplacian_sharpen(region(&mut copy, area.d.w, area.d.h, channels), 0.5, &NEUTRAL)?;
      let result: Vec<f32> = (area.p.y..area.p.y + area.d.h)
        .flat_map(|y| output[(y * width + area.p.x) * channels..(y * width + area.p.x + area.d.w) * channels].to_vec())
        .collect();
      assert_eq!(result, copy);
    }
    Ok(())
  }

  #[test]
  fn statistics_have_no_effect_on_the_result() -> crate::Result<()> {
    let input = noisy_image(64, 64, 3, 0.1, |_, _, c| 0.2 + 0.1 * c as f32);
    let (mut with, mut without) = (input.clone(), input.clone());
    let stats = sharpen_region(region(&mut with, 64, 64, 3), 0.5, &NEUTRAL, true)?;
    assert!(sharpen_region(region(&mut without, 64, 64, 3), 0.5, &NEUTRAL, false)?.is_none());
    assert_eq!(with, without);
    assert!(stats.is_some());
    // Nothing to report if nothing is changed
    assert!(sharpen_region(region(&mut with, 64, 64, 3), 0.0, &NEUTRAL, true)?.is_none());
    Ok(())
  }

  #[test]
  fn statistics_show_the_gain_of_the_noise() -> crate::Result<()> {
    let strength = 0.5;
    let mut output = noisy_image(128, 128, 1, 0.02, |_, _, _| 0.25);
    let input = output.clone();
    let stats = sharpen_region(region(&mut output, 128, 128, 1), strength, &NEUTRAL, true)?.expect("statistics");
    // Gain of white noise: the center with 1 + strength, four neighbors with strength / 4
    let gain = ((1.0 + strength).powi(2) + 4.0 * (strength / 4.0).powi(2)).sqrt();
    let measured = stats.noise.after / stats.noise.before;
    assert!((measured - gain).abs() < 0.05 * gain, "{} {}", measured, gain);
    let changes: Vec<f32> = input.iter().zip(&output).map(|(a, b)| (stabilized(*a) - stabilized(*b)).abs()).collect();
    let mean = changes.iter().sum::<f32>() / changes.len() as f32;
    let largest = changes.iter().copied().fold(0.0, f32::max);
    assert!((stats.mean_change - mean).abs() < 1e-5, "{} {}", stats.mean_change, mean);
    assert!((stats.largest_change - largest).abs() < 1e-5, "{} {}", stats.largest_change, largest);
    Ok(())
  }
}
