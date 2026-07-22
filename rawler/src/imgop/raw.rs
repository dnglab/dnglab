// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::xyz::Illuminant;
use crate::imgop::matrix::{multiply, normalize, pseudo_inverse};
use crate::imgop::xyz::SRGB_TO_XYZ_D65;
use crate::imgop::{Point, Rect};
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

/// Correct black and white levels for an interleaved linear raw image.
///
/// DNG black levels are stored in row-column-sample order and may repeat over
/// multiple pixels, while white levels are stored once per sample plane. The
/// repeat pattern is relative to `blacklevel_origin` (the top-left corner of
/// the DNG ActiveArea).
pub fn correct_blacklevel_linear(
  raw: &mut [f32],
  width: usize,
  height: usize,
  cpp: usize,
  blacklevel: &BlackLevel,
  whitelevel: &WhiteLevel,
  blacklevel_origin: Point,
) -> crate::Result<()> {
  if width == 0 || height == 0 || cpp == 0 {
    return Err(format!("Invalid linear raw dimensions: {width}x{height} with {cpp} components per pixel").into());
  }

  let row_len = width
    .checked_mul(cpp)
    .ok_or_else(|| format!("Linear raw row size overflow: width {width}, cpp {cpp}"))?;
  let expected_raw_len = row_len
    .checked_mul(height)
    .ok_or_else(|| format!("Linear raw image size overflow: {width}x{height} with {cpp} components per pixel"))?;
  if raw.len() != expected_raw_len {
    return Err(format!("Linear raw data length mismatch: expected {expected_raw_len}, found {}", raw.len()).into());
  }

  if blacklevel.width == 0 || blacklevel.height == 0 || blacklevel.cpp == 0 {
    return Err(
      format!(
        "Invalid black level repeat dimensions: {}x{} with {} components per pixel",
        blacklevel.width, blacklevel.height, blacklevel.cpp
      )
      .into(),
    );
  }
  if blacklevel.cpp != 1 && blacklevel.cpp != cpp {
    return Err(format!("Black level component count mismatch: expected 1 or {cpp}, found {}", blacklevel.cpp).into());
  }

  let expected_blacklevel_len = blacklevel
    .width
    .checked_mul(blacklevel.height)
    .and_then(|count| count.checked_mul(blacklevel.cpp))
    .ok_or_else(|| {
      format!(
        "Black level repeat size overflow: {}x{} with {} components per pixel",
        blacklevel.width, blacklevel.height, blacklevel.cpp
      )
    })?;
  if blacklevel.levels.len() != expected_blacklevel_len {
    return Err(
      format!(
        "Black level data length mismatch: expected {expected_blacklevel_len}, found {}",
        blacklevel.levels.len()
      )
      .into(),
    );
  }

  let whitelevels = match whitelevel.0.as_slice() {
    [level] => vec![*level as f32; cpp],
    levels if levels.len() == cpp => levels.iter().map(|level| *level as f32).collect(),
    levels => {
      return Err(format!("White level component count mismatch: expected 1 or {cpp}, found {}", levels.len()).into());
    }
  };
  let blacklevels = blacklevel.as_vec();
  if blacklevels.iter().any(|level| !level.is_finite()) {
    return Err("Black level contains a non-finite value".into());
  }

  // DNG normalization uses the maximum computed black level for each sample
  // plane, even when the repeating pattern contains different local values.
  let mut max_blacklevels = vec![f32::NEG_INFINITY; cpp];
  for cell in blacklevels.chunks_exact(blacklevel.cpp) {
    for channel in 0..cpp {
      let black_channel = if blacklevel.cpp == 1 { 0 } else { channel };
      max_blacklevels[channel] = max_blacklevels[channel].max(cell[black_channel]);
    }
  }
  let scales: Vec<f32> = whitelevels
    .iter()
    .zip(&max_blacklevels)
    .enumerate()
    .map(|(channel, (white, black))| {
      let scale = *white - *black;
      if scale.is_finite() && scale > 0.0 {
        Ok(scale)
      } else {
        Err(format!("Invalid black/white level range for channel {channel}: black {black}, white {white}"))
      }
    })
    .collect::<std::result::Result<_, _>>()?;

  // Most linear DNGs, including Samsung's 2x2 patterns, repeat identical
  // values. Keep that common case on the vectorized per-channel path.
  let first_cell = &blacklevels[..blacklevel.cpp];
  if blacklevels.chunks_exact(blacklevel.cpp).all(|cell| cell == first_cell) {
    let flat_blacklevels: Vec<f32> = (0..cpp).map(|channel| first_cell[if blacklevel.cpp == 1 { 0 } else { channel }]).collect();
    correct_blacklevel(raw, &flat_blacklevels, &whitelevels);
    return Ok(());
  }

  let origin_x = blacklevel_origin.x % blacklevel.width;
  let origin_y = blacklevel_origin.y % blacklevel.height;
  raw.par_chunks_exact_mut(row_len).enumerate().for_each(|(y, row)| {
    let repeat_y = repeat_position(y, origin_y, blacklevel.height);
    row.chunks_exact_mut(cpp).enumerate().for_each(|(x, pixel)| {
      let repeat_x = repeat_position(x, origin_x, blacklevel.width);
      let cell_offset = (repeat_y * blacklevel.width + repeat_x) * blacklevel.cpp;
      for channel in 0..cpp {
        let black_channel = if blacklevel.cpp == 1 { 0 } else { channel };
        let corrected = pixel[channel] - blacklevels[cell_offset + black_channel];
        pixel[channel] = if corrected.is_sign_negative() { 0.0 } else { corrected / scales[channel] };
      }
    });
  });

  Ok(())
}

#[inline]
fn repeat_position(position: usize, origin: usize, repeat: usize) -> usize {
  let position = position % repeat;
  if position >= origin {
    position - origin
  } else {
    repeat - (origin - position)
  }
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
