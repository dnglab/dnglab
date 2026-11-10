// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

use crate::{
  CFA,
  cfa::PlaneColor,
  imgop::sensor::check_mosaic,
  pixarray::{Color2D, ImageRegion},
};

/// Interpolate the pixels of `pixels` within `border` pixels of its edges
///
/// Each pixel keeps its own sample, each other plane is the mean of the
/// samples of its color in the 3x3 neighbourhood, or 0 if there is none (the
/// `border_interpolate` of dcraw). Demosaic algorithms that need a
/// neighbourhood beyond the edges of the image leave these pixels out.
///
/// The CFA pattern is aligned to the top-left pixel of the image the region is
/// taken from, `colors` maps the CFA colors to the planes of `out`.
///
/// # Errors
/// Fails if `out` doesn't have the size of `pixels`, or for mosaic data and
/// colors that [`check_mosaic`] rejects.
pub fn interpolate_border<const N: usize>(
  pixels: ImageRegion<'_, f32>,
  cfa: &CFA,
  colors: &PlaneColor,
  out: &mut Color2D<f32, N>,
  border: usize,
) -> crate::Result<()>
where
  [f32; N]: Default,
{
  check_mosaic::<N, f32>("Border interpolation", &pixels, cfa, colors)?;
  if (out.width, out.height) != (pixels.width(), pixels.height()) {
    return Err(format!("Border interpolation: output of {}x{} pixels doesn't match {:?}", out.width, out.height, pixels).into());
  }
  let (w, h) = (pixels.width(), pixels.height());
  let roi = pixels.area();
  let cfa = cfa.shift(roi.p.x, roi.p.y);
  let plane_map = colors.plane_lookup_table();
  let plane = |row: usize, col: usize| plane_map[cfa.color_at(row, col)];
  let rows: Vec<&[f32]> = pixels.rows().collect();
  for row in 0..h {
    let inner_row = row >= border && row + border < h;
    let mut col = 0;
    while col < w {
      // Rows away from the edges only have the pixels at the left and right edge
      if inner_row && col == border {
        col = col.max(w - border);
        if col == w {
          break;
        }
      }
      let mut sum = [(0.0, 0_u32); N];
      for y in row.saturating_sub(1)..(row + 2).min(h) {
        for x in col.saturating_sub(1)..(col + 2).min(w) {
          let p = plane(y, x);
          sum[p] = (sum[p].0 + rows[y][x], sum[p].1 + 1);
        }
      }
      let own = plane(row, col);
      for (p, value) in out.at_mut(row, col).iter_mut().enumerate() {
        *value = match sum[p] {
          _ if p == own => rows[row][col],
          (_, 0) => 0.0,
          (total, count) => total / count as f32,
        };
      }
      col += 1;
    }
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{
    imgop::{Dim2, Point, Rect},
    pixarray::ImageView,
  };

  /// Check each pixel of `out` within `border` pixels of the edges against the
  /// samples of the region `area` of an image of `width` pixels per row
  fn assert_border<const N: usize>(samples: &[f32], width: usize, area: Rect, cfa: &CFA, colors: &PlaneColor, out: &Color2D<f32, N>, border: usize)
  where
    [f32; N]: Default,
  {
    let plane_map = colors.plane_lookup_table();
    let at = |row: usize, col: usize| samples[(area.p.y + row) * width + area.p.x + col];
    let plane = |row: usize, col: usize| plane_map[cfa.color_at(area.p.y + row, area.p.x + col)];
    let (w, h) = (area.d.w, area.d.h);
    for (row, col) in (0..h).flat_map(|row| (0..w).map(move |col| (row, col))) {
      let pixel = out.at(row, col);
      if row >= border && row + border < h && col >= border && col + border < w {
        assert_eq!(pixel, &[0.0; N], "Inner pixel {}:{} is written", col, row);
        continue;
      }
      let mut sum = [(0.0, 0); N];
      for y in row.saturating_sub(1)..(row + 2).min(h) {
        for x in col.saturating_sub(1)..(col + 2).min(w) {
          sum[plane(y, x)] = (sum[plane(y, x)].0 + at(y, x), sum[plane(y, x)].1 + 1);
        }
      }
      for p in 0..N {
        let expected = if p == plane(row, col) {
          at(row, col)
        } else if sum[p].1 > 0 {
          sum[p].0 / sum[p].1 as f32
        } else {
          0.0
        };
        assert_eq!(pixel[p], expected, "Plane {} of {}:{}", p, col, row);
      }
    }
  }

  #[test]
  fn border_keeps_samples_and_averages_neighbours() {
    let (width, height) = (24, 20);
    let samples: Vec<f32> = (0..width * height).map(|i| (i * 7919 % 1000) as f32 / 1000.0).collect();
    let view = ImageView::new(&samples, width, height, 1);
    // A region that doesn't start at a pattern boundary shifts the CFA
    let area = Rect::new(Point::new(3, 1), Dim2::new(17, 15));
    for border in [0, 1, 5, 9] {
      for pattern in ["RGGB", "GBRG"] {
        let (cfa, colors) = (CFA::new(pattern), PlaneColor::new("RGB"));
        let mut out = Color2D::<f32, 3>::new(area.d.w, area.d.h);
        interpolate_border(view.region(area), &cfa, &colors, &mut out, border).unwrap();
        assert_border(&samples, width, area, &cfa, &colors, &out, border);
      }
      let (cfa, colors) = (CFA::new("CYGM"), PlaneColor::new("CYGM"));
      let mut out = Color2D::<f32, 4>::new(area.d.w, area.d.h);
      interpolate_border(view.region(area), &cfa, &colors, &mut out, border).unwrap();
      assert_border(&samples, width, area, &cfa, &colors, &out, border);
    }
  }

  #[test]
  fn small_images_are_border_only() {
    let cfa = CFA::new("RGGB");
    for (width, height) in (1..=7).flat_map(|width| (1..=7).map(move |height| (width, height))) {
      let samples: Vec<f32> = (0..width * height).map(|i| (i + 1) as f32).collect();
      let view = ImageView::new(&samples, width, height, 1);
      let mut out = Color2D::<f32, 3>::new(width, height);
      interpolate_border(view.into(), &cfa, &PlaneColor::new("RGB"), &mut out, 3).unwrap();
      assert_border(&samples, width, view.rect(), &cfa, &PlaneColor::new("RGB"), &out, 3);
    }
  }
}
