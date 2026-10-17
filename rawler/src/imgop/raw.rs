// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::xyz::Illuminant;
use crate::imgop::Rect;
use crate::imgop::matrix::{multiply, normalize, pseudo_inverse};
use crate::imgop::xyz::SRGB_TO_XYZ_D65;
use crate::pixarray::{Color2D, RgbF32};
use crate::rawimage::{BlackLevel, RawPhotometricInterpretation, WhiteLevel};

use multiversion::multiversion;
use rayon::prelude::*;

/// Conversion matrix for a specific illuminant
#[derive(Debug)]
pub struct ColorMatrix {
  pub illuminant: Illuminant,
  pub matrix: [[f32; 3]; 4],
}

/// Parameters for raw image development
#[derive(Debug)]
pub struct DevelopParams {
  pub width: usize,
  pub height: usize,
  pub cpp: usize,
  pub color_matrices: Vec<ColorMatrix>,
  pub whitelevel: WhiteLevel,
  pub blacklevel: BlackLevel,
  //pub pattern: Option<BayerPattern>,
  pub photometric: RawPhotometricInterpretation,
  pub wb_coeff: [f32; 4],
  pub active_area: Option<Rect>,
  pub crop_area: Option<Rect>,
  //pub gamma: f32,
}

/// CLip only underflow values < 0.0
pub fn clip_negative<const N: usize>(pix: &[f32; N]) -> [f32; N] {
  pix.map(|p| if p < 0.0 { 0.0 } else { p })
}

/// Clip only overflow values > 1.0
pub fn clip_overflow<const N: usize>(pix: &[f32; N]) -> [f32; N] {
  pix.map(|p| if p > 1.0 { 1.0 } else { p })
}

/// Clip into range of 0.0 - 1.0
pub fn clip<const N: usize>(pix: &[f32; N]) -> [f32; N] {
  clip_range(pix, 0.0, 1.0)
}

/// Clip into range of lower and upper bound
pub fn clip_range<const N: usize>(pix: &[f32; N], lower: f32, upper: f32) -> [f32; N] {
  pix.map(|p| clip_value(p, lower, upper))
}

pub fn clip_value(p: f32, lower: f32, upper: f32) -> f32 {
  if p < lower {
    lower
  } else if p > upper {
    upper
  } else {
    p
  }
}

/// Clip pixel with N components by:
/// 1. Normalize pixel by max(pix) if any component is > 1.0
/// 2. Compute euclidean norm of the pixel, normalized by sqrt(N)
/// 3. Compute channel-wise average of normalized pixel + euclidean norm
pub fn clip_euclidean_norm_avg<const N: usize>(pix: &[f32; N]) -> [f32; N] {
  let pix = clip_negative(pix);
  let max_val = pix.iter().copied().reduce(f32::max).unwrap_or(f32::NAN);
  if max_val > 1.0 {
    // Retains color
    let color = pix.map(|p| p / max_val);
    // Euclidean norm
    let eucl = pix.map(|p| p.powi(2)).iter().sum::<f32>().sqrt() / (N as f32).sqrt();
    // Take average of both
    color.map(|p| (p + eucl) / 2.0)
  } else {
    pix
  }
}

/// Correct data by blacklevel and whitelevel on CFA (bayer) data.
///
/// The output is between 0.0 .. 1.0.
///
/// This version is optimized vor vectorization, so please check
/// modifications on godbolt before committing.
#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn correct_blacklevel_channels<const CH: usize>(raw: &mut [f32], blacklevel: &[f32; CH], whitelevel: &[f32; CH]) {
  // max value can be pre-computed for all channels.
  let mut max = *whitelevel;
  max.iter_mut().enumerate().for_each(|(i, x)| *x -= blacklevel[i]);

  let clip = |v: f32| {
    if v.is_sign_negative() { 0.0 } else { v }
  };
  if CH == 1 {
    let max = max[0];
    let blacklevel = blacklevel[0];
    raw.iter_mut().for_each(|p| *p = clip(*p - blacklevel) / max);
  } else {
    raw.chunks_exact_mut(CH).for_each(|block| {
      for i in 0..CH {
        block[i] = clip(block[i] - blacklevel[i]) / max[i];
      }
    });
  }
}

