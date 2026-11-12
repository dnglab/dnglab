use super::Point;

// These are the constant factors for each segment of the curve.
// Each segment i will have the formula:
// f(x) = a[i] + b[i]*(x - x[i]) + c[i]*(x - x[i])^2 + d[i]*(x - x[i])^3
#[derive(Clone, Debug, Default)]
struct Segment {
  a: f32,
  b: f32,
  c: f32,
  d: f32,
}

// This is a Natural Cubic Spline. The second derivative at curve ends are zero.
// See https://en.wikipedia.org/wiki/Spline_(mathematics)
// section "Algorithm for computing natural cubic splines"
pub struct Spline {
  num_coords: usize,
  num_segments: usize,
  xcp: Vec<usize>,
  segments: Vec<Segment>,
}

impl Spline {
  fn prepare(&mut self) {
    // Extra values used during computation
    let mut h = vec![0.0; self.num_segments];
    let mut alpha = vec![0.0; self.num_segments];
    let mut mu = vec![0.0; self.num_coords];
    let mut z = vec![0.0; self.num_coords];

    for i in 0..self.num_segments {
      h[i] = (self.xcp[i + 1] - self.xcp[i]) as f32;
    }

    for i in 1..self.num_segments {
      let sp = &self.segments[i - 1];
      let s = &self.segments[i];
      let sn = &self.segments[i + 1];

      alpha[i] = (3. / h[i]) * (sn.a - s.a) - (3. / h[i - 1]) * (s.a - sp.a);
    }

    mu[0] = 1.0;
    z[0] = 0.0;

    for i in (0..self.num_segments).rev() {
      let sn = self.segments[i + 1].clone();
      let s = &mut self.segments[i];
      s.c = z[i] - mu[i] * sn.c;
      s.b = (sn.a - s.a) / h[i] - h[i] * (sn.c + 2. * s.c) / 3.;
      s.d = (sn.c - s.c) / (3. * h[i]);
    }

    // The last segment is nonsensical, and was only used to temporarily store
    // the a and c to simplify calculations, so drop that 'segment' now
    self.segments.pop();

    assert_eq!(self.num_segments, self.segments.len());
  }

  /// Spline through `control_points`, `None` if there are fewer than two, or
  /// their x coordinates don't start at 0, end at 65535 or decrease
  pub fn new(control_points: &[Point]) -> Option<Self> {
    let valid = control_points.len() >= 2
      && control_points.first().is_some_and(|p| p.x == 0)
      && control_points.last().is_some_and(|p| p.x == u16::MAX as usize)
      && control_points.windows(2).all(|p| p[0].x <= p[1].x);
    if !valid {
      return None;
    }

    // TODO: "The X coordinates must all be strictly increasing"
    // TODO: The Y coords must be limited to the range of value_type

    let num_coords = control_points.len();
    let num_segments = num_coords - 1;
    let mut xcp = vec![0; num_coords];
    let mut segments = vec![Segment::default(); num_coords];

    for (i, cpoint) in control_points.iter().enumerate() {
      xcp[i] = cpoint.x;
      segments[i].a = cpoint.y as f32;
    }

    let mut val = Self {
      num_coords,
      num_segments,
      xcp,
      segments,
    };

    val.prepare();

    Some(val)
  }

  pub fn calculate_curve(self) -> Vec<u16> {
    let mut curve = vec![0; u16::MAX as usize + 1];

    for (i, s) in self.segments.iter().enumerate() {
      for x in self.xcp[i]..=self.xcp[i + 1] {
        let diff = (x - self.xcp[i]) as f32;
        let diff_2 = (diff * diff) as f32;
        let diff_3 = (diff * diff * diff) as f32;

        let interpolated = s.a + s.b * diff + s.c * diff_2 + s.d * diff_3;

        curve[x] = interpolated as u16;
      }
    }

    curve
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn spline_test() -> std::result::Result<(), Box<dyn std::error::Error>> {
    crate::init_test_logger();

    let points = [Point::new(0, 0), Point::new(u16::MAX as usize, 10)];

    let spline = Spline::new(&points).ok_or("invalid control points")?;

    let _data = spline.calculate_curve();

    Ok(())
  }

  #[test]
  fn spline_needs_valid_control_points() {
    let p = |x, y| Point::new(x, y);
    assert!(Spline::new(&[p(0, 0)]).is_none());
    // Not starting at 0, not ending at 65535, beyond 65535
    assert!(Spline::new(&[p(1, 0), p(65535, 10)]).is_none());
    assert!(Spline::new(&[p(0, 0), p(65534, 10)]).is_none());
    assert!(Spline::new(&[p(0, 0), p(70000, 5), p(65535, 10)]).is_none());
    assert!(Spline::new(&[p(0, 0), p(65536, 10)]).is_none());
    // Decreasing
    assert!(Spline::new(&[p(0, 0), p(2000, 5), p(1000, 6), p(65535, 10)]).is_none());
    // Equal x coordinates are allowed
    let curve = Spline::new(&[p(0, 0), p(1000, 1000), p(1000, 1000), p(65535, 65535)]).map(Spline::calculate_curve);
    assert_eq!(curve.map(|curve| curve.len()), Some(65536));
  }
}
