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

  pub fn new(control_points: &[Point]) -> Self {
    assert!(control_points.len() >= 2, "Need at least two points to interpolate between");
    assert_eq!(control_points.first().map(|p| p.x as u16), Some(u16::MIN));
    assert_eq!(control_points.last().map(|p| p.x as u16), Some(u16::MAX));

    let mut prev = 0;
    for p in control_points {
      if p.x < prev {
        panic!("err, p.x {} must be >= {}", p.x, prev);
      }
      prev = p.x;
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

    val
  }

  pub fn calculate_curve(self) -> Vec<u16> {
    let mut curve = vec![0; u16::MAX as usize + 1];

    for (i, s) in self.segments.iter().enumerate() {
      for x in self.xcp[i]..=self.xcp[i + 1] {
        let diff = (x - self.xcp[i]) as f32;
        let diff_2 = diff * diff;
        let diff_3 = diff * diff * diff;

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

    let spline = Spline::new(&points);

    let _data = spline.calculate_curve();

    Ok(())
  }
}