/// Correct data by blacklevel and whitelevel on CFA (bayer) data.
///
/// The output is between 0.0 .. 1.0.
///
/// This version is optimized vor vectorization, so please check
/// modifications on godbolt before committing.
#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
pub fn correct_blacklevel(raw: &mut [f32], blacklevel: &[f32], whitelevel: &[f32]) {
  match (blacklevel.len(), whitelevel.len()) {
    (1, 1) => correct_blacklevel_channels::<1>(
      raw,
      blacklevel.try_into().expect("Array size mismatch"),
      whitelevel.try_into().expect("Array size mismatch"),
    ),
    (3, 3) => correct_blacklevel_channels::<3>(
      raw,
      blacklevel.try_into().expect("Array size mismatch"),
      whitelevel.try_into().expect("Array size mismatch"),
    ),
    (a, b) if a == b => {
      // max value can be pre-computed for all channels.
      let mut max = whitelevel.to_vec();
      max.iter_mut().enumerate().for_each(|(i, x)| *x -= blacklevel[i]);

      let clip = |v: f32| {
        if v.is_sign_negative() { 0.0 } else { v }
      };

      let ch = blacklevel.len();
      raw.chunks_exact_mut(ch).for_each(|block| {
        for i in 0..ch {
          block[i] = clip(block[i] - blacklevel[i]) / max[i];
        }
      });
    }
    _ => log::warn!(
      "Blacklevel ({}) and Whitelevel ({}) count mismatch, skipping correction",
      blacklevel.len(),
      whitelevel.len()
    ),
  }
}

/// Correct raw data using a repeating black level plus DNG per-column and per-row deltas.
///
/// DNG black-level patterns and delta tables are relative to the top-left corner of the
/// active area. Normalization uses the maximum computed black level for each sample plane.
pub(crate) fn correct_blacklevel_with_deltas(
  raw: &mut [f32],
  width: usize,
  height: usize,
  cpp: usize,
  blacklevel: &BlackLevel,
  whitelevel: &[f32],
  active_area: Rect,
) {
  assert_eq!(raw.len(), width * height * cpp);
  assert_eq!(blacklevel.cpp, cpp);
  assert_eq!(whitelevel.len(), cpp);
  assert!(active_area.d.w > 0 && active_area.d.h > 0);
  assert!(active_area.p.x + active_area.d.w <= width);
  assert!(active_area.p.y + active_area.d.h <= height);
  assert!(blacklevel.delta_h.is_empty() || blacklevel.delta_h.len() == active_area.d.w);
  assert!(blacklevel.delta_v.is_empty() || blacklevel.delta_v.len() == active_area.d.h);

  let delta_h: Vec<f32> = if blacklevel.delta_h.is_empty() {
    vec![0.0; active_area.d.w]
  } else {
    blacklevel.delta_h.iter().map(|value| value.n as f32 / value.d as f32).collect()
  };
  let delta_v: Vec<f32> = if blacklevel.delta_v.is_empty() {
    vec![0.0; active_area.d.h]
  } else {
    blacklevel.delta_v.iter().map(|value| value.n as f32 / value.d as f32).collect()
  };

  let mut max_delta_h = vec![f32::NEG_INFINITY; blacklevel.width];
  for (column, delta) in delta_h.iter().enumerate() {
    max_delta_h[column % blacklevel.width] = max_delta_h[column % blacklevel.width].max(*delta);
  }

  let mut max_delta_v = vec![f32::NEG_INFINITY; blacklevel.height];
  for (row, delta) in delta_v.iter().enumerate() {
    max_delta_v[row % blacklevel.height] = max_delta_v[row % blacklevel.height].max(*delta);
  }

  let mut maximum_blacklevel = vec![f32::NEG_INFINITY; cpp];
  for (pattern_row, row_delta) in max_delta_v.iter().enumerate() {
    if !row_delta.is_finite() {
      continue;
    }
    for (pattern_column, column_delta) in max_delta_h.iter().enumerate() {
      if !column_delta.is_finite() {
        continue;
      }
      for (sample, maximum) in maximum_blacklevel.iter_mut().enumerate() {
        let index = (pattern_row * blacklevel.width + pattern_column) * cpp + sample;
        *maximum = maximum.max(blacklevel.levels[index].as_f32() + column_delta + row_delta);
      }
    }
  }

  let scale: Vec<f32> = whitelevel
    .iter()
    .zip(maximum_blacklevel.iter())
    .map(|(white, black)| 1.0 / (white - black))
    .collect();
  let active_right = active_area.p.x + active_area.d.w;
  let active_bottom = active_area.p.y + active_area.d.h;
  let pattern_origin_x = active_area.p.x % blacklevel.width;
  let pattern_origin_y = active_area.p.y % blacklevel.height;

  raw.par_chunks_exact_mut(width * cpp).enumerate().for_each(|(row, line)| {
    let pattern_row = (row + blacklevel.height - pattern_origin_y) % blacklevel.height;
    let row_delta = if row >= active_area.p.y && row < active_bottom {
      delta_v[row - active_area.p.y]
    } else {
      0.0
    };

    line.chunks_exact_mut(cpp).enumerate().for_each(|(column, pixel)| {
      let pattern_column = (column + blacklevel.width - pattern_origin_x) % blacklevel.width;
      let column_delta = if column >= active_area.p.x && column < active_right {
        delta_h[column - active_area.p.x]
      } else {
        0.0
      };

      for sample in 0..cpp {
        let index = (pattern_row * blacklevel.width + pattern_column) * cpp + sample;
        let black = blacklevel.levels[index].as_f32() + column_delta + row_delta;
        pixel[sample] = (pixel[sample] - black).max(0.0) * scale[sample];
      }
    });
  });
}

