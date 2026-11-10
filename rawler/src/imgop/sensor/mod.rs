// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use crate::{
  CFA,
  cfa::PlaneColor,
  pixarray::{Color2D, ImageRegion, SubPixel},
};

pub mod bayer;
pub mod border;
pub mod lab;
pub mod tiles;
pub mod vng;
pub mod xtrans;

/// Identifies the type of color filter array (CFA) sensor.
///
/// Digital camera sensors use a mosaic of color filters over each photosite.
/// The two most common patterns are:
/// - **Bayer**: A 2x2 repeating pattern (RGGB, BGGR, etc.) used by most manufacturers.
/// - **X-Trans**: A 6x6 repeating pattern used by Fujifilm, designed to reduce moiré
///   without an optical low-pass filter.
#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Clone, Copy)]
pub enum SensorType {
  /// Standard 2x2 Bayer CFA (e.g. RGGB, BGGR, GBRG, GRBG).
  Bayer,
  /// Fujifilm 6x6 X-Trans CFA.
  Xtrans,
}

impl SensorType {
  /// Infer the sensor type from a [`CFA`] pattern based on its dimensions.
  ///
  /// - 2x2 with at least 3 unique colors → [`SensorType::Bayer`]
  /// - 6x6 → [`SensorType::Xtrans`]
  ///
  /// # Panics
  /// Panics (via `unimplemented!()`) for CFA patterns that do not match
  /// either a Bayer or X-Trans layout.
  pub fn from_cfa(cfa: &CFA) -> Self {
    if cfa.width == 2 && cfa.height == 2 && cfa.unique_colors() >= 3 {
      Self::Bayer
    } else if cfa.width == 6 && cfa.height == 6 {
      Self::Xtrans
    } else {
      unimplemented!()
    }
  }
}

/// Trait for demosaicing algorithms that reconstruct a multi-channel color image
/// from single-channel mosaic sensor data.
///
/// # Type Parameters
/// - `T`: The pixel sample type (e.g. `f32`, `u16`), must implement [`SubPixel`].
/// - `N`: The number of output color channels (typically 3 for RGB).
pub trait Demosaic<T: SubPixel, const N: usize> {
  /// Demosaic the mosaic data of the region `pixels`.
  ///
  /// # Parameters
  /// - `pixels`: Region of single-channel mosaic pixel data from the sensor,
  ///   e.g. `pix.region(roi)` of a [`Pix2D`](crate::pixarray::Pix2D), or
  ///   `pix.view().into()` for the whole image. The CFA pattern is aligned to
  ///   the top-left pixel of the image the region is taken from.
  /// - `cfa`: The color filter array pattern describing the sensor layout.
  /// - `colors`: Mapping from CFA plane indices to color channel indices.
  ///
  /// # Returns
  /// An `N`-channel color image covering the region.
  ///
  /// # Panics
  /// Panics if `pixels` has more than one channel.
  fn demosaic(&self, pixels: ImageRegion<'_, T>, cfa: &CFA, colors: &PlaneColor) -> Color2D<T, N>;
}

#[cfg(test)]
mod tests {
  use super::{
    bayer::{
      ahd::AhdDemosaic,
      bilinear::Bilinear4Channel,
      ppg::PPGDemosaic,
      rcd::RcdDemosaic,
      superpixel::{Superpixel3Channel, Superpixel4Channel},
    },
    vng::VngDemosaic,
    xtrans::{bilinear::XTransBilinearDemosaic, markesteijn::XTransMarkesteijnDemosaic},
    *,
  };
  use crate::{
    imgop::{Dim2, Point, Rect},
    pixarray::ImageView,
  };

  const XTRANS: &str = "GGRGGBGGBGGRBRGRBGGGBGGRGGRGGBRBGBRG";

