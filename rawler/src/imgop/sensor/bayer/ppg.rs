// SPDX-License-Identifier: LGPL-2.1
// Copyright 2022 Daniel Vogelbacher <daniel@chaospixel.com>

use multiversion::multiversion;
use rayon::prelude::*;
use std::{ops::Add, time::Instant};

use crate::{
  cfa::{CFA, CFA_COLOR_B, CFA_COLOR_G, CFA_COLOR_R},
  imgop::{Dim2, Rect},
  pixarray::{Rgb2D, RgbF32},
};

/// PPG demosaic a raw image (f32 values)
///
/// The PPG - Pattern Pixel Grouping - alogrithm was developed by Chuan-kai Lin
/// Source on Internet Archive:
/// https://web.archive.org/web/20160923211135/https://sites.google.com/site/chklin/demosaic/
///
/// This is a simple algorithm but provides acceptable results.
///
/// # Panics
///
/// This function panics for CFA pattern that are not RGGB or variants. You need
/// to check the pattern before calling.
pub fn demosaic_ppg(raw: &[f32], dim: Dim2, cfa_orig: CFA, roi: Rect) -> RgbF32 {
  // PPG can only applied to pure RGGB or variants.
  if !cfa_orig.is_rgb() {
    panic!("CFA pattern '{}' is not a RGB pattern, can not demosaic with PPG", cfa_orig);
  }
  // Measure time
  let now = Instant::now();

  // The ROI changes the pattern if not perfectly aligned on the origin pattern
  let cfa_roi = cfa_orig.shift(roi.p.x, roi.p.y);

  // Expand the bayer data to full RGB channel image, but only for ROI
  let mut rgb = super::expand_bayer_rgb(raw, dim, cfa_orig, roi);

  // Now interpolate the missing channels
  interpolate_borders(&mut rgb, &cfa_roi);
  interpolate_green(&mut rgb, &cfa_roi);
  interpolate_rb_at_green(&mut rgb, &cfa_roi);
  interpolate_rb_at_non_green(&mut rgb, &cfa_roi);

  log::debug!("PPG total debayer time: {:.5}s", now.elapsed().as_secs_f32());
  rgb
}

