// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Exposure of linear image data
//!
//! All color planes of a pixel are multiplied by the same factor, which keeps its
//! chromaticity. A negative exposure follows the curve of a negative
//! BaselineExposure instead of a plain multiplication: clipped highlights stay
//! white and don't turn gray.
//!
//! The estimate of [`auto_exposure`] meters the image like a camera would, but it
//! only corrects what the highlights show to be wrong. A dark image is brightened
//! as far as its highlights have room, a bright image is only darkened if its
//! highlights are clipped. A plain meter would also pull bright scenes towards
//! gray and lift dark ones, which makes most correct exposures worse.

use std::time::Instant;

use rayon::prelude::*;

use super::luma::luma_coefficients;
use crate::{
  dng::profile_render::ToneCurve,
  pixarray::{ImageRegion, ImageRegionMut},
};

/// Number of evenly spaced pixels to read for the estimate, at least
const METER_PIXELS: usize = 250_000;

/// Geometric mean of the luma that a badly exposed image is corrected to, normally
/// exposed files have about 0.1
const TARGET_LUMA: f32 = 0.08;

/// Added to the luma before the logarithm, so black pixels don't dominate the mean
const LOG_OFFSET: f32 = 1.0 / 1024.0;

/// Share of the pixels below the highlights of an image, in permille
const HIGHLIGHT_PERMILLE: usize = 995;

/// Highlights of this luma or above are clipped
const CLIPPED_LUMA: f32 = 0.98;

/// Largest correction of the estimate in EV
const MAX_AUTO_EV: f32 = 4.0;

/// Largest factor of a positive exposure. Samples stay finite, an infinite factor
/// would make black pixels NaN. It is far above any useful factor: the smallest
/// sample above zero of 16 bit data is at white after 16 EV.
const MAX_FACTOR: f32 = 1e30;

/// Luma coefficients of `channels` color planes, or an error for other numbers of planes
fn coefficients(channels: usize, scale: &[f32; 4]) -> crate::Result<[f32; 4]> {
  let Some(coefficients) = luma_coefficients(channels, scale) else {
    return Err(input_error!("Exposure of {} color planes is not supported", channels));
  };
  Ok(coefficients)
}

/// Luma of a pixel
#[inline(always)]
fn pixel_luma(pixel: &[f32], coefficients: &[f32; 4]) -> f32 {
  pixel.iter().zip(coefficients).map(|(sample, coefficient)| sample * coefficient).sum()
}

/// Estimate the correction of the exposure of the linear values of `region`, in
/// EV, without changing them
///
/// `scale` maps the values of each plane to neutral colors in the brightness of
/// the image, with a white of 1.0, see [`luma_coefficients`]. The correction is 0.0
/// if the image needs none or has no pixels.
pub fn auto_exposure(region: ImageRegion<'_, f32>, scale: &[f32; 4]) -> crate::Result<f32> {
  let start = Instant::now();
  let channels = region.channels();
  let coefficients = coefficients(channels, scale)?;
  let (width, height) = (region.width(), region.height());
  // Evenly spaced pixels, the distance of the rows and columns is the same
  let step = ((width * height / METER_PIXELS) as f64).sqrt().max(1.0) as usize;
  let mut luma: Vec<f32> = region
    .rows()
    .step_by(step)
    .flat_map(|row| row.chunks_exact(channels).step_by(step))
    .map(|pixel| pixel_luma(pixel, &coefficients).max(0.0))
    .filter(|luma| luma.is_finite())
    .collect();
  if luma.is_empty() {
    return Ok(0.0);
  }
  let sum: f64 = luma.iter().map(|luma| f64::from((luma + LOG_OFFSET).log2())).sum();
  let mean_log = (sum / luma.len() as f64) as f32;
  let index = (luma.len() * HIGHLIGHT_PERMILLE / 1000).min(luma.len() - 1);
  let (_, highlight, _) = luma.select_nth_unstable_by(index, f32::total_cmp);
  let highlight = *highlight;
  let ev = correction(mean_log, highlight);
  log::debug!(
    "Auto exposure of {}x{} pixels, {} read: mean log2 luma {:.3}, highlights {:.3}, metered {:+.2} EV, correction {:+.2} EV, {:.3} s",
    width,
    height,
    luma.len(),
    mean_log,
    highlight,
    TARGET_LUMA.log2() - mean_log,
    ev,
    start.elapsed().as_secs_f64()
  );
  Ok(ev)
}

