// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

use std::time::Instant;

use multiversion::multiversion;

use crate::{
  cfa::{CFA, CFA_COLOR_G, PlaneColor},
  imgop::{
    Dim2,
    sensor::{
      Demosaic,
      border::interpolate_border,
      check_bayer_rgb, check_mosaic,
      tiles::{HaloTile, halo_tiles, inner_area, par_tiles},
    },
  },
  pixarray::{Color2D, DisjointRegionMut, ImageRegion, ImageRegionMut},
};

/// Pixels at the edges that RCD leaves to [`interpolate_border`], and the halo
/// of its tiles: the reach of its steps
const HALO: usize = 10;

/// Width and height of the windows of the tiles, with the halo
const TILE: usize = 256;

/// Added to gradients and to the sums of ratios, so they are never zero
const EPS: f32 = 1e-5;

/// Smallest value of the directional statistics
const EPSSQ: f32 = 1e-10;

/// RCD demosaic of Bayer RGB data
///
/// Ratio Corrected Demosaicing, an algorithm of Luis Sanz Rodríguez:
///
/// 1. A vertical and a horizontal statistic, the squared high-pass filters of
///    the samples in the column and in the row, give the direction of the
///    interpolation of each pixel.
/// 2. A low-pass filter of the samples gives a soft image without artifacts.
/// 3. At red and blue pixels, green is interpolated from the 4 green
///    neighbours, each corrected by the ratio of the low-pass filter at the
///    pixel and at the pixel of its color behind the neighbour. The estimates
///    are weighted by the gradients of their direction, and the vertical and
///    horizontal estimates by the direction of step 1.
/// 4. Red and blue are interpolated from the differences to green: at blue
///    and red pixels along the diagonals, with a diagonal statistic like in
///    step 1, at green pixels vertically and horizontally.
///
/// Where the direction of a pixel is less clear than the mean of its diagonal
/// neighbours, it takes their mean. The ratios of step 3 avoid the overshoots
/// of color differences at sharp edges.
///
/// The image is processed in tiles with a halo of 10 pixels in parallel, the
/// 10 pixels at the edges are interpolated bilinearly ([`interpolate_border`]).
/// Negative samples are interpolated as 0, values above white are kept.
///
/// # Errors
///
/// Fails for CFA patterns that are not RGGB or variants.
pub struct RcdDemosaic {
  /// Size of the cores of the tiles, the area each tile writes
  core: Dim2,
}

impl Default for RcdDemosaic {
  fn default() -> Self {
    Self::new()
  }
}

impl RcdDemosaic {
  pub fn new() -> Self {
    Self::with_tile(TILE)
  }

  /// Demosaic in tiles of `tile` pixels (with the halo)
  fn with_tile(tile: usize) -> Self {
    assert!(tile > 2 * HALO, "Tile of {} pixels has no core", tile);
    Self {
      core: Dim2::new(tile - 2 * HALO, tile - 2 * HALO),
    }
  }
}

impl Demosaic<f32, 3> for RcdDemosaic {
  fn demosaic(&self, pixels: ImageRegion<'_, f32>, cfa: &CFA, colors: &PlaneColor) -> crate::Result<Color2D<f32, 3>> {
    check_mosaic::<3, f32>("RCD", &pixels, cfa, colors)?;
    check_bayer_rgb("RCD", cfa)?;
    let now = Instant::now();
    // The ROI changes the pattern if not perfectly aligned on the origin pattern
    let roi = pixels.area();
    let cfa_roi = cfa.shift(roi.p.x, roi.p.y);

    let mut out = Color2D::<f32, 3>::new(roi.width(), roi.height());
    interpolate_border(pixels, cfa, colors, &mut out, HALO)?;
    let tiles = halo_tiles(roi.d, inner_area(roi.d, HALO), self.core, HALO)?;
    par_tiles(ImageRegionMut::from(out.view_mut()), &tiles, Buffers::default, |buffers, tile, region| {
      rcd_tile(pixels, &cfa_roi, tile, buffers, region);
    })?;
    log::debug!("RCD debayer time: {:.5}s", now.elapsed().as_secs_f32());
    Ok(out)
  }
}

