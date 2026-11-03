// SPDX-License-Identifier: LGPL-2.1
// Copyright 2022 Daniel Vogelbacher <daniel@chaospixel.com>

use multiversion::multiversion;
use rayon::prelude::*;
use std::{ops::Add, time::Instant};

use crate::{
  cfa::{CFA, CFA_COLOR_B, CFA_COLOR_G, CFA_COLOR_R, PlaneColor},
  imgop::sensor::Demosaic,
  pixarray::{Color2D, ImageRegion, RgbF32},
};

#[derive(Default)]
pub struct PPGDemosaic {}

impl PPGDemosaic {
  pub fn new() -> Self {
    Self {}
  }
}

/// PPG demosaic a raw image (f32 values)
///
/// The PPG - Pattern Pixel Grouping - alogrithm was developed by Chuan-kai Lin
/// Source on Internet Archive:
/// [](https://web.archive.org/web/20160923211135/https://sites.google.com/site/chklin/demosaic/)
///
/// This is a simple algorithm but provides acceptable results. Like the color
/// differences it interpolates, it works best on white balanced data.
///
/// # Panics
///
/// This function panics for CFA pattern that are not RGGB or variants. You need
/// to check the pattern before calling.
impl Demosaic<f32, 3> for PPGDemosaic {
  /// Debayer image by using the PPG (Patterned Pixel Grouping) algorithm.
  fn demosaic(&self, pixels: ImageRegion<'_, f32>, cfa: &CFA, _colors: &PlaneColor) -> Color2D<f32, 3> {
    assert_eq!(pixels.channels(), 1, "Mosaic data must have one channel");
    // PPG can only applied to pure RGGB or variants.
    if !cfa.is_rgb() {
      panic!("CFA pattern '{}' is not a RGB pattern, can not demosaic with PPG", cfa);
    }
    // Measure time
    let now = Instant::now();

    // The ROI changes the pattern if not perfectly aligned on the origin pattern
    let roi = pixels.area();
    let cfa_roi = cfa.shift(roi.p.x, roi.p.y);

    // Expand the bayer data to full RGB channel image, but only for ROI
    let mut rgb = super::expand_bayer_rgb(pixels, cfa);

    // Now interpolate the missing channels
    interpolate_borders(&mut rgb, &cfa_roi);
    interpolate_green(&mut rgb, &cfa_roi);
    interpolate_rb_at_green(&mut rgb, &cfa_roi);
    interpolate_rb_at_non_green(&mut rgb, &cfa_roi);

    log::debug!("PPG total debayer time: {:.5}s", now.elapsed().as_secs_f32());
    rgb
  }
}

/// PPG Demosaic: Interpolate borders
/// We take 3 pixels on each border and interpolate by bilinear interpolation.
/// Bilinear interpolation is done by `(x1 + x2 + x3 + x4) / 4` or a
/// reduced sum and count if not all samples are available like on borders.
///
/// Basically, for each pixel position, we iterate around it and collect
/// all channel values. Then apply interpolation to calculate the missing
/// two channel colors for the origin pixel position.
#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn interpolate_borders(input: &mut RgbF32, shifted: &CFA) {
  let w = input.width;
  let h = input.height;
  // Iterate over all rows
  for row in 0..h {
    let mut col = 0;
    while col < w {
      // Full process first 3 and last 3 rows, process only left and right 3 pixels for all other
      if col == 3 && row >= 3 && row < h - 3 {
        col = col.max(w - 3) // go to right border, unless it starts left of here
      }
      // Store the pixel sum and count for each RGB channel (R=0, G=1, B=2)
      let mut sum = [(0.0, 0_usize); 3];
      // We iterate around the current pixel, 8+1 pixels total in ideal cases (less on edges)
      for y in row.saturating_sub(1)..=row.add(1) {
        for x in col.saturating_sub(1)..=col.add(1) {
          // We have ensured that the target is not out of range for north and west, let's check for east and south.
          if y < h && x < w {
            let ch = shifted.color_at(y, x);
            sum[ch].0 += input.at(y, x)[ch];
            sum[ch].1 += 1;
          }
        }
      }
      // Now we have collected all surrounding pixels, let's interpolate missing 2 channels
      let ch = shifted.color_at(row, col);
      for (color, p) in input.at_mut(row, col).iter_mut().enumerate() {
        // Check if the color is one of the missing 2 channels (ch is the known on)
        if color != ch && sum[color].1 > 0 {
          *p = sum[color].0 / sum[color].1 as f32; // Bilinear interpolation
        }
      }
      col += 1;
    }
  }
}