/// Correction in EV of an image with the mean of the log2 luma `mean_log` and the
/// luma `highlight` of its highlights
fn correction(mean_log: f32, highlight: f32) -> f32 {
  let metered = TARGET_LUMA.log2() - mean_log;
  let ev = if metered > 0.0 {
    // Only as far as the highlights reach white
    metered.min(-highlight.min(1.0).log2())
  } else if highlight >= CLIPPED_LUMA {
    metered
  } else {
    0.0
  };
  ev.clamp(-MAX_AUTO_EV, MAX_AUTO_EV)
}

/// Change the exposure of the linear values of `region` by `ev`
///
/// `scale` maps the values of each plane to neutral colors with a white of 1.0, see
/// [`luma_coefficients`]. A positive exposure multiplies the values by 2^ev, they
/// can be above white afterwards. A negative exposure scales the luma by the curve
/// of [`ToneCurve::negative_exposure`]: up to 0.25 by 2^ev and less above, a luma
/// of 1.0 or more is not changed.
pub fn apply_exposure(mut region: ImageRegionMut<'_, f32>, ev: f32, scale: &[f32; 4]) -> crate::Result<()> {
  let start = Instant::now();
  let channels = region.channels();
  let coefficients = coefficients(channels, scale)?;
  if let Some(curve) = ToneCurve::negative_exposure(ev) {
    region.par_rows_mut().for_each(|row| {
      for pixel in row.chunks_exact_mut(channels) {
        let luma = pixel_luma(pixel, &coefficients);
        if luma > 0.0 && luma < 1.0 {
          let factor = curve.eval(luma) / luma;
          pixel.iter_mut().for_each(|sample| *sample *= factor);
        }
      }
    });
  } else if ev > 0.0 {
    let factor = ev.exp2().min(MAX_FACTOR);
    region.par_rows_mut().for_each(|row| row.iter_mut().for_each(|sample| *sample *= factor));
  }
  log::debug!(
    "Exposure of {}x{} pixels: {:+.2} EV, {:.3} s",
    region.width(),
    region.height(),
    ev,
    start.elapsed().as_secs_f64()
  );
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::imgop::{Dim2, Point, Rect};
  use crate::pixarray::{ImageView, ImageViewMut};

  const NEUTRAL: [f32; 4] = [1.0; 4];

  fn estimate(data: &[f32], width: usize, height: usize, channels: usize, scale: &[f32; 4]) -> crate::Result<f32> {
    auto_exposure(ImageRegion::from(ImageView::new(data, width, height, channels)), scale)
  }

  fn applied(input: &[f32], width: usize, height: usize, channels: usize, ev: f32) -> crate::Result<Vec<f32>> {
    let mut output = input.to_vec();
    apply_exposure(ImageRegionMut::from(ImageViewMut::new(&mut output, width, height, channels)), ev, &NEUTRAL)?;
    Ok(output)
  }

  /// Monochrome image of 256x256 pixels with a gradient from `low` to `high`
  fn gradient(low: f32, high: f32) -> Vec<f32> {
    (0..256 * 256).map(|i| low + (high - low) * ((i % 256) as f32 + 0.5) / 256.0).collect()
  }

  /// Mean of the log2 luma of a monochrome image, like the estimate
  fn mean_log2(data: &[f32]) -> f32 {
    data.iter().map(|v| (v + LOG_OFFSET).log2()).sum::<f32>() / data.len() as f32
  }

  #[test]
  fn dark_image_is_brightened_to_the_target() -> crate::Result<()> {
    // The highlights leave more room than the meter asks for
    let image = gradient(0.02, 0.09);
    let ev = estimate(&image, 256, 256, 1, &NEUTRAL)?;
    let expected = TARGET_LUMA.log2() - mean_log2(&image);
    assert!(expected > 0.0 && (ev - expected).abs() < 1e-3, "{} {}", ev, expected);
    Ok(())
  }

  #[test]
  fn brightening_is_limited_by_the_highlights() -> crate::Result<()> {
    // The meter asks for much more than the 1 EV that the highlights have room for
    let mut image = vec![0.002; 256 * 256];
    image[..256 * 16].fill(0.5);
    let ev = estimate(&image, 256, 256, 1, &NEUTRAL)?;
    assert!((ev - 1.0).abs() < 1e-3, "{}", ev);
    assert!(TARGET_LUMA.log2() - mean_log2(&image) > 2.0);
    Ok(())
  }

  #[test]
  fn bright_image_without_clipped_highlights_is_kept() -> crate::Result<()> {
    assert_eq!(estimate(&gradient(0.3, 0.9), 256, 256, 1, &NEUTRAL)?, 0.0);
    Ok(())
  }

  #[test]
  fn overexposed_image_with_clipped_highlights_is_darkened() -> crate::Result<()> {
    let mut image = gradient(0.3, 0.9);
    image[..256 * 32].fill(1.0);
    let ev = estimate(&image, 256, 256, 1, &NEUTRAL)?;
    let expected = TARGET_LUMA.log2() - mean_log2(&image);
    assert!(expected < 0.0 && (ev - expected).abs() < 1e-3, "{} {}", ev, expected);
    Ok(())
  }

  #[test]
  fn clipped_highlights_without_a_bright_image_are_kept() -> crate::Result<()> {
    // Highlights at white leave no room, also if the meter asks for more
    let mut image = vec![0.01; 256 * 256];
    image[..256 * 16].fill(1.0);
    assert_eq!(estimate(&image, 256, 256, 1, &NEUTRAL)?, 0.0);
    Ok(())
  }

  #[test]
  fn estimate_follows_a_change_of_the_exposure() -> crate::Result<()> {
    // Between the limits, the offset of the logarithm is the only difference
    let base = gradient(0.03, 0.09);
    let reference = estimate(&base, 256, 256, 1, &NEUTRAL)?;
    assert!(reference > 0.0);
    for k in [-2, -1] {
      let factor = (k as f32).exp2();
      let scaled: Vec<f32> = base.iter().map(|v| v * factor).collect();
      let ev = estimate(&scaled, 256, 256, 1, &NEUTRAL)?;
      assert!((ev - (reference - k as f32)).abs() < 0.15, "{}: {} {}", k, ev, reference);
    }
    Ok(())
  }

  #[test]
  fn scale_is_taken_into_account() -> crate::Result<()> {
    // Values before a white balance and a gain
    let wb = [2.0, 1.0, 1.5, f32::NAN];
    let gray = gradient(0.005, 0.1);
    let expected = estimate(&gray, 256, 256, 1, &NEUTRAL)?;
    assert!(expected > 0.0);
    let color: Vec<f32> = gray.iter().flat_map(|v| [v / 2.0, *v, v / 1.5]).collect();
    assert!((estimate(&color, 256, 256, 3, &wb)? - expected).abs() < 1e-3);
    let darker: Vec<f32> = gray.iter().map(|v| v / 4.0).collect();
    assert!((estimate(&darker, 256, 256, 1, &[4.0; 4])? - expected).abs() < 1e-3);
    // A coefficient that can't be used counts as 1.0
    assert_eq!(estimate(&gray, 256, 256, 1, &[f32::NAN; 4])?, expected);
    Ok(())
  }

  #[test]
  fn correction_is_limited() -> crate::Result<()> {
    // Black, also without highlights to limit the correction
    let ev = estimate(&vec![0.0; 256 * 256], 256, 256, 1, &NEUTRAL)?;
    assert_eq!(ev, MAX_AUTO_EV);
    assert_eq!(correction(-20.0, 1e-6), MAX_AUTO_EV);
    // Far above white
    assert_eq!(estimate(&vec![1000.0; 256 * 256], 256, 256, 1, &NEUTRAL)?, -MAX_AUTO_EV);
    Ok(())
  }

  #[test]
  fn values_that_are_not_finite_are_ignored() -> crate::Result<()> {
    let mut image = gradient(0.005, 0.1);
    let expected = estimate(&image, 256, 256, 1, &NEUTRAL)?;
    assert!(expected > 0.0);
    image[3] = f32::INFINITY;
    assert!(estimate(&image, 256, 256, 1, &NEUTRAL)?.is_finite());
    image[3] = f32::NAN;
    assert!((estimate(&image, 256, 256, 1, &NEUTRAL)? - expected).abs() < 0.01);
    Ok(())
  }

  #[test]
  fn region_without_pixels_needs_no_correction() -> crate::Result<()> {
    assert_eq!(estimate(&[], 0, 0, 3, &NEUTRAL)?, 0.0);
    Ok(())
  }

  #[test]
  fn region_is_estimated_like_a_copy_of_it() -> crate::Result<()> {
    let (width, height) = (64, 48);
    let area = Rect::new(Point::new(8, 6), Dim2::new(40, 32));
    let image: Vec<f32> = (0..width * height).map(|i| 0.01 + 0.002 * (i % width) as f32).collect();
    let in_region = auto_exposure(ImageRegion::new(&image, width, height, 1, area), &NEUTRAL)?;
    let copy: Vec<f32> = (area.p.y..area.p.y + area.d.h)
      .flat_map(|y| image[y * width + area.p.x..y * width + area.p.x + area.d.w].to_vec())
      .collect();
    assert_eq!(in_region, estimate(&copy, area.d.w, area.d.h, 1, &NEUTRAL)?);
    assert_ne!(in_region, estimate(&image, width, height, 1, &NEUTRAL)?);
    Ok(())
  }

  #[test]
  fn large_images_are_read_at_a_distance() -> crate::Result<()> {
    // A distance of two pixels: only the even columns are read
    let (width, height) = (1024, 1024);
    let image: Vec<f32> = (0..width * height).map(|i| if (i % width) % 2 == 0 { 0.02 } else { 0.5 }).collect();
    let expected = TARGET_LUMA.log2() - (0.02 + LOG_OFFSET).log2();
    assert!((estimate(&image, width, height, 1, &NEUTRAL)? - expected).abs() < 1e-3);
    Ok(())
  }

  #[test]
  fn numbers_of_color_planes() -> crate::Result<()> {
    let gray = gradient(0.005, 0.1);
    let expected = estimate(&gray, 256, 256, 1, &NEUTRAL)?;
    assert!(expected > 0.0);
    for channels in [3, 4] {
      let color: Vec<f32> = gray.iter().flat_map(|v| vec![*v; channels]).collect();
      assert!((estimate(&color, 256, 256, channels, &NEUTRAL)? - expected).abs() < 1e-3, "{}", channels);
      let brighter: Vec<f32> = color.iter().map(|v| v * 2.0).collect();
      assert_eq!(applied(&color, 256, 256, channels, 1.0)?, brighter, "{}", channels);
    }
    for channels in [2, 5] {
      let mut data = vec![0.5; 4 * channels];
      assert!(estimate(&data, 2, 2, channels, &NEUTRAL).is_err(), "{}", channels);
      assert!(applied(&data, 2, 2, channels, 1.0).is_err(), "{}", channels);
      // Also without a change
      let region = ImageRegionMut::from(ImageViewMut::new(&mut data, 2, 2, channels));
      assert!(apply_exposure(region, 0.0, &NEUTRAL).is_err(), "{}", channels);
    }
    Ok(())
  }

  #[test]
  fn positive_exposure_multiplies_all_values() -> crate::Result<()> {
    let input = [0.0, 0.1, 0.2, 0.5, 1.0, 2.0];
    assert_eq!(applied(&input, 6, 1, 1, 1.0)?, [0.0, 0.2, 0.4, 1.0, 2.0, 4.0]);
    assert_eq!(applied(&input, 2, 1, 3, 2.0)?, [0.0, 0.4, 0.8, 2.0, 4.0, 8.0]);
    Ok(())
  }

  #[test]
  fn negative_exposure_follows_the_curve() -> crate::Result<()> {
    let ev = -1.2;
    let slope = (ev as f32).exp2();
    let curve = ToneCurve::negative_exposure(ev).expect("curve");
    // Up to 0.25 by the full amount
    for value in [0.001, 0.05, 0.1, 0.18, 0.25] {
      let output = applied(&[value], 1, 1, 1, ev)?[0];
      assert!((output - value * slope).abs() < 1e-5, "{}: {}", value, output);
    }
    // Above by less: 0.83 EV at 0.5 and 0.38 EV at 0.75
    for (value, drop) in [(0.5, 0.83), (0.75, 0.38)] {
      let output = applied(&[value], 1, 1, 1, ev)?[0];
      assert!((output - curve.eval(value)).abs() < 1e-5, "{}: {}", value, output);
      assert!(((value / output).log2() - drop).abs() < 0.01, "{}: {}", value, output);
    }
    Ok(())
  }

  #[test]
  fn negative_exposure_keeps_white_and_black() -> crate::Result<()> {
    let input = [0.0, 1.0, 1.5, 100.0];
    assert_eq!(applied(&input, 4, 1, 1, -2.0)?, input);
    // The white balance decides about white
    let mut color = [1.0, 0.5, 0.5];
    apply_exposure(ImageRegionMut::from(ImageViewMut::new(&mut color, 1, 1, 3)), -2.0, &[1.0, 2.0, 2.0, f32::NAN])?;
    assert_eq!(color, [1.0, 0.5, 0.5]);
    Ok(())
  }

  #[test]
  fn all_color_planes_get_the_same_factor() -> crate::Result<()> {
    for ev in [-1.2, 1.5] {
      let input: Vec<f32> = (0..64)
        .flat_map(|i| [0.01 + 0.012 * i as f32, 0.02 + 0.008 * i as f32, 0.015 + 0.005 * i as f32])
        .collect();
      let output = applied(&input, 64, 1, 3, ev)?;
      assert_ne!(input, output);
      for (before, after) in input.chunks_exact(3).zip(output.chunks_exact(3)) {
        let factor = after[0] / before[0];
        assert!((after[1] / before[1] - factor).abs() < 1e-5, "{:?} {:?}", before, after);
        assert!((after[2] / before[2] - factor).abs() < 1e-5, "{:?} {:?}", before, after);
      }
    }
    Ok(())
  }

  #[test]
  fn no_exposure_changes_nothing() -> crate::Result<()> {
    let input = gradient(0.0, 1.5);
    for ev in [0.0, -0.0, f32::NAN] {
      assert_eq!(applied(&input, 256, 256, 1, ev)?, input, "{}", ev);
    }
    Ok(())
  }

  #[test]
  fn very_large_exposures() -> crate::Result<()> {
    let input = [0.0, 1e-5, 0.5, 1.0, 1.5];
    // Far above the largest factor: all values are finite and black stays black
    let output = applied(&input, 5, 1, 1, 1000.0)?;
    assert_eq!(output[0], 0.0);
    assert!(output.iter().all(|v| v.is_finite()), "{:?}", output);
    assert!(output[1..].iter().all(|v| *v > 1.0), "{:?}", output);
    // All values below white get black
    let output = applied(&input, 5, 1, 1, -1000.0)?;
    assert!(output.iter().all(|v| v.is_finite()), "{:?}", output);
    assert_eq!(output[..2], [0.0, 0.0]);
    assert_eq!(output[3..], [1.0, 1.5]);
    assert!((0.0..1.0).contains(&output[2]), "{:?}", output);
    Ok(())
  }

  #[test]
  fn region_leaves_other_pixels_untouched() -> crate::Result<()> {
    let (width, height) = (16, 12);
    let area = Rect::new(Point::new(4, 3), Dim2::new(8, 6));
    let input: Vec<f32> = (0..width * height * 3).map(|i| 0.1 + 0.001 * i as f32).collect();
    for ev in [-1.0, 1.0] {
      let mut output = input.clone();
      apply_exposure(ImageRegionMut::new(&mut output, width, height, 3, area), ev, &NEUTRAL)?;
      for y in 0..height {
        for x in 0..width {
          let pixel = (y * width + x) * 3;
          let inside = (area.p.x..area.p.x + area.d.w).contains(&x) && (area.p.y..area.p.y + area.d.h).contains(&y);
          assert_eq!(output[pixel..pixel + 3] != input[pixel..pixel + 3], inside, "{} {} {}", ev, x, y);
        }
      }
    }
    Ok(())
  }
}