/// Buffers of a tile, reused for the following tiles of a worker
///
/// They are not cleared between the tiles: each step reads only values that
/// the steps before have written for the tile.
#[derive(Default)]
struct Buffers {
  /// Samples of the window, at least 0
  cfa: Vec<f32>,
  /// Red, green and blue planes
  rgb: [Vec<f32>; 3],
  /// Squared vertical and horizontal high-pass filters
  v_hpf: Vec<f32>,
  h_hpf: Vec<f32>,
  /// Direction of the interpolation: 0 is vertical, 1 horizontal
  vh_dir: Vec<f32>,
  /// Low-pass filter at red and blue pixels
  lpf: Vec<f32>,
  /// Squared high-pass filters along the diagonals: P from the top-left to
  /// the bottom-right, Q from the top-right to the bottom-left
  p_hpf: Vec<f32>,
  q_hpf: Vec<f32>,
  /// Diagonal direction at red and blue pixels: 0 is P, 1 is Q
  pq_dir: Vec<f32>,
}

/// Squared high-pass filter of 7 samples along a line, `at(k)` is the sample
/// `k` steps from the center
#[inline(always)]
fn high_pass(at: impl Fn(isize) -> f32) -> f32 {
  let value = (at(-3) - at(-1) - at(1) + at(3)) - 3.0 * (at(-2) + at(2)) + 6.0 * at(0);
  value * value
}

/// `weight` of `a` and `1 - weight` of `b`
#[inline(always)]
fn blend(weight: f32, a: f32, b: f32) -> f32 {
  weight * (a - b) + b
}

/// Mean of two estimates from opposite sides, each weighted by the gradient
/// of the other side
#[inline(always)]
fn weighted(grad_a: f32, est_a: f32, grad_b: f32, est_b: f32) -> f32 {
  (grad_b * est_a + grad_a * est_b) / (grad_a + grad_b)
}

/// Direction at `i` of `dir`: the value of the pixel, or the mean of its
/// diagonal neighbours if that is further from 0.5
#[inline(always)]
fn refined(dir: &[f32], i: usize, w: usize) -> f32 {
  let central = dir[i];
  let neighbourhood = 0.25 * (dir[i - w - 1] + dir[i - w + 1] + dir[i + w - 1] + dir[i + w + 1]);
  if (0.5 - central).abs() < (0.5 - neighbourhood).abs() {
    neighbourhood
  } else {
    central
  }
}

