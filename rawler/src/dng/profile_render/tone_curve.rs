// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! ProfileToneCurve

/// Number of intervals of the lookup table
const TABLE_SIZE: usize = 4096;

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

  /// Apply the curve to RGB values, preserving the hue
  ///
  /// The largest and smallest channel are mapped by the curve, the middle
  /// channel keeps its relative position between them.
  pub fn apply_rgb(&self, rgb: [f32; 3]) -> [f32; 3] {
    let rgb = rgb.map(|c| c.clamp(0.0, 1.0));
    let mut order = [0, 1, 2];
    order.sort_by(|a, b| rgb[*b].total_cmp(&rgb[*a]));
    let [max, mid, min] = order;
    if rgb[max] <= rgb[min] {
      return rgb.map(|c| self.eval(c));
    }
    let mut result = [0.0; 3];
    result[max] = self.eval(rgb[max]);
    result[min] = self.eval(rgb[min]);
    result[mid] = result[min] + (result[max] - result[min]) * (rgb[mid] - rgb[min]) / (rgb[max] - rgb[min]);
    result
  }
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
  fn invalid_curves() {
    assert!(ToneCurve::new(&[0.0, 0.0]).is_none());
    assert!(ToneCurve::new(&[0.0, 0.0, 1.0]).is_none());
    assert!(ToneCurve::new(&[0.0, 0.0, 0.5, 0.5, 0.5, 0.6, 1.0, 1.0]).is_none());
    assert!(ToneCurve::new(&[0.0, 0.0, f32::NAN, 0.5, 1.0, 1.0]).is_none());
  }
}
