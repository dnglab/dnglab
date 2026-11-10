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
  /// # Errors
  /// Fails for data the algorithm can't process: `pixels` with more than one
  /// channel, `colors` without `N` planes or with other colors than `cfa`, or
  /// a CFA pattern of another layout than the algorithm needs.
  fn demosaic(&self, pixels: ImageRegion<'_, T>, cfa: &CFA, colors: &PlaneColor) -> crate::Result<Color2D<T, N>>;
}

/// Check the mosaic data and colors of a demosaic with `N` planes by
/// `algorithm`: `pixels` has one channel, `colors` has `N` planes, each color
/// of the CFA pattern has a plane and each plane occurs in the pattern
pub(crate) fn check_mosaic<const N: usize, T>(algorithm: &str, pixels: &ImageRegion<'_, T>, cfa: &CFA, colors: &PlaneColor) -> crate::Result<()> {
  if pixels.channels() != 1 {
    return Err(format!("{}: mosaic data must have one channel, not {}", algorithm, pixels.channels()).into());
  }
  if colors.plane_count() != N {
    return Err(format!("{} with {} planes needs {} color planes, not {}", algorithm, N, N, colors.plane_count()).into());
  }
  let plane_map = colors.plane_lookup_table();
  let pattern: Vec<usize> = (0..cfa.height).flat_map(|row| (0..cfa.width).map(move |col| cfa.color_at(row, col))).collect();
  if pattern.iter().any(|&color| plane_map[color] >= N) {
    return Err(format!("{}: a color of CFA pattern '{}' has no plane in {:?}", algorithm, cfa, colors.colors).into());
  }
  if let Some(color) = colors.colors.iter().find(|color| !pattern.contains(&(**color as usize))) {
    return Err(format!("{}: plane color {:?} doesn't occur in CFA pattern '{}'", algorithm, color, cfa).into());
  }
  Ok(())
}

/// Check that `cfa` is a 2x2 pattern for `algorithm`
pub(crate) fn check_2x2(algorithm: &str, cfa: &CFA) -> crate::Result<()> {
  if cfa.width != 2 || cfa.height != 2 {
    return Err(format!("{} needs a 2x2 CFA pattern, not '{}'", algorithm, cfa).into());
  }
  Ok(())
}

/// Check that `cfa` is a 2x2 RGB pattern (Bayer RGB) for `algorithm`
pub(crate) fn check_bayer_rgb(algorithm: &str, cfa: &CFA) -> crate::Result<()> {
  if cfa.width != 2 || cfa.height != 2 || !cfa.is_rgb() {
    return Err(format!("{} needs a 2x2 RGB CFA pattern, not '{}'", algorithm, cfa).into());
  }
  Ok(())
}

