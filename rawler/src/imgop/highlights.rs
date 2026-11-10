// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Highlights above white
//!
//! A positive BaselineExposure is applied as linear gain. Values above 1.0
//! after it are highlights that the file holds, but the output can't show.
//! DNG readers clip each channel at 1.0: colored highlights turn white and
//! lose their gradation. [`HighlightShoulder`] compresses them into the range
//! below white instead.

use crate::dng::profile_render::map_preserving_hue;

/// Values up to the knee are unchanged, the shoulder starts here
const KNEE: f32 = 0.75;

/// Treatment of values above white after a positive BaselineExposure
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HighlightMode {
  /// Compress them by a [`HighlightShoulder`], which keeps their hue
  #[default]
  Compress,
  /// Clip each channel at white, like DNG readers
  Clip,
}

#[cfg(feature = "clap")]
impl clap::ValueEnum for HighlightMode {
  fn value_variants<'a>() -> &'a [Self] {
    &[Self::Compress, Self::Clip]
  }

  fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
    Some(match self {
      Self::Compress => clap::builder::PossibleValue::new("compress"),
      Self::Clip => clap::builder::PossibleValue::new("clip"),
    })
  }
}

/// Smooth shoulder that maps linear values from 0.0 up to `white` to 0.0 ..= 1.0
///
/// Values up to 0.75 are unchanged. Above, the curve starts with a slope of
/// 1.0 and flattens until it reaches 1.0 at `white` (an extended Reinhard
/// curve). Values above `white` are clipped to 1.0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HighlightShoulder {
  white: f32,
}

impl HighlightShoulder {
  /// Shoulder for values up to `white`, `None` if `white` is 1.0 or less:
  /// clipping at 1.0 loses nothing then
  pub fn new(white: f32) -> Option<Self> {
    (white > 1.0 && white.is_finite()).then_some(Self { white })
  }

  /// Map one value
  pub fn eval(&self, x: f32) -> f32 {
    if x <= KNEE {
      return x.max(0.0);
    }
    if x >= self.white {
      return 1.0;
    }
    let range = 1.0 - KNEE;
    let t = (x - KNEE) / range;
    let end = (self.white - KNEE) / range;
    (KNEE + range * t * (1.0 + t / (end * end)) / (1.0 + t)).min(1.0)
  }

  /// Map the largest and smallest channel of `rgb`, the middle channel keeps
  /// its relative position between them, so the hue doesn't change
  pub fn apply_rgb(&self, rgb: [f32; 3]) -> [f32; 3] {
    map_preserving_hue(rgb, |c| self.eval(c))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn no_shoulder_without_headroom() {
    assert!(HighlightShoulder::new(1.0).is_none());
    assert!(HighlightShoulder::new(0.5).is_none());
    assert!(HighlightShoulder::new(f32::INFINITY).is_none());
  }

  #[test]
  fn shoulder_curve() {
    for white in [1.01, 1.5, 2.0, 5.0, 64.0] {
      let shoulder = HighlightShoulder::new(white).expect("shoulder");
      // Unchanged below the knee
      for x in [0.0, 0.1, 0.5, KNEE] {
        assert_eq!(shoulder.eval(x), x);
      }
      assert_eq!(shoulder.eval(-0.5), 0.0);
      // White is reached at `white`
      assert!((shoulder.eval(white * 0.9999) - 1.0).abs() < 1e-3, "white {}", white);
      assert_eq!(shoulder.eval(white), 1.0);
      assert_eq!(shoulder.eval(white * 2.0), 1.0);
      // The slope is 1.0 at the knee
      let slope = (shoulder.eval(KNEE + 1e-3) - KNEE) / 1e-3;
      assert!((slope - 1.0).abs() < 0.02, "white {}: slope {}", white, slope);
      let mut last = 0.0;
      for i in 0..=1000 {
        let y = shoulder.eval(white * i as f32 / 1000.0);
        assert!(y >= last && y <= 1.0, "white {}: {} after {}", white, y, last);
        last = y;
      }
    }
  }

  #[test]
  fn shoulder_keeps_hue() {
    let shoulder = HighlightShoulder::new(4.0).expect("shoulder");
    let [r, g, b] = shoulder.apply_rgb([4.0, 1.0, 0.2]);
    assert_eq!([r, b], [1.0, 0.2]);
    assert!(g < 1.0 && g > b, "{}", g);
    assert!(((g - b) / (r - b) - 0.8 / 3.8).abs() < 1e-6, "{}", g);
    // Neutral values stay neutral
    let gray = shoulder.apply_rgb([2.0; 3]);
    assert!(gray[0] < 1.0 && gray.iter().all(|c| *c == gray[0]), "{:?}", gray);
  }
}