/// PPG Demosaic: Interpolate missing G channels
/// After this procedure, all green values are known.
///
/// It writes G at R/B sites and reads the samples of R/B sites and G at G
/// sites, so it doesn't read values it writes.
#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn interpolate_green(img: &mut RgbF32, shifted: &CFA) {
  let w = img.width;

  par_interior_rows(img, |row, [n2, n1], line, [s1, s2]| {
    for col in 3..w - 3 {
      if shifted.color_at(row, col) != CFA_COLOR_G {
        let ch = shifted.color_at(row, col); // It's R or B
        let x = line[col][ch];
        let n_1 = n1[col][CFA_COLOR_G];
        let n_2 = n2[col][ch];
        let e_1 = line[col + 1][CFA_COLOR_G];
        let e_2 = line[col + 2][ch];
        let s_1 = s1[col][CFA_COLOR_G];
        let s_2 = s2[col][ch];
        let w_1 = line[col - 1][CFA_COLOR_G];
        let w_2 = line[col - 2][ch];

        // Calculate the gradients for each direction.
        let n = (x - n_2).abs() * 2.0 + (n_1 - s_1).abs();
        let e = (x - e_2).abs() * 2.0 + (w_1 - e_1).abs();
        let w = (x - w_2).abs() * 2.0 + (w_1 - e_1).abs();
        let s = (x - s_2).abs() * 2.0 + (n_1 - s_1).abs();

        // Find the minimum value of the gradients.
        let mut min = n;
        if e < min {
          min = e
        };
        if w < min {
          min = w
        };
        if s < min {
          min = s
        };

        // The minimum gradient wins.
        let p_green = if min == n {
          (n_1 * 3.0 + s_1 + x - n_2) / 4.0
        } else if min == e {
          (e_1 * 3.0 + w_1 + x - e_2) / 4.0
        } else if min == w {
          (w_1 * 3.0 + e_1 + x - w_2) / 4.0
        } else {
          (s_1 * 3.0 + n_1 + x - s_2) / 4.0
        };
        line[col][CFA_COLOR_G] = p_green;
      }
    }
  });
}

/// PPG Demosaic: Interpolate R/B channel at G channels
///
/// It writes R and B at G sites and reads G and the samples of R/B sites, so
/// it doesn't read values it writes.
#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn interpolate_rb_at_green(img: &mut RgbF32, shifted: &CFA) {
  let w = img.width;

  par_interior_rows(img, |row, [_, n1], line, [s1, _]| {
    for col in 3..w - 3 {
      if shifted.color_at(row, col) == CFA_COLOR_G {
        let h_ch = shifted.color_at(row, col + 1); // horizontal corresponding channel
        let v_ch = shifted.color_at(row + 1, col); // vertical corresponding channel

        // Green samples in all directions
        let g_x = line[col][CFA_COLOR_G];
        let g_w = line[col - 1][CFA_COLOR_G];
        let g_e = line[col + 1][CFA_COLOR_G];
        let g_n = n1[col][CFA_COLOR_G];
        let g_s = s1[col][CFA_COLOR_G];
        // Horizontal samples for channel (R or B)
        let h_w = line[col - 1][h_ch];
        let h_e = line[col + 1][h_ch];
        // Vertial samples for channel (R or B)
        let v_n = n1[col][v_ch];
        let v_s = s1[col][v_ch];

        line[col][h_ch] = hue_transit(g_w, g_x, g_e, h_w, h_e);
        line[col][v_ch] = hue_transit(g_n, g_x, g_s, v_n, v_s);
      }
    }
  });
}