/// Correct data by blacklevel and whitelevel on CFA (bayer) data.
///
/// The output is between 0.0 .. 1.0.
///
/// This version is optimized vor vectorization, so please check
/// modifications on godbolt before committing.
#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
pub fn correct_blacklevel_cfa(raw: &mut [f32], width: usize, _height: usize, blacklevel: &[f32; 4], whitelevel: &[f32; 4]) {
  //assert_eq!(width % 2, 0, "width is {}", width);
  //assert_eq!(height % 2, 0, "height is {}", height);
  // max value can be pre-computed for all channels.
  let max = [
    whitelevel[0] - blacklevel[0],
    whitelevel[1] - blacklevel[1],
    whitelevel[2] - blacklevel[2],
    whitelevel[3] - blacklevel[3],
  ];
  let clip = |v: f32| {
    if v.is_sign_negative() { 0.0 } else { v }
  };
  // Process two bayer lines at once.
  raw.par_chunks_exact_mut(width * 2).for_each(|lines| {
    // It's bayer data, so we have two lines for sure.
    let (line0, line1) = lines.split_at_mut(width);
    //line0.array_chunks_mut::<2>().zip(line1.array_chunks_mut::<2>()).for_each(|(a, b)| {
    line0.chunks_exact_mut(2).zip(line1.chunks_exact_mut(2)).for_each(|(a, b)| {
      a[0] = clip(a[0] - blacklevel[0]) / max[0];
      a[1] = clip(a[1] - blacklevel[1]) / max[1];
      b[0] = clip(b[0] - blacklevel[2]) / max[2];
      b[1] = clip(b[1] - blacklevel[3]) / max[3];
    });
  });
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::formats::tiff::SRational;
  use approx::assert_abs_diff_eq;

  #[test]
  fn corrects_signed_dng_blacklevel_deltas() {
    let mut blacklevel = BlackLevel::new(&[10_u16], 1, 1, 1);
    blacklevel.delta_h = vec![SRational::new(0, 1), SRational::new(1, 1), SRational::new(2, 1)];
    blacklevel.delta_v = vec![SRational::new(-1, 1), SRational::new(3, 1)];

    let mut raw = vec![9.0, 10.0, 11.0, 13.0, 56.5, 100.0];
    correct_blacklevel_with_deltas(
      &mut raw,
      3,
      2,
      1,
      &blacklevel,
      &[100.0],
      Rect::new(crate::imgop::Point::zero(), crate::imgop::Dim2::new(3, 2)),
    );

    assert_abs_diff_eq!(raw[0], 0.0);
    assert_abs_diff_eq!(raw[1], 0.0);
    assert_abs_diff_eq!(raw[2], 0.0);
    assert_abs_diff_eq!(raw[3], 0.0);
    assert_abs_diff_eq!(raw[4], 0.5, epsilon = 1e-6);
    assert_abs_diff_eq!(raw[5], 1.0, epsilon = 1e-6);
  }

  #[test]
  fn anchors_dng_blacklevel_pattern_to_active_area() {
    let mut blacklevel = BlackLevel::new(&[10_u16, 20, 30, 40], 2, 2, 1);
    blacklevel.delta_h = vec![SRational::new(0, 1); 2];
    blacklevel.delta_v = vec![SRational::new(0, 1); 2];

    let mut raw = vec![0.0; 16];
    raw[5] = 10.0;
    raw[6] = 20.0;
    raw[9] = 30.0;
    raw[10] = 100.0;
    correct_blacklevel_with_deltas(
      &mut raw,
      4,
      4,
      1,
      &blacklevel,
      &[100.0],
      Rect::new(crate::imgop::Point::new(1, 1), crate::imgop::Dim2::new(2, 2)),
    );

    assert_abs_diff_eq!(raw[5], 0.0);
    assert_abs_diff_eq!(raw[6], 0.0);
    assert_abs_diff_eq!(raw[9], 0.0);
    assert_abs_diff_eq!(raw[10], 1.0, epsilon = 1e-6);
  }
}