/// PPG Demosaic: Interpolate borders
/// We take 3 pixels on each border and interpolate by bilinear interpolation.
/// Bilinear interpolation is done by `(x1 + x2 + x3 + x4) / 4` or a
/// reduced sum and count if not all samples are available like on borders.
///
/// Basically, for each pixel position, we iterate around it and collect
/// all channel values. Then apply interpolation to calculate the missing
/// two channel colors for the origin pixel position.
#[multiversion]
#[clone(target = "[x86|x86_64]+avx+avx2")]
#[clone(target = "x86+sse")]
fn interpolate_borders(input: &mut Rgb2D<f32>, shifted: &CFA) {
  let w = input.width;
  let h = input.height;
  // Iterate over all rows
  for row in 0..h {
    let mut col = 0;
    while col < w {
      // Full process first 3 and last 3 rows, process only left and right 3 pixels for all other
      if col == 3 && row >= 3 && row < h - 3 {
        col = w - 3 // go to right border
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
#[multiversion]
#[clone(target = "[x86|x86_64]+avx+avx2")]
#[clone(target = "x86+sse")]
fn interpolate_green(img: &mut Rgb2D<f32>, shifted: &CFA) {
  let w = img.width;
  let h = img.height;

  // Secondary data pointer, so we can have a mutable subslice reference
  // inside the par_* methods and a second immutable reference to the whole
  // image. This is not possible in Rust without unsafe code.
  let dataptr = img.data_ptr();

  img.pixels_mut().par_chunks_exact_mut(w).enumerate().skip(3).take(h - 6).for_each(|(row, buf)| {
    for (col, pixel) in buf.iter_mut().enumerate().skip(3).take(w - 6) {
      if shifted.color_at(row, col) != CFA_COLOR_G {
        let ch = shifted.color_at(row, col); // It's R or B
        let x = pixel[ch];
        let n_1 = unsafe { dataptr.at(row - 1, col)[CFA_COLOR_G] };
        let n_2 = unsafe { dataptr.at(row - 2, col)[ch] };
        let e_1 = unsafe { dataptr.at(row, col + 1)[CFA_COLOR_G] };
        let e_2 = unsafe { dataptr.at(row, col + 2)[ch] };
        let s_1 = unsafe { dataptr.at(row + 1, col)[CFA_COLOR_G] };
        let s_2 = unsafe { dataptr.at(row + 2, col)[ch] };
        let w_1 = unsafe { dataptr.at(row, col - 1)[CFA_COLOR_G] };
        let w_2 = unsafe { dataptr.at(row, col - 2)[ch] };

        // Calculate the gradients for each direction.
        let n = (x - n_2).abs() * 2.0 + (n_1 - s_1);
        let e = (x - e_2).abs() * 2.0 + (w_1 - e_1);
        let w = (x - w_2).abs() * 2.0 + (w_1 - e_1);
        let s = (x - s_2).abs() * 2.0 + (n_1 - s_1);

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
        pixel[CFA_COLOR_G] = p_green;
      }
    }
  });
}

/// PPG Demosaic: Interpolate R/B channel at G channels
#[multiversion]
#[clone(target = "[x86|x86_64]+avx+avx2")]
#[clone(target = "x86+sse")]
fn interpolate_rb_at_green(img: &mut Rgb2D<f32>, shifted: &CFA) {
  let w = img.width;
  let h = img.height;

  // Secondary data pointer, so we can have a mutable subslice reference
  // inside the par_* methods and a second immutable reference to the whole
  // image. This is not possible in Rust without unsafe code.
  let dataptr = img.data_ptr();

  img.pixels_mut().par_chunks_exact_mut(w).enumerate().skip(3).take(h - 6).for_each(|(row, buf)| {
    for (col, pixel) in buf.iter_mut().enumerate().skip(3).take(w - 6) {
      if shifted.color_at(row, col) == CFA_COLOR_G {
        let h_ch = shifted.color_at(row, col + 1); // horizontal corresponding channel
        let v_ch = shifted.color_at(row + 1, col); // vertical corresponding channel

        // Green samples in all directions
        let g_x = pixel[CFA_COLOR_G];
        let g_w = unsafe { dataptr.at(row, col - 1)[CFA_COLOR_G] };
        let g_e = unsafe { dataptr.at(row, col + 1)[CFA_COLOR_G] };
        let g_n = unsafe { dataptr.at(row - 1, col)[CFA_COLOR_G] };
        let g_s = unsafe { dataptr.at(row + 1, col)[CFA_COLOR_G] };
        // Horizontal samples for channel (R or B)
        let h_w = unsafe { dataptr.at(row, col - 1).get_unchecked(h_ch) };
        let h_e = unsafe { dataptr.at(row, col + 1).get_unchecked(h_ch) };
        // Vertial samples for channel (R or B)
        let v_n = unsafe { dataptr.at(row - 1, col).get_unchecked(v_ch) };
        let v_s = unsafe { dataptr.at(row + 1, col).get_unchecked(v_ch) };

        *unsafe { pixel.get_unchecked_mut(h_ch) } = hue_transit(g_w, g_x, g_e, *h_w, *h_e);
        *unsafe { pixel.get_unchecked_mut(v_ch) } = hue_transit(g_n, g_x, g_s, *v_n, *v_s);
      }
    }
  });
}

/// PPG Demosaic: Interpolate R/B channel at non-G channels
#[multiversion]
#[clone(target = "[x86|x86_64]+avx+avx2")]
#[clone(target = "x86+sse")]
fn interpolate_rb_at_non_green(img: &mut Rgb2D<f32>, shifted: &CFA) {
  let w = img.width;
  let h = img.height;

  // Secondary data pointer, so we can have a mutable subslice reference
  // inside the par_* methods and a second immutable reference to the whole
  // image. This is not possible in Rust without unsafe code.
  let dataptr = img.data_ptr();

  img.pixels_mut().par_chunks_exact_mut(w).enumerate().skip(3).take(h - 6).for_each(|(row, buf)| {
    for (col, pixel) in buf.iter_mut().enumerate().skip(3).take(w - 6) {
      if shifted.color_at(row, col) != CFA_COLOR_G {
        let x_ch = shifted.color_at(row, col); // current
        let y_ch = if x_ch == CFA_COLOR_R { CFA_COLOR_B } else { CFA_COLOR_R };

        let y_ne_1 = unsafe { dataptr.at(row - 1, col + 1)[y_ch] };
        let y_sw_1 = unsafe { dataptr.at(row + 1, col - 1)[y_ch] };
        let x_ne_2 = unsafe { dataptr.at(row - 2, col + 2)[x_ch] };
        let x_center = pixel[x_ch];
        let x_sw_2 = unsafe { dataptr.at(row + 2, col - 2)[x_ch] };
        let g_ne_1 = unsafe { dataptr.at(row - 1, col + 1)[CFA_COLOR_G] };
        let g_center = pixel[CFA_COLOR_G];
        let g_sw_1 = unsafe { dataptr.at(row + 1, col - 1)[CFA_COLOR_G] };
        let y_nw_1 = unsafe { dataptr.at(row - 1, col - 1)[y_ch] };
        let y_se_1 = unsafe { dataptr.at(row + 1, col + 1)[y_ch] };
        let x_nw_2 = unsafe { dataptr.at(row - 2, col - 2)[x_ch] };
        let x_se_2 = unsafe { dataptr.at(row + 2, col + 2)[x_ch] };
        let g_nw_1 = unsafe { dataptr.at(row - 1, col - 1)[CFA_COLOR_G] };
        let g_se_1 = unsafe { dataptr.at(row + 1, col + 1)[CFA_COLOR_G] };

        let ne = (y_ne_1 - y_sw_1).abs() + (x_ne_2 - x_center).abs() + (x_center - x_sw_2).abs() + (g_ne_1 - g_center).abs() + (g_center - g_sw_1).abs();

        let nw = (y_nw_1 - y_se_1).abs() + (x_nw_2 - x_center).abs() + (x_center - x_se_2).abs() + (g_nw_1 + g_center).abs() + (g_center - g_se_1).abs();

        pixel[y_ch] = if ne < nw {
          hue_transit(g_ne_1, g_center, g_sw_1, y_ne_1, y_sw_1)
        } else {
          hue_transit(g_nw_1, g_center, g_se_1, y_nw_1, y_se_1)
        };
      }
    }
  });
}

/// PPG helper procedure to calculate hue transit
#[inline(always)]
fn hue_transit(l1: f32, l2: f32, l3: f32, v1: f32, v3: f32) -> f32 {
  if (l1 < l2 && l2 < l3) || (l1 > l2 && l2 > l3) {
    v1 + (v3 - v1) * (l2 - l1) / (l3 - l1)
  } else {
    (v1 + v3) / 2.0 + (l2 * 2.0 - l1 - l3) / 4.0
  }
}
