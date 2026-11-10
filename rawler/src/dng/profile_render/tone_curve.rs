// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! ProfileToneCurve

use rayon::prelude::*;

use super::{decode_overrange, encode_overrange};
use crate::pixarray::ImageViewMut;

/// Number of intervals of the lookup table
const TABLE_SIZE: usize = 4096;

/// Points of the default curve, pairs of linear input and output values
///
/// An S-curve for images that have no curve of their own. It adds contrast around
/// a fixed point at about 0.116 and keeps the brightness of the image: shadows get
/// darker and highlights brighter. Brightness is a matter of the exposure.
///
/// The points are fitted to the tone map of the camera JPEGs of 65 files without
/// its brightening of about 0.54 EV. They keep the median of the mean encoded
/// brightness of these files.
const DEFAULT_RENDER_POINTS: [f32; 12] = [0.0, 0.0, 0.02, 0.012, 0.08, 0.073, 0.25, 0.291, 0.55, 0.655, 1.0, 1.0];

/// Tone curve of a camera profile, sampled into a lookup table
///
/// The curve points are connected by a natural cubic spline: the curve is
/// continuous in its first and second derivative and the second derivative
/// is zero at both end points.
#[derive(Debug, Clone, PartialEq)]
pub struct ToneCurve {
  table: Vec<f32>,
}

impl ToneCurve {
  /// Build a curve from pairs of input and output values in 0.0 ..= 1.0
  pub fn new(points: &[f32]) -> Option<Self> {
    let (x, y): (Vec<f64>, Vec<f64>) = points.as_chunks::<2>().0.iter().map(|[x, y]| (*x as f64, *y as f64)).unzip();
    if !points.len().is_multiple_of(2) || x.len() < 2 || x.windows(2).any(|pair| pair[1] <= pair[0]) || !x.iter().chain(&y).all(|v| v.is_finite()) {
      log::warn!("Invalid tone curve with {} values, ignoring it", points.len());
      return None;
    }
    let second_derivatives = natural_spline(&x, &y);
    let mut segment = 0;
    Some(Self::from_fn(|t| {
      if t <= x[0] {
        return y[0];
      }
      if t >= x[x.len() - 1] {
        return y[y.len() - 1];
      }
      while t > x[segment + 1] {
        segment += 1;
      }
      let (x0, x1, m0, m1) = (x[segment], x[segment + 1], second_derivatives[segment], second_derivatives[segment + 1]);
      let h = x1 - x0;
      let (a, b) = (x1 - t, t - x0);
      (m0 * a.powi(3) + m1 * b.powi(3)) / (6.0 * h) + (y[segment] / h - m0 * h / 6.0) * a + (y[segment + 1] / h - m1 * h / 6.0) * b
    }))
  }

  /// Default curve for images that have no curve of their own
  pub fn default_render() -> Self {
    Self::new(&DEFAULT_RENDER_POINTS).expect("Valid curve points")
  }

  /// Curve of a negative exposure compensation of `ev` EV, `None` for `ev >= 0`
  ///
  /// Values up to 0.25 are scaled by 2^ev. Above, a quadratic curve with the
  /// same value and slope at 0.25 maps 1.0 to 1.0, so white stays white
  /// instead of being darkened like the rest of the image.
  pub fn negative_exposure(ev: f32) -> Option<Self> {
    if ev.is_nan() || ev >= 0.0 {
      return None;
    }
    let slope = (ev as f64).exp2();
    let a = 16.0 / 9.0 * (1.0 - slope);
    let b = slope - 0.5 * a;
    let c = 1.0 - a - b;
    Some(Self::from_fn(|x| if x <= 0.25 { x * slope } else { (a * x + b) * x + c }))
  }

  /// `curve` applied to the result of a negative exposure compensation of `ev` EV,
  /// `None` if there is neither a curve nor a negative exposure
  pub fn with_exposure(ev: f32, curve: Option<Self>) -> Option<Self> {
    match (Self::negative_exposure(ev), curve) {
      (Some(exposure_curve), Some(curve)) => Some(curve.after(&exposure_curve)),
      (exposure_curve, curve) => exposure_curve.or(curve),
    }
  }