/// Check that `cfa` is a 6x6 RGB pattern (X-Trans) for `algorithm`
pub(crate) fn check_xtrans(algorithm: &str, cfa: &CFA) -> crate::Result<()> {
  if cfa.width != 6 || cfa.height != 6 || !cfa.is_rgb() {
    return Err(format!("{} needs a 6x6 RGB CFA pattern, not '{}'", algorithm, cfa).into());
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::{
    bayer::{
      ahd::AhdDemosaic,
      bilinear::BilinearDemosaic,
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
    let expected = algorithm.demosaic(copy.into(), &cfa.shift(area.p.x, area.p.y), &colors).unwrap();
    let demosaiced = algorithm.demosaic(region, &cfa, &colors).unwrap();
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
      let rgb: [&dyn Demosaic<f32, 3>; 6] = [
        &BilinearDemosaic::new(),
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
      let cygm: [&dyn Demosaic<f32, 4>; 3] = [&BilinearDemosaic::new(), &Superpixel4Channel::new(), &VngDemosaic::new()];
      for algorithm in cygm {
        assert_region_equals_copy(algorithm, "CYGM", "CYGM", area);
      }
    }
  }

  /// Algorithms with 3 planes for Bayer RGB data, and if their result has half
  /// the size
  fn bayer_rgb_algorithms() -> [(Box<dyn Demosaic<f32, 3>>, bool); 6] {
    [
      (Box::new(BilinearDemosaic::new()), false),
      (Box::new(PPGDemosaic::new()), false),
      (Box::new(AhdDemosaic::new()), false),
      (Box::new(VngDemosaic::new()), false),
      (Box::new(RcdDemosaic::new()), false),
      (Box::new(Superpixel3Channel::new()), true),
    ]
  }

  /// Algorithms with 3 planes for X-Trans data
  fn xtrans_algorithms() -> [Box<dyn Demosaic<f32, 3>>; 4] {
    [
      Box::new(XTransBilinearDemosaic::new()),
      Box::new(XTransMarkesteijnDemosaic::new_pass_1()),
      Box::new(XTransMarkesteijnDemosaic::new_pass_3()),
      Box::new(VngDemosaic::new()),
    ]
  }

  /// Algorithms with 4 planes for 4-color Bayer data, and if their result has
  /// half the size
  fn four_color_algorithms() -> [(Box<dyn Demosaic<f32, 4>>, bool); 3] {
    [
      (Box::new(BilinearDemosaic::new()), false),
      (Box::new(Superpixel4Channel::new()), true),
      (Box::new(VngDemosaic::new()), false),
    ]
  }

  #[test]
  fn all_sizes_work() {
    for (width, height) in (0..=9).flat_map(|width| (0..=9).map(move |height| (width, height))) {
      let samples: Vec<f32> = (0..width * height).map(|i| (i * 7919 % 1000) as f32 / 1000.0).collect();
      let view = ImageView::new(&samples, width, height, 1);
      let size = |half: bool| if half { (width / 2, height / 2) } else { (width, height) };
      let rgb = PlaneColor::new("RGB");
      for (i, (algorithm, half)) in bayer_rgb_algorithms().into_iter().enumerate() {
        let out = algorithm.demosaic(view.into(), &CFA::new("GBRG"), &rgb).expect("Bayer RGB");
        assert_eq!((out.width, out.height), size(half), "Bayer algorithm {} {}x{}", i, width, height);
      }
      for (i, algorithm) in xtrans_algorithms().into_iter().enumerate() {
        let out = algorithm.demosaic(view.into(), &CFA::new(XTRANS), &rgb).expect("X-Trans");
        assert_eq!((out.width, out.height), size(false), "X-Trans algorithm {} {}x{}", i, width, height);
      }
      for (i, (algorithm, half)) in four_color_algorithms().into_iter().enumerate() {
        let out = algorithm.demosaic(view.into(), &CFA::new("CYGM"), &PlaneColor::new("CYGM")).expect("4 colors");
        assert_eq!((out.width, out.height), size(half), "4-color algorithm {} {}x{}", i, width, height);
      }
    }
  }

  #[test]
  fn invalid_input_is_an_error() {
    let (width, height) = (24, 18);
    let samples: Vec<f32> = (0..width * height * 3).map(|i| (i * 7919 % 1000) as f32 / 1000.0).collect();
    let mosaic = ImageView::new(&samples[..width * height], width, height, 1);
    let three_channels = ImageView::new(&samples, width, height, 3);
    let (rgb, cygm) = (PlaneColor::new("RGB"), PlaneColor::new("CYGM"));
    for (i, (algorithm, _)) in bayer_rgb_algorithms().into_iter().enumerate() {
      let vng = i == 3;
      // X-Trans data for the Bayer algorithms, VNG works for any CFA
      assert_eq!(
        algorithm.demosaic(mosaic.into(), &CFA::new(XTRANS), &rgb).is_err(),
        !vng,
        "Bayer algorithm {}",
        i
      );
      assert!(algorithm.demosaic(mosaic.into(), &CFA::new("RGGB"), &cygm).is_err(), "Bayer algorithm {}", i);
      assert!(
        algorithm.demosaic(three_channels.into(), &CFA::new("RGGB"), &rgb).is_err(),
        "Bayer algorithm {}",
        i
      );
      // Blue has no plane
      assert!(
        algorithm.demosaic(mosaic.into(), &CFA::new("RGGB"), &PlaneColor::new("RGE")).is_err(),
        "Bayer algorithm {}",
        i
      );
    }
    for (i, algorithm) in xtrans_algorithms().into_iter().enumerate() {
      let (bilinear, vng) = (i == 0, i == 3);
      // Bayer data for Markesteijn, the X-Trans bilinear and VNG work for any RGB CFA
      assert_eq!(
        algorithm.demosaic(mosaic.into(), &CFA::new("RGGB"), &rgb).is_err(),
        !bilinear && !vng,
        "X-Trans algorithm {}",
        i
      );
      assert!(algorithm.demosaic(mosaic.into(), &CFA::new(XTRANS), &cygm).is_err(), "X-Trans algorithm {}", i);
      assert!(
        algorithm.demosaic(three_channels.into(), &CFA::new(XTRANS), &rgb).is_err(),
        "X-Trans algorithm {}",
        i
      );
    }
    for (i, (algorithm, _)) in four_color_algorithms().into_iter().enumerate() {
      assert!(algorithm.demosaic(mosaic.into(), &CFA::new("CYGM"), &rgb).is_err(), "4-color algorithm {}", i);
      assert!(
        algorithm.demosaic(three_channels.into(), &CFA::new("CYGM"), &cygm).is_err(),
        "4-color algorithm {}",
        i
      );
      // CYGM data with the planes of RGBE
      assert!(
        algorithm.demosaic(mosaic.into(), &CFA::new("CYGM"), &PlaneColor::new("RGBE")).is_err(),
        "4-color algorithm {}",
        i
      );
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
        let rgb = PPGDemosaic::new()
          .demosaic(ImageView::new(&samples, width, height, 1).into(), &cfa, &PlaneColor::new("RGB"))
          .unwrap();
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