/// Demosaic `tile` of `pixels` into `region`, its core
///
/// Each step covers the area of the pixels the following steps read, given
/// as the distance from the edges of the window.
#[multiversion(targets("x86_64+avx512f+avx512bw+avx512vl", "x86_64+avx+avx2", "x86_64+sse4.1", "aarch64+neon"))]
fn rcd_tile(pixels: ImageRegion<f32>, cfa_roi: &CFA, tile: &HaloTile, buffers: &mut Buffers, region: &mut DisjointRegionMut<f32>) {
  let (tw, th) = (tile.window.d.w, tile.window.d.h);
  let core = tile.core_in_window();
  // halo_tiles over the inner area with the same halo gives the whole halo
  debug_assert!(
    core.p.x == HALO && core.p.y == HALO && core.d.w + 2 * HALO == tw && core.d.h + 2 * HALO == th,
    "Window of {:?} has no halo of {} pixels",
    tile,
    HALO
  );
  let cfa = cfa_roi.shift(tile.window.p.x, tile.window.p.y);
  let Buffers {
    cfa: c,
    rgb,
    v_hpf,
    h_hpf,
    vh_dir,
    lpf,
    p_hpf,
    q_hpf,
    pq_dir,
  } = buffers;
  pixels.region(tile.window).copy_into(c);
  c.iter_mut().for_each(|v| *v = v.max(0.0));
  for buffer in rgb
    .iter_mut()
    .chain([&mut *v_hpf, &mut *h_hpf, &mut *vh_dir, &mut *lpf, &mut *p_hpf, &mut *q_hpf, &mut *pq_dir])
  {
    buffer.resize(tw * th, 0.0);
  }
  let w = tw;
  let (w2, w3, w4) = (2 * w, 3 * w, 4 * w);
  // Rows and columns at least `m` pixels away from the edges of the window
  let rows = |m: usize| m..th - m;
  let cols = |m: usize| m..tw - m;
  // Pixels of `row` at least `m` pixels away from the edges, the green ones
  // or the red and blue ones
  let sites = |row: usize, m: usize, green: bool| {
    let first = m + ((cfa.color_at(row, m) == CFA_COLOR_G) != green) as usize;
    (first..tw - m).step_by(2)
  };
  for row in 0..th {
    for col in 0..tw {
      let i = row * w + col;
      rgb[cfa.color_at(row, col)][i] = c[i];
    }
  }

  // Step 1: the vertical and horizontal direction
  for row in rows(3) {
    for col in cols(3) {
      let i = row * w + col;
      v_hpf[i] = high_pass(|k| c[i.wrapping_add_signed(k * w as isize)]);
      h_hpf[i] = high_pass(|k| c[i.wrapping_add_signed(k)]);
    }
  }
  for row in rows(4) {
    for col in cols(4) {
      let i = row * w + col;
      let v = (v_hpf[i - w] + v_hpf[i] + v_hpf[i + w]).max(EPSSQ);
      let h = (h_hpf[i - 1] + h_hpf[i] + h_hpf[i + 1]).max(EPSSQ);
      vh_dir[i] = v / (v + h);
    }
  }

  // Step 2: low-pass filter at red and blue pixels
  for row in rows(3) {
    for col in sites(row, 3, false) {
      let i = row * w + col;
      lpf[i] = c[i] + 0.5 * (c[i - w] + c[i + w] + c[i - 1] + c[i + 1]) + 0.25 * (c[i - w - 1] + c[i - w + 1] + c[i + w - 1] + c[i + w + 1]);
    }
  }

  // Step 3: green at red and blue pixels
  for row in rows(5) {
    for col in sites(row, 5, false) {
      let i = row * w + col;
      let disc = refined(vh_dir, i, w);
      // Gradients of the samples towards each side
      let n_grad = EPS + (c[i - w] - c[i + w]).abs() + (c[i] - c[i - w2]).abs() + (c[i - w] - c[i - w3]).abs() + (c[i - w2] - c[i - w4]).abs();
      let s_grad = EPS + (c[i - w] - c[i + w]).abs() + (c[i] - c[i + w2]).abs() + (c[i + w] - c[i + w3]).abs() + (c[i + w2] - c[i + w4]).abs();
      let w_grad = EPS + (c[i - 1] - c[i + 1]).abs() + (c[i] - c[i - 2]).abs() + (c[i - 1] - c[i - 3]).abs() + (c[i - 2] - c[i - 4]).abs();
      let e_grad = EPS + (c[i - 1] - c[i + 1]).abs() + (c[i] - c[i + 2]).abs() + (c[i + 1] - c[i + 3]).abs() + (c[i + 2] - c[i + 4]).abs();
      // The green neighbours, corrected by the ratio of the low-pass filter
      // at the pixel and at the pixel of its color behind the neighbour
      let ratio = |far: usize| 1.0 + (lpf[i] - lpf[far]) / (EPS + lpf[i] + lpf[far]);
      let n_est = c[i - w] * ratio(i - w2);
      let s_est = c[i + w] * ratio(i + w2);
      let w_est = c[i - 1] * ratio(i - 2);
      let e_est = c[i + 1] * ratio(i + 2);
      let v_est = weighted(n_grad, n_est, s_grad, s_est);
      let h_est = weighted(w_grad, w_est, e_grad, e_est);
      rgb[CFA_COLOR_G][i] = blend(disc, h_est, v_est);
    }
  }

  // Step 4.1: the diagonal direction at red and blue pixels
  for row in rows(4) {
    for col in sites(row, 4, false) {
      let i = row * w + col;
      p_hpf[i] = high_pass(|k| c[i.wrapping_add_signed(k * (w as isize + 1))]);
      q_hpf[i] = high_pass(|k| c[i.wrapping_add_signed(k * (w as isize - 1))]);
    }
  }
  for row in rows(6) {
    for col in sites(row, 6, false) {
      let i = row * w + col;
      let p = (p_hpf[i - w - 1] + p_hpf[i] + p_hpf[i + w + 1]).max(EPSSQ);
      let q = (q_hpf[i - w + 1] + q_hpf[i] + q_hpf[i + w - 1]).max(EPSSQ);
      pq_dir[i] = p / (p + q);
    }
  }

  // Step 4.2: red at blue and blue at red pixels, from the diagonal neighbours
  for row in rows(7) {
    for col in sites(row, 7, false) {
      let i = row * w + col;
      let other = 2 - cfa.color_at(row, col);
      let (o, g) = (&rgb[other], &rgb[CFA_COLOR_G]);
      let disc = refined(pq_dir, i, w);
      let nw_grad = EPS + (o[i - w - 1] - o[i + w + 1]).abs() + (o[i - w - 1] - o[i - w3 - 3]).abs() + (g[i] - g[i - w2 - 2]).abs();
      let ne_grad = EPS + (o[i - w + 1] - o[i + w - 1]).abs() + (o[i - w + 1] - o[i - w3 + 3]).abs() + (g[i] - g[i - w2 + 2]).abs();
      let sw_grad = EPS + (o[i - w + 1] - o[i + w - 1]).abs() + (o[i + w - 1] - o[i + w3 - 3]).abs() + (g[i] - g[i + w2 - 2]).abs();
      let se_grad = EPS + (o[i - w - 1] - o[i + w + 1]).abs() + (o[i + w + 1] - o[i + w3 + 3]).abs() + (g[i] - g[i + w2 + 2]).abs();
      // Color differences at the diagonal neighbours
      let difference = |n: usize| o[n] - g[n];
      let p_est = weighted(nw_grad, difference(i - w - 1), se_grad, difference(i + w + 1));
      let q_est = weighted(ne_grad, difference(i - w + 1), sw_grad, difference(i + w - 1));
      let value = g[i] + blend(disc, q_est, p_est);
      rgb[other][i] = value;
    }
  }

  // Step 4.3: red and blue at green pixels, from the vertical and horizontal
  // neighbours
  for row in rows(HALO) {
    for col in sites(row, HALO, true) {
      let i = row * w + col;
      let disc = refined(vh_dir, i, w);
      for color in [0, 2] {
        let (o, g) = (&rgb[color], &rgb[CFA_COLOR_G]);
        let (vertical, horizontal) = ((o[i - w] - o[i + w]).abs(), (o[i - 1] - o[i + 1]).abs());
        let n_grad = EPS + (g[i] - g[i - w2]).abs() + (o[i - w] - o[i - w3]).abs() + vertical;
        let s_grad = EPS + (g[i] - g[i + w2]).abs() + (o[i + w] - o[i + w3]).abs() + vertical;
        let w_grad = EPS + (g[i] - g[i - 2]).abs() + (o[i - 1] - o[i - 3]).abs() + horizontal;
        let e_grad = EPS + (g[i] - g[i + 2]).abs() + (o[i + 1] - o[i + 3]).abs() + horizontal;
        // Color differences at the neighbours
        let difference = |n: usize| o[n] - g[n];
        let v_est = weighted(n_grad, difference(i - w), s_grad, difference(i + w));
        let h_est = weighted(w_grad, difference(i - 1), e_grad, difference(i + 1));
        let value = g[i] + blend(disc, h_est, v_est);
        rgb[color][i] = value;
      }
    }
  }

  // Write the core
  for (row, line) in (HALO..).zip(region.rows_mut()) {
    for (col, pixel) in (HALO..).zip(line.as_chunks_mut::<3>().0) {
      let i = row * w + col;
      *pixel = [rgb[0][i].max(0.0), rgb[1][i].max(0.0), rgb[2][i].max(0.0)];
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::pixarray::ImageView;

  const PATTERNS: [&str; 4] = ["RGGB", "BGGR", "GRBG", "GBRG"];

  fn mosaic(width: usize, height: usize) -> Vec<f32> {
    (0..width * height).map(|i| (i * 7919 % 1000) as f32 / 1000.0).collect()
  }

  fn demosaic(algorithm: &RcdDemosaic, samples: &[f32], width: usize, height: usize, pattern: &str) -> Color2D<f32, 3> {
    algorithm
      .demosaic(ImageView::new(samples, width, height, 1).into(), &CFA::new(pattern), &PlaneColor::new("RGB"))
      .expect("demosaic")
  }

  #[test]
  fn tiling_doesnt_change_the_result() {
    // Tiles with cores of 1, 7 and 20 pixels against one tile for the image
    let (width, height) = (71, 53);
    let samples = mosaic(width, height);
    for pattern in PATTERNS {
      let expected = demosaic(&RcdDemosaic::with_tile(100), &samples, width, height, pattern);
      for tile in [21, 27, 40] {
        let demosaiced = demosaic(&RcdDemosaic::with_tile(tile), &samples, width, height, pattern);
        assert!(demosaiced.data == expected.data, "{} with tiles of {} pixels", pattern, tile);
      }
    }
  }

  #[test]
  fn small_images_keep_their_samples() {
    // Images up to 20 pixels high or wide are border only
    for pattern in PATTERNS {
      let cfa = CFA::new(pattern);
      for (width, height) in (1..=24).flat_map(|width| (1..=24).map(move |height| (width, height))) {
        let samples = mosaic(width, height);
        let rgb = demosaic(&RcdDemosaic::new(), &samples, width, height, pattern);
        assert_eq!((rgb.width, rgb.height), (width, height), "{}", pattern);
        for (row, col) in (0..height).flat_map(|row| (0..width).map(move |col| (row, col))) {
          assert_eq!(
            rgb.at(row, col)[cfa.color_at(row, col)],
            samples[row * width + col],
            "{} {}x{} at {}:{}",
            pattern,
            width,
            height,
            col,
            row
          );
        }
      }
    }
  }

  #[test]
  fn negative_samples_are_zero() {
    let (width, height) = (40, 30);
    let samples: Vec<f32> = mosaic(width, height).iter().map(|v| v - 0.5).collect();
    let rgb = demosaic(&RcdDemosaic::new(), &samples, width, height, "RGGB");
    let inner = inner_area(Dim2::new(width, height), HALO);
    for row in inner.p.y..inner.p.y + inner.d.h {
      for col in inner.p.x..inner.p.x + inner.d.w {
        let pixel = rgb.at(row, col);
        assert!(pixel.iter().all(|v| *v >= 0.0), "{:?} at {}:{}", pixel, col, row);
        assert_eq!(pixel[CFA::new("RGGB").color_at(row, col)], samples[row * width + col].max(0.0));
      }
    }
  }

  #[test]
  fn flat_colors_stay_flat() {
    let (width, height) = (48, 36);
    let color = [0.2, 0.5, 0.3];
    for pattern in PATTERNS {
      let cfa = CFA::new(pattern);
      let samples: Vec<f32> = (0..width * height).map(|i| color[cfa.color_at(i / width, i % width)]).collect();
      let rgb = demosaic(&RcdDemosaic::new(), &samples, width, height, pattern);
      for (i, pixel) in rgb.pixels().iter().enumerate() {
        for c in 0..3 {
          assert!((pixel[c] - color[c]).abs() < 1e-5, "{} {:?} at {}:{}", pattern, pixel, i % width, i / width);
        }
      }
    }
  }

  #[test]
  fn edges_stay_neutral() {
    // Neutral data with a horizontal or vertical edge: the interpolation
    // along the edge is neutral, across it there are false colors
    let (width, height) = (60, 50);
    for pattern in PATTERNS {
      for horizontal in [true, false] {
        let value = |row: usize, col: usize| {
          let across = if horizontal { row } else { col };
          if across < 25 { 0.2 } else { 0.8 }
        };
        let samples: Vec<f32> = (0..width * height).map(|i| value(i / width, i % width)).collect();
        let rgb = demosaic(&RcdDemosaic::new(), &samples, width, height, pattern);
        let inner = inner_area(Dim2::new(width, height), HALO);
        for row in inner.p.y..inner.p.y + inner.d.h {
          for col in inner.p.x..inner.p.x + inner.d.w {
            let pixel = rgb.at(row, col);
            assert!(
              (pixel[0] - pixel[1]).abs() < 1e-5 && (pixel[2] - pixel[1]).abs() < 1e-5,
              "{} {:?} at {}:{}",
              pattern,
              pixel,
              col,
              row
            );
          }
        }
      }
    }
  }

  #[test]
  fn scaled_data_gives_a_scaled_result() {
    // The statistics, ratios and color differences scale with the data, only
    // EPS doesn't. Data of few levels has many equal values.
    let (width, height) = (48, 36);
    let levels: Vec<f32> = (0..width * height).map(|i| ((i * 7919 + i / 5) % 13) as f32).collect();
    for pattern in PATTERNS {
      let (a, b) = (0.05, 0.05 * 1.03);
      let scaled = |scale: f32| levels.iter().map(|v| v * scale).collect::<Vec<f32>>();
      let first = demosaic(&RcdDemosaic::new(), &scaled(a), width, height, pattern);
      let second = demosaic(&RcdDemosaic::new(), &scaled(b), width, height, pattern);
      for (i, (p, q)) in first.pixels().iter().zip(second.pixels()).enumerate() {
        for (v, w) in p.iter().zip(q) {
          assert!((v / a - w / b).abs() < 1e-3, "{} {:?} != {:?} at {}:{}", pattern, p, q, i % width, i / width);
        }
      }
    }
  }
}