/// PPG Demosaic: Interpolate R/B channel at non-G channels
///
/// It writes the missing one of R and B at R/B sites and reads G and the
/// samples of R/B sites, so it doesn't read values it writes.
#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn interpolate_rb_at_non_green(img: &mut RgbF32, shifted: &CFA) {
  let w = img.width;

  par_interior_rows(img, |row, [n2, n1], line, [s1, s2]| {
    for col in 3..w - 3 {
      if shifted.color_at(row, col) != CFA_COLOR_G {
        let x_ch = shifted.color_at(row, col); // current
        let y_ch = if x_ch == CFA_COLOR_R { CFA_COLOR_B } else { CFA_COLOR_R };

        let y_ne_1 = n1[col + 1][y_ch];
        let y_sw_1 = s1[col - 1][y_ch];
        let x_ne_2 = n2[col + 2][x_ch];
        let x_center = line[col][x_ch];
        let x_sw_2 = s2[col - 2][x_ch];
        let g_ne_1 = n1[col + 1][CFA_COLOR_G];
        let g_center = line[col][CFA_COLOR_G];
        let g_sw_1 = s1[col - 1][CFA_COLOR_G];
        let y_nw_1 = n1[col - 1][y_ch];
        let y_se_1 = s1[col + 1][y_ch];
        let x_nw_2 = n2[col - 2][x_ch];
        let x_se_2 = s2[col + 2][x_ch];
        let g_nw_1 = n1[col - 1][CFA_COLOR_G];
        let g_se_1 = s1[col + 1][CFA_COLOR_G];

        let ne = (y_ne_1 - y_sw_1).abs() + (x_ne_2 - x_center).abs() + (x_center - x_sw_2).abs() + (g_ne_1 - g_center).abs() + (g_center - g_sw_1).abs();

        let nw = (y_nw_1 - y_se_1).abs() + (x_nw_2 - x_center).abs() + (x_center - x_se_2).abs() + (g_nw_1 - g_center).abs() + (g_center - g_se_1).abs();

        line[col][y_ch] = if ne < nw {
          hue_transit(g_ne_1, g_center, g_sw_1, y_ne_1, y_sw_1)
        } else {
          hue_transit(g_nw_1, g_center, g_se_1, y_nw_1, y_se_1)
        };
      }
    }
  });
}

/// Rows of the bands of [`par_interior_rows()`]
const BAND_ROWS: usize = 32;

/// Call `op(row, above, line, below)` in parallel for the rows 3 pixels away
/// from the borders of `img`: `line` is the row, `above` are the two rows
/// above it, from the top, and `below` the two rows below it.
///
/// The image is split into bands of rows, which are processed row by row. In
/// two rounds, the bands of one parity are written while the bands of the
/// other parity are read. So `op` must not read values of `above` and `below`
/// that it writes, or the result depends on the order of the rows.
fn par_interior_rows<F>(img: &mut RgbF32, op: F)
where
  F: Fn(usize, [&[[f32; 3]]; 2], &mut [[f32; 3]], [&[[f32; 3]]; 2]) + Sync,
{
  let (w, h) = (img.width, img.height);
  if w <= 6 || h <= 6 {
    return;
  }
  // Samples of row `i` of a band
  let row_of = |i: usize| i * w..(i + 1) * w;
  for parity in 0..2 {
    let mut bands = Vec::new();
    // Bands of the other parity, band b is at index b / 2
    let mut neighbours = Vec::new();
    for (band, rows) in img.pixels_mut().chunks_mut(BAND_ROWS * w).enumerate() {
      if band % 2 == parity {
        bands.push((band, rows));
      } else {
        neighbours.push(&*rows);
      }
    }
    bands.into_par_iter().for_each(|(band, rows)| {
      // Bands next to this one, the first band has none above, the last none below
      let (prev_band, next_band) = (band.checked_sub(1), band + 1);
      let (prev, next) = (prev_band.map(|prev| neighbours[prev / 2]), neighbours.get(next_band / 2).copied());
      let count = rows.len() / w;
      for i in 0..count {
        let row = band * BAND_ROWS + i;
        if row < 3 || row >= h - 3 {
          continue;
        }
        let (before, rest) = rows.split_at_mut(i * w);
        let (line, after) = rest.split_at_mut(w);
        let (before, after) = (&*before, &*after);
        // Rows outside of the band are in the bands next to it
        let above = [2, 1].map(|k| match i.checked_sub(k) {
          Some(j) => &before[row_of(j)],
          None => {
            let prev = prev.expect("Band above");
            &prev[row_of(prev.len() / w + i - k)]
          }
        });
        let below = [1, 2].map(|k| match (i + k).checked_sub(count) {
          None => &after[row_of(k - 1)],
          Some(j) => {
            let next = next.expect("Band below");
            &next[row_of(j)]
          }
        });
        op(row, above, line, below);
      }
    });
  }
}

/// PPG helper procedure to calculate hue transit
///
/// `v1` and `v3` of a color plane are interpolated at the position of `l2`,
/// `l1` to `l3` are the green values. Unless green changes monotonically, the
/// color difference to green is interpolated. Lin's description damps the
/// green term by 1/4, the full color difference (1/2, like dcraw's PPG) keeps
/// neutral edges of white balanced data neutral.
#[inline(always)]
fn hue_transit(l1: f32, l2: f32, l3: f32, v1: f32, v3: f32) -> f32 {
  if (l1 < l2 && l2 < l3) || (l1 > l2 && l2 > l3) {
    v1 + (v3 - v1) * (l2 - l1) / (l3 - l1)
  } else {
    (v1 + v3) / 2.0 + (l2 * 2.0 - l1 - l3) / 2.0
  }
}