  /// This curve applied to the result of `first`
  pub fn after(&self, first: &ToneCurve) -> Self {
    Self::from_fn(|x| self.eval(first.eval(x as f32)) as f64)
  }

  /// Sample `curve` into the lookup table, it is called for increasing values in 0.0 ..= 1.0
  fn from_fn(mut curve: impl FnMut(f64) -> f64) -> Self {
    Self {
      table: (0..=TABLE_SIZE).map(|i| curve(i as f64 / TABLE_SIZE as f64) as f32).collect(),
    }
  }

  /// Curve value of `x`, limited to 0.0 ..= 1.0
  pub fn eval(&self, x: f32) -> f32 {
    let scaled = x.clamp(0.0, 1.0) * TABLE_SIZE as f32;
    let index = (scaled as usize).min(TABLE_SIZE - 1);
    let fract = scaled - index as f32;
    self.table[index] + (self.table[index + 1] - self.table[index]) * fract
  }

  /// Curve value of `x`, values above 1.0 follow the line through the end
  /// point of the curve with the slope of the curve there
  fn eval_extended(&self, x: f32) -> f32 {
    if x > 1.0 {
      let end = self.table[TABLE_SIZE];
      let slope = (end - self.table[TABLE_SIZE - 1]) * TABLE_SIZE as f32;
      end + slope * (x - 1.0)
    } else {
      self.eval(x)
    }
  }

  /// Apply the curve to RGB values, preserving the hue
  ///
  /// The largest and smallest channel are mapped by the curve, the middle
  /// channel keeps its relative position between them.
  pub fn apply_rgb(&self, rgb: [f32; 3]) -> [f32; 3] {
    map_preserving_hue(rgb.map(|c| c.clamp(0.0, 1.0)), |c| self.eval(c))
  }

  /// Apply the curve to RGB values of a High Dynamic Range profile, preserving the hue
  ///
  /// Like [`Self::apply_rgb`], but the curve is applied to values encoded by
  /// [`encode_overrange`]. Encoded values above 1.0 (linear values above
  /// 16.0) follow the slope at the end of the curve.
  pub fn apply_rgb_overrange(&self, rgb: [f32; 3]) -> [f32; 3] {
    map_preserving_hue(rgb.map(encode_overrange), |c| self.eval_extended(c)).map(decode_overrange)
  }

  /// Apply the curve to an image: to the RGB values of an image with three
  /// channels (see [`Self::apply_rgb`]), to each sample otherwise
  pub fn apply_image<'a>(&self, image: impl Into<ImageViewMut<'a, f32>>) {
    let mut image = image.into();
    match image.as_array_slice_mut::<3>() {
      Some(pixels) => pixels.par_iter_mut().for_each(|pixel| *pixel = self.apply_rgb(*pixel)),
      None => image.as_mut_slice().par_iter_mut().for_each(|v| *v = self.eval(*v)),
    }
  }
}

/// Map the largest and smallest channel of `rgb` by `curve`, the middle
/// channel keeps its relative position between them
pub(crate) fn map_preserving_hue(rgb: [f32; 3], curve: impl Fn(f32) -> f32) -> [f32; 3] {
  let mut order = [0, 1, 2];
  order.sort_by(|a, b| rgb[*b].total_cmp(&rgb[*a]));
  let [max, mid, min] = order;
  if rgb[max] <= rgb[min] {
    return rgb.map(&curve);
  }
  let mut result = [0.0; 3];
  result[max] = curve(rgb[max]);
  result[min] = curve(rgb[min]);
  result[mid] = result[min] + (result[max] - result[min]) * (rgb[mid] - rgb[min]) / (rgb[max] - rgb[min]);
  result
}

