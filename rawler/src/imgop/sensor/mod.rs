// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use crate::{
  CFA,
  cfa::PlaneColor,
  imgop::Rect,
  pixarray::{Color2D, Pix2D, SubPixel},
};

pub mod bayer;
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
  /// Demosaic the given mosaic `pixels` within the specified `roi`.
  ///
  /// # Parameters
  /// - `pixels`: Single-channel mosaic pixel data from the sensor.
  /// - `cfa`: The color filter array pattern describing the sensor layout.
  /// - `colors`: Mapping from CFA plane indices to color channel indices.
  /// - `roi`: The region of interest to demosaic within `pixels`.
  ///
  /// # Returns
  /// An `N`-channel color image covering the requested ROI.
  fn demosaic(&self, pixels: &Pix2D<T>, cfa: &CFA, colors: &PlaneColor, roi: Rect) -> Color2D<T, N>;
}
