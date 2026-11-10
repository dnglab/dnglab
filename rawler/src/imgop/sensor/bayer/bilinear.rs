// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::{borrow::Cow, time::Instant};

use multiversion::multiversion;

use crate::{
  CFA,
  cfa::PlaneColor,
  imgop::{
    Dim2,
    sensor::{Demosaic, border::interpolate_border, check_2x2, check_mosaic},
  },
  pixarray::{Color2D, ImageRegion},
};

/// Bilinear demosaic of 2x2 CFA data with 3 or 4 planes
///
/// Each pixel keeps its sample, each other plane is the mean of the samples
/// of its color in the 3x3 neighbourhood. In a 2x2 CFA, a color is next to a
/// pixel either only orthogonally or only diagonally, so this is the bilinear
/// interpolation: green of Bayer RGB data comes from the 4 orthogonal
/// neighbours, red and blue from 2 or 4 neighbours. At the edges of the image
/// the means are of the neighbours inside of it ([`interpolate_border`]).
///
/// It is fast, but soft, with zipper artifacts and false colors at edges. For
/// X-Trans data there is
/// [`XTransBilinearDemosaic`](crate::imgop::sensor::xtrans::bilinear::XTransBilinearDemosaic).
///
/// # Errors
///
/// Fails for CFA patterns that are not 2x2, or if `colors` doesn't have N
/// planes or doesn't match the CFA colors.
#[derive(Default)]
pub struct BilinearDemosaic {}

impl BilinearDemosaic {
  pub fn new() -> Self {
    Self {}
  }
}

impl Demosaic<f32, 3> for BilinearDemosaic {
  fn demosaic(&self, pixels: ImageRegion<'_, f32>, cfa: &CFA, colors: &PlaneColor) -> crate::Result<Color2D<f32, 3>> {
    bilinear(pixels, cfa, colors)
  }
}

impl Demosaic<f32, 4> for BilinearDemosaic {
  fn demosaic(&self, pixels: ImageRegion<'_, f32>, cfa: &CFA, colors: &PlaneColor) -> crate::Result<Color2D<f32, 4>> {
    bilinear(pixels, cfa, colors)
  }
}