#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
pub(crate) fn map_3ch_to_rgb(src: &Color2D<f32, 3>, wb_coeff: &[f32; 4], xyz2cam: [[f32; 3]; 4]) -> RgbF32 {
  let rgb2cam = normalize(multiply(&xyz2cam, &SRGB_TO_XYZ_D65));
  let cam2rgb = pseudo_inverse(rgb2cam);

  let mut out = Vec::with_capacity(src.data.len());

  src
    .pixels()
    .par_iter()
    .map(|pix| {
      // We apply wb coeffs on the fly
      let r = pix[0] * wb_coeff[0];
      let g = pix[1] * wb_coeff[1];
      let b = pix[2] * wb_coeff[2];
      let srgb = [
        cam2rgb[0][0] * r + cam2rgb[0][1] * g + cam2rgb[0][2] * b,
        cam2rgb[1][0] * r + cam2rgb[1][1] * g + cam2rgb[1][2] * b,
        cam2rgb[2][0] * r + cam2rgb[2][1] * g + cam2rgb[2][2] * b,
      ];
      clip_euclidean_norm_avg(&srgb)
    })
    .collect_into_vec(&mut out);

  RgbF32::new_with(out, src.width, src.height)
}

#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
pub(crate) fn map_4ch_to_rgb(src: &Color2D<f32, 4>, wb_coeff: &[f32; 4], xyz2cam: [[f32; 3]; 4]) -> RgbF32 {
  let rgb2cam = normalize(multiply(&xyz2cam, &SRGB_TO_XYZ_D65));
  let cam2rgb = pseudo_inverse(rgb2cam);

  let mut out = Vec::with_capacity(src.data.len());

  src
    .pixels()
    .par_iter()
    .map(|pix| {
      // We apply wb coeffs on the fly
      let ch0 = pix[0] * wb_coeff[0];
      let ch1 = pix[1] * wb_coeff[1];
      let ch2 = pix[2] * wb_coeff[2];
      let ch3 = pix[3] * wb_coeff[3];
      let srgb = [
        cam2rgb[0][0] * ch0 + cam2rgb[0][1] * ch1 + cam2rgb[0][2] * ch2 + cam2rgb[0][3] * ch3,
        cam2rgb[1][0] * ch0 + cam2rgb[1][1] * ch1 + cam2rgb[1][2] * ch2 + cam2rgb[1][3] * ch3,
        cam2rgb[2][0] * ch0 + cam2rgb[2][1] * ch1 + cam2rgb[2][2] * ch2 + cam2rgb[2][3] * ch3,
      ];
      clip_euclidean_norm_avg(&srgb)
    })
    .collect_into_vec(&mut out);

  RgbF32::new_with(out, src.width, src.height)
}

/// Collect iterator into array
pub fn collect_array<T, I, const N: usize>(itr: I) -> [T; N]
where
  T: Default + Copy,
  I: IntoIterator<Item = T>,
{
  let mut res = [T::default(); N];
  for (it, elem) in res.iter_mut().zip(itr) {
    *it = elem
  }

  res
}

/// Calculate the multiplicative invert of an array
/// (sum of each row equals to 1.0)
pub fn mul_invert_array<const N: usize>(a: &[f32; N]) -> [f32; N] {
  let mut b = [f32::default(); N];
  b.iter_mut().zip(a.iter()).for_each(|(x, y)| *x = 1.0 / y);
  b
}

pub fn rotate_90(src: &[u16], dst: &mut [u16], width: usize, height: usize) {
  let dst = &mut dst[..src.len()]; // optimize len hints for compiler
  let owidth = height;
  for (row, line) in src.chunks_exact(width).enumerate() {
    for (col, pix) in line.iter().enumerate() {
      let orow = col;
      let ocol = (owidth - 1) - row; // inverse
      dst[orow * owidth + ocol] = *pix;
    }
  }
}