/// Second derivatives of the natural cubic spline through the points `x`, `y`
fn natural_spline(x: &[f64], y: &[f64]) -> Vec<f64> {
  let n = x.len();
  let mut second = vec![0.0; n];
  if n < 3 {
    return second;
  }
  // Tridiagonal system for the inner points, solved by forward elimination
  let mut diagonal = vec![0.0; n];
  let mut rhs = vec![0.0; n];
  for i in 1..n - 1 {
    let (h0, h1) = (x[i] - x[i - 1], x[i + 1] - x[i]);
    diagonal[i] = 2.0 * (h0 + h1);
    rhs[i] = 6.0 * ((y[i + 1] - y[i]) / h1 - (y[i] - y[i - 1]) / h0);
    if i > 1 {
      let factor = h0 / diagonal[i - 1];
      diagonal[i] -= factor * h0;
      rhs[i] -= factor * rhs[i - 1];
    }
  }
  for i in (1..n - 1).rev() {
    let h1 = x[i + 1] - x[i];
    second[i] = (rhs[i] - h1 * second[i + 1]) / diagonal[i];
  }
  second
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn curve_through_points() {
    let points = [0.0, 0.0, 0.25, 0.15, 0.5, 0.55, 0.75, 0.85, 1.0, 1.0];
    let curve = ToneCurve::new(&points).expect("curve");
    for point in points.chunks_exact(2) {
      assert!((curve.eval(point[0]) - point[1]).abs() < 1e-5, "{:?}", point);
    }
    // Monotonic between the points
    assert!((1..=100).all(|i| curve.eval(i as f32 / 100.0) >= curve.eval((i - 1) as f32 / 100.0)));
    assert_eq!(curve.eval(-1.0), 0.0);
    assert_eq!(curve.eval(2.0), 1.0);
  }

  #[test]
  fn natural_spline_of_line() {
    // A natural spline through points of a line is the line itself
    let curve = ToneCurve::new(&[0.0, 0.0, 0.5, 0.5, 1.0, 1.0]).expect("curve");
    for i in 0..=10 {
      let x = i as f32 / 10.0;
      assert!((curve.eval(x) - x).abs() < 1e-6);
    }
  }

  #[test]
  fn rgb_tone_preserves_hue() {
    let curve = ToneCurve::new(&[0.0, 0.0, 0.25, 0.15, 0.75, 0.85, 1.0, 1.0]).expect("curve");
    let rgb = [0.2, 0.6, 0.4];
    let toned = curve.apply_rgb(rgb);
    assert_eq!(toned[1], curve.eval(0.6));
    assert_eq!(toned[0], curve.eval(0.2));
    // The middle channel keeps its relative position
    assert!(((toned[2] - toned[0]) / (toned[1] - toned[0]) - 0.5).abs() < 1e-5);
    assert_eq!(curve.apply_rgb([0.3; 3]), [curve.eval(0.3); 3]);
    assert_eq!(curve.apply_rgb([1.5, -0.2, 0.5])[0], 1.0);
  }

  #[test]
  fn negative_exposure_keeps_white() {
    let curve = ToneCurve::negative_exposure(-1.0).expect("curve");
    // Linear up to 0.25
    for x in [0.0, 0.1, 0.2, 0.25] {
      assert!((curve.eval(x) - x / 2.0).abs() < 1e-5, "{}", x);
    }
    // Same value and slope above 0.25, white stays white
    let slope_above = (curve.eval(0.2501) - curve.eval(0.25)) / 0.0001;
    assert!((slope_above - 0.5).abs() < 0.01, "{}", slope_above);
    assert!((curve.eval(1.0) - 1.0).abs() < 1e-6);
    assert!((1..=1000).all(|i| curve.eval(i as f32 / 1000.0) > curve.eval((i - 1) as f32 / 1000.0)));
    for ev in [0.0, 0.5, f32::NAN] {
      assert!(ToneCurve::negative_exposure(ev).is_none());
    }
  }

  #[test]
  fn curve_composition() {
    let exposure = ToneCurve::negative_exposure(-1.0).expect("curve");
    let identity = ToneCurve::new(&[0.0, 0.0, 1.0, 1.0]).expect("curve");
    let lift = ToneCurve::new(&[0.0, 0.0, 0.5, 0.75, 1.0, 1.0]).expect("curve");
    for i in 0..=20 {
      let x = i as f32 / 20.0;
      assert!((identity.after(&exposure).eval(x) - exposure.eval(x)).abs() < 1e-5);
      assert!((exposure.after(&identity).eval(x) - exposure.eval(x)).abs() < 1e-5);
      assert!((lift.after(&exposure).eval(x) - lift.eval(exposure.eval(x))).abs() < 1e-4);
    }
  }

  #[test]
  fn default_render_curve() {
    let curve = ToneCurve::default_render();
    for point in DEFAULT_RENDER_POINTS.chunks_exact(2) {
      assert!((curve.eval(point[0]) - point[1]).abs() < 1e-5, "{:?}", point);
    }
    // Strictly increasing and limited to the output range, with no overshoot
    assert!(curve.table.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!((curve.table[0], curve.table[TABLE_SIZE]), (0.0, 1.0));
    // More contrast around a fixed point: darker shadows and brighter highlights
    assert!((curve.eval(0.116) - 0.116).abs() < 0.002, "{}", curve.eval(0.116));
    for x in [0.01, 0.02, 0.05, 0.08] {
      assert!(curve.eval(x) < x, "{}", x);
    }
    for x in [0.15, 0.25, 0.55, 0.9] {
      assert!(curve.eval(x) > x, "{}", x);
    }
    assert!(curve.eval(0.15) - curve.eval(0.1) > 0.15 - 0.1);
  }

  #[test]
  fn curve_with_exposure() {
    let lift = ToneCurve::new(&[0.0, 0.0, 0.5, 0.75, 1.0, 1.0]).expect("curve");
    let exposure = ToneCurve::negative_exposure(-1.0).expect("curve");
    assert!(ToneCurve::with_exposure(0.0, None).is_none());
    assert_eq!(ToneCurve::with_exposure(1.0, Some(lift.clone())), Some(lift.clone()));
    assert_eq!(ToneCurve::with_exposure(-1.0, None), Some(exposure.clone()));
    assert_eq!(ToneCurve::with_exposure(-1.0, Some(lift.clone())), Some(lift.after(&exposure)));
  }

  #[test]
  fn invalid_curves() {
    assert!(ToneCurve::new(&[0.0, 0.0]).is_none());
    assert!(ToneCurve::new(&[0.0, 0.0, 1.0]).is_none());
    assert!(ToneCurve::new(&[0.0, 0.0, 0.5, 0.5, 0.5, 0.6, 1.0, 1.0]).is_none());
    assert!(ToneCurve::new(&[0.0, 0.0, f32::NAN, 0.5, 1.0, 1.0]).is_none());
  }

  #[test]
  fn overrange_curve() {
    let assert_close = |result: [f32; 3], expected: [f32; 3]| {
      for (a, b) in result.iter().zip(expected) {
        assert!((a - b).abs() <= 1e-4 * b.max(1.0), "{:?} != {:?}", result, expected);
      }
    };
    // The identity curve keeps values, also above 16.0 (encoded above 1.0)
    let identity = ToneCurve::new(&[0.0, 0.0, 1.0, 1.0]).expect("curve");
    for rgb in [[4.0, 2.0, 0.5], [40.0, 20.0, 0.1]] {
      assert_close(identity.apply_rgb_overrange(rgb), rgb);
    }
    // The curve is applied to encoded values
    let lift = ToneCurve::new(&[0.0, 0.0, 0.5, 0.75, 1.0, 1.0]).expect("curve");
    for x in [0.05, 0.5, 2.0, 8.0] {
      assert_close(lift.apply_rgb_overrange([x; 3]), [decode_overrange(lift.eval(encode_overrange(x))); 3]);
    }
    // Above its end, the curve follows its slope at 1.0
    let slope = (lift.eval(1.0) - lift.eval(1.0 - 1.0 / TABLE_SIZE as f32)) * TABLE_SIZE as f32;
    let expected = decode_overrange(1.0 + slope * (encode_overrange(32.0) - 1.0));
    assert_close(lift.apply_rgb_overrange([32.0; 3]), [expected; 3]);
  }
}