/// Bilinear demosaic of `pixels` with N planes, see [`BilinearDemosaic`]
fn bilinear<const N: usize>(pixels: ImageRegion<'_, f32>, cfa: &CFA, colors: &PlaneColor) -> crate::Result<Color2D<f32, N>>
where
  [f32; N]: Default,
{
  check_mosaic::<N, f32>("Bilinear demosaic", &pixels, cfa, colors)?;
  check_2x2("Bilinear demosaic", cfa)?;
  let now = Instant::now();
  let roi = pixels.area();
  let phases = phases::<N>(&cfa.shift(roi.p.x, roi.p.y), colors, roi.d.w)?;

  let mut out = Color2D::<f32, N>::new(roi.width(), roi.height());
  interpolate_border(pixels, cfa, colors, &mut out, 1)?;
  // Contiguous samples are used in place, others are copied
  let samples = match pixels.image_view() {
    Some(view) => Cow::Borrowed(view.as_slice()),
    None => Cow::Owned(pixels.to_vec()),
  };
  interpolate_inner(&samples, roi.d, &phases, &mut out);
  log::debug!("Bilinear debayer time: {:.5}s", now.elapsed().as_secs_f32());
  Ok(out)
}

/// Interpolation of the pixels of one phase of a 2x2 CFA
struct Phase<const N: usize> {
  /// Plane of the pixel
  own: usize,
  /// Neighbours in the 3x3 neighbourhood with another plane than the pixel,
  /// row by row: offset in the samples and plane
  neighbours: Vec<(isize, usize)>,
  /// Number of neighbours of each plane
  counts: [u32; N],
}

/// Phases of the 2x2 `cfa`, row by row, for images of `width` pixels per row
///
/// # Errors
/// Fails if a CFA color has no plane in `colors`.
fn phases<const N: usize>(cfa: &CFA, colors: &PlaneColor, width: usize) -> crate::Result<Vec<Phase<N>>> {
  let plane_map = colors.plane_lookup_table();
  if let Some(color) = (0..4).map(|i| cfa.color_at(i / 2, i % 2)).find(|&color| plane_map[color] >= N) {
    return Err(format!("CFA color {} of '{}' has no plane in {} planes", color, cfa, N).into());
  }
  // Every color of the pattern has a plane, see above
  let plane = |row: usize, col: usize| plane_map[cfa.color_at(row, col)];
  Ok(
    (0..4)
      .map(|phase| {
        // Rows and columns 2 and 3 are the pattern again, they avoid negative positions
        let (row, col) = (2 + phase / 2, 2 + phase % 2);
        let own = plane(row, col);
        let mut neighbours = Vec::new();
        let mut counts = [0; N];
        for (dy, dx) in (-1..=1).flat_map(|dy| (-1..=1).map(move |dx| (dy, dx))) {
          let p = plane(row.wrapping_add_signed(dy), col.wrapping_add_signed(dx));
          if p != own {
            neighbours.push((dy * width as isize + dx, p));
            counts[p] += 1;
          }
        }
        Phase { own, neighbours, counts }
      })
      .collect(),
  )
}

/// Interpolate the pixels of `out` that are not at the edges, `samples` are
/// the pixels of an image of `dim` row by row
#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn interpolate_inner<const N: usize>(samples: &[f32], dim: Dim2, phases: &[Phase<N>], out: &mut Color2D<f32, N>)
where
  [f32; N]: Default,
{
  if dim.w < 3 || dim.h < 3 {
    return;
  }
  out.for_each_row(|row, line| {
    if row == 0 || row == dim.h - 1 {
      return;
    }
    for (col, pixel) in line.iter_mut().enumerate().take(dim.w - 1).skip(1) {
      let i = row * dim.w + col;
      let phase = &phases[(row % 2) * 2 + col % 2];
      let mut sums = [0.0_f32; N];
      for &(offset, plane) in &phase.neighbours {
        sums[plane] += samples[i.wrapping_add_signed(offset)];
      }
      for (plane, value) in pixel.iter_mut().enumerate() {
        *value = match phase.counts[plane] {
          _ if plane == phase.own => samples[i],
          0 => 0.0,
          count => sums[plane] / count as f32,
        };
      }
    }
  });
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{
    imgop::{Point, Rect},
    pixarray::ImageView,
  };

  /// Check each pixel of `out` against the samples of the region `area` of an
  /// image of `width` pixels per row: its own sample, and the mean of the
  /// samples of each other plane in the 3x3 neighbourhood inside of the area
  fn assert_bilinear<const N: usize>(samples: &[f32], width: usize, area: Rect, pattern: &str, colors: &str, out: &Color2D<f32, N>)
  where
    [f32; N]: Default,
  {
    let (cfa, plane_map) = (CFA::new(pattern), PlaneColor::new(colors).plane_lookup_table());
    let at = |row: usize, col: usize| samples[(area.p.y + row) * width + area.p.x + col];
    let plane = |row: usize, col: usize| plane_map[cfa.color_at(area.p.y + row, area.p.x + col)];
    let (w, h) = (area.d.w, area.d.h);
    assert_eq!((out.width, out.height), (w, h));
    for (row, col) in (0..h).flat_map(|row| (0..w).map(move |col| (row, col))) {
      let mut sum = [(0.0, 0); N];
      for y in row.saturating_sub(1)..(row + 2).min(h) {
        for x in col.saturating_sub(1)..(col + 2).min(w) {
          sum[plane(y, x)] = (sum[plane(y, x)].0 + at(y, x), sum[plane(y, x)].1 + 1);
        }
      }
      for p in 0..N {
        let expected = match sum[p] {
          _ if p == plane(row, col) => at(row, col),
          (_, 0) => 0.0,
          (total, count) => total / count as f32,
        };
        assert_eq!(out.at(row, col)[p], expected, "{} {}x{} plane {} at {}:{}", pattern, w, h, p, col, row);
      }
    }
  }

  fn mosaic(width: usize, height: usize) -> Vec<f32> {
    (0..width * height).map(|i| (i * 7919 % 1000) as f32 / 1000.0).collect()
  }

  #[test]
  fn interpolates_the_3x3_neighbourhood() {
    let (width, height) = (31, 23);
    let samples = mosaic(width, height);
    let view = ImageView::new(&samples, width, height, 1);
    // The whole image and regions that shift the CFA
    for area in [
      view.rect(),
      Rect::new(Point::new(3, 1), Dim2::new(25, 19)),
      Rect::new(Point::new(2, 5), Dim2::new(4, 3)),
    ] {
      for pattern in ["RGGB", "BGGR", "GRBG", "GBRG"] {
        let rgb: Color2D<f32, 3> = BilinearDemosaic::new()
          .demosaic(view.region(area), &CFA::new(pattern), &PlaneColor::new("RGB"))
          .expect("demosaic");
        assert_bilinear(&samples, width, area, pattern, "RGB", &rgb);
      }
      for (pattern, colors) in [("CYGM", "CYGM"), ("ERBG", "RGBE")] {
        let planes: Color2D<f32, 4> = BilinearDemosaic::new()
          .demosaic(view.region(area), &CFA::new(pattern), &PlaneColor::new(colors))
          .expect("demosaic");
        assert_bilinear(&samples, width, area, pattern, colors, &planes);
      }
    }
  }

  #[test]
  fn all_sizes_are_interpolated() {
    // Images of 1 or 2 pixels in a direction are edges only
    for (width, height) in (1..=7).flat_map(|width| (1..=7).map(move |height| (width, height))) {
      let samples = mosaic(width, height);
      let view = ImageView::new(&samples, width, height, 1);
      let rgb: Color2D<f32, 3> = BilinearDemosaic::new()
        .demosaic(view.into(), &CFA::new("GRBG"), &PlaneColor::new("RGB"))
        .expect("demosaic");
      assert_bilinear(&samples, width, view.rect(), "GRBG", "RGB", &rgb);
      let planes: Color2D<f32, 4> = BilinearDemosaic::new()
        .demosaic(view.into(), &CFA::new("CYGM"), &PlaneColor::new("CYGM"))
        .expect("demosaic");
      assert_bilinear(&samples, width, view.rect(), "CYGM", "CYGM", &planes);
    }
  }

  #[test]
  fn xtrans_data_is_not_supported() {
    let samples = mosaic(12, 12);
    let result: crate::Result<Color2D<f32, 3>> = BilinearDemosaic::new().demosaic(
      ImageView::new(&samples, 12, 12, 1).into(),
      &CFA::new("GGRGGBGGBGGRBRGRBGGGBGGRGGRGGBRBGBRG"),
      &PlaneColor::new("RGB"),
    );
    assert!(result.err().is_some_and(|err| err.to_string().contains("needs a 2x2 CFA pattern")));
  }
}