  /// Demosaic `area` of a mosaic by `algorithm` as a region of the image and
  /// from a copy of the area with the CFA shifted to it, both must be equal
  fn assert_region_equals_copy<const N: usize>(algorithm: &dyn Demosaic<f32, N>, pattern: &str, colors: &str, area: Rect) {
    let (width, height) = (96, 72);
    let samples: Vec<f32> = (0..width * height).map(|i| (i * 7919 % 1000) as f32 / 1000.0).collect();
    let (cfa, colors) = (CFA::new(pattern), PlaneColor::new(colors));
    let region = ImageView::new(&samples, width, height, 1).region(area);
    let copy = region.to_vec();
    let copy = ImageView::new(&copy, area.width(), area.height(), 1);
    let expected = algorithm.demosaic(copy.into(), &cfa.shift(area.p.x, area.p.y), &colors);
    let demosaiced = algorithm.demosaic(region, &cfa, &colors);
    assert_eq!(
      (demosaiced.width, demosaiced.height),
      (expected.width, expected.height),
      "{} {:?}",
      pattern,
      area
    );
    assert!(demosaiced.data == expected.data, "{} {:?}", pattern, area);
  }

  #[test]
  fn demosaic_region() {
    // A band of full rows is demosaiced in place, other areas are copied
    for area in [Rect::new(Point::new(0, 6), Dim2::new(96, 48)), Rect::new(Point::new(5, 3), Dim2::new(70, 50))] {
      let rgb: [&dyn Demosaic<f32, 3>; 5] = [
        &PPGDemosaic::new(),
        &AhdDemosaic::new(),
        &VngDemosaic::new(),
        &RcdDemosaic::new(),
        &Superpixel3Channel::new(),
      ];
      for algorithm in rgb {
        assert_region_equals_copy(algorithm, "RGGB", "RGB", area);
      }
      let xtrans: [&dyn Demosaic<f32, 3>; 3] = [&XTransBilinearDemosaic::new(), &XTransMarkesteijnDemosaic::new_pass_3(), &VngDemosaic::new()];
      for algorithm in xtrans {
        assert_region_equals_copy(algorithm, XTRANS, "RGB", area);
      }
      let cygm: [&dyn Demosaic<f32, 4>; 3] = [&Bilinear4Channel::new(), &Superpixel4Channel::new(), &VngDemosaic::new()];
      for algorithm in cygm {
        assert_region_equals_copy(algorithm, "CYGM", "CYGM", area);
      }
    }
  }

  #[test]
  fn ppg_small_images() {
    // PPG interpolates the pixels 3 pixels away from the borders, the borders
    // bilinear. Images up to 6 pixels high or wide are border only.
    for pattern in ["RGGB", "BGGR", "GRBG", "GBRG"] {
      let cfa = CFA::new(pattern);
      for (width, height) in (1..=9).flat_map(|width| (1..=9).map(move |height| (width, height))) {
        let samples: Vec<f32> = (0..width * height).map(|i| (i * 7919 % 1000) as f32 / 1000.0).collect();
        let rgb = PPGDemosaic::new().demosaic(ImageView::new(&samples, width, height, 1).into(), &cfa, &PlaneColor::new("RGB"));
        assert_eq!((rgb.width, rgb.height), (width, height), "{}", pattern);
        for (row, col) in (0..height).flat_map(|row| (0..width).map(move |col| (row, col))) {
          let (pixel, own) = (rgb.at(row, col), cfa.color_at(row, col));
          assert_eq!(pixel[own], samples[row * width + col], "{} {}x{} at {}:{}", pattern, width, height, col, row);
          if width > 6 && height > 6 {
            continue;
          }
          // Mean of the samples of each color around the pixel, 0 if there is none
          let mut sum = [(0.0, 0); 3];
          for y in row.saturating_sub(1)..(row + 2).min(height) {
            for x in col.saturating_sub(1)..(col + 2).min(width) {
              let color = cfa.color_at(y, x);
              sum[color] = (sum[color].0 + samples[y * width + x], sum[color].1 + 1);
            }
          }
          for color in (0..3).filter(|color| *color != own) {
            let expected = if sum[color].1 > 0 { sum[color].0 / sum[color].1 as f32 } else { 0.0 };
            assert_eq!(pixel[color], expected, "{} {}x{} at {}:{}", pattern, width, height, col, row);
          }
        }
      }
    }
  }
}
