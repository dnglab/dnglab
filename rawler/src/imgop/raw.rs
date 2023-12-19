// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::sensor::bayer::ppg::demosaic_ppg;
use super::xyz::Illuminant;
use super::{Dim2, Point, Result};
use crate::imgop::matrix::{multiply, normalize, pseudo_inverse};
use crate::imgop::xyz::SRGB_TO_XYZ_D65;
use crate::imgop::Rect;
use crate::pixarray::{Pix2D, RgbF32};
use crate::rawimage::{BlackLevel, RawPhotometricInterpretation, WhiteLevel};
use crate::{RawImageData, CFA};
use std::iter;

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
#[multiversion]
#[clone(target = "[x86|x86_64]+avx+avx2")]
#[clone(target = "x86+sse")]
fn correct_blacklevel_channels<const CH: usize>(raw: &mut [f32], blacklevel: &[f32; CH], whitelevel: &[f32; CH]) {
  // max value can be pre-computed for all channels.
  let mut max = *whitelevel;
  max.iter_mut().enumerate().for_each(|(i, x)| *x -= blacklevel[i]);

  let clip = |v: f32| {
    if v.is_sign_negative() {
      0.0
    } else {
      v
    }
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
#[multiversion]
#[clone(target = "[x86|x86_64]+avx+avx2")]
#[clone(target = "x86+sse")]
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
        if v.is_sign_negative() {
          0.0
        } else {
          v
        }
      };

      let ch = blacklevel.len();
      raw.chunks_exact_mut(ch).for_each(|block| {
        for i in 0..ch {
          block[i] = clip(block[i] - blacklevel[i]) / max[i];
        }
      });
    }
    _ => panic!("Blacklevel ({}) and Whitelevel ({})count mismatch", blacklevel.len(), whitelevel.len()),
  }
}

/// Correct data by blacklevel and whitelevel on CFA (bayer) data.
///
/// The output is between 0.0 .. 1.0.
///
/// This version is optimized vor vectorization, so please check
/// modifications on godbolt before committing.
#[multiversion]
#[clone(target = "[x86|x86_64]+avx+avx2")]
#[clone(target = "x86+sse")]
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
    if v.is_sign_negative() {
      0.0
    } else {
      v
    }
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

/// Rescale raw pixels by removing black level and scale to white level
/// Clip negative values to zero.
/// TODO: remove this old code, only used by superbayer
pub fn rescale<const N: usize>(pix: &[u16; N], black_level: &[f32; N], white_level: &[f32; N]) -> [f32; N] {
  let mut out = [f32::default(); N];
  for i in 0..N {
    out[i] = (pix[i] as f32 - black_level[i]) / (white_level[i] - black_level[i]);
  }
  clip_negative(&out)
}

#[multiversion]
#[clone(target = "[x86|x86_64]+avx+avx2")]
#[clone(target = "x86+sse")]
fn rgb_to_srgb_with_wb(rgb: &mut RgbF32, wb_coeff: &[f32; 4], xyz2cam: [[f32; 3]; 4]) {
  let rgb2cam = normalize(multiply(&xyz2cam, &SRGB_TO_XYZ_D65));
  let cam2rgb = pseudo_inverse(rgb2cam);

  rgb.for_each(|pix| {
    // We apply wb coeffs on the fly
    let r = pix[0] * wb_coeff[0];
    let g = pix[1] * wb_coeff[1];
    let b = pix[2] * wb_coeff[2];
    let srgb = [
      cam2rgb[0][0] * r + cam2rgb[0][1] * g + cam2rgb[0][2] * b,
      cam2rgb[1][0] * r + cam2rgb[1][1] * g + cam2rgb[1][2] * b,
      cam2rgb[2][0] * r + cam2rgb[2][1] * g + cam2rgb[2][2] * b,
    ];
    let mut clippd = clip_euclidean_norm_avg(&srgb);
    clippd.iter_mut().for_each(|p| *p = super::srgb::srgb_apply_gamma(*p));
    clippd
  });
}

fn develop_raw_cfa_srgb(pixels: &RawImageData, params: &DevelopParams, cfa: &CFA) -> Result<(Vec<f32>, Dim2)> {
  let black_level: [f32; 4] = match params.blacklevel.levels.len() {
    1 => Ok(collect_array(iter::repeat(params.blacklevel.levels[0].as_f32()))),
    4 => Ok(collect_array(params.blacklevel.levels.iter().map(|p| p.as_f32()))),
    c => Err(format!("Black level sample count of {} is invalid", c)),
  }?;
  let white_level: [f32; 4] = match params.whitelevel.0.len() {
    1 => Ok(collect_array(iter::repeat(params.whitelevel.0[0] as f32))),
    4 => Ok(collect_array(params.whitelevel.0.iter().map(|p| *p as f32))),
    c => Err(format!("White level sample count of {} is invalid", c)),
  }?;
  let wb_coeff: [f32; 4] = params.wb_coeff;

  log::debug!("Develop raw, wb: {:?}, black: {:?}, white: {:?}", wb_coeff, black_level, white_level);

  //Color Space Conversion
  let xyz2cam = params
    .color_matrices
    .iter()
    .find(|m| m.illuminant == Illuminant::D65)
    .ok_or("Illuminant matrix D65 not found")?
    .matrix;

  let raw_size = Rect::new_with_points(Point::zero(), Point::new(params.width, params.height));
  let active_area = params.active_area.unwrap_or(raw_size);
  let crop_area = params.crop_area.unwrap_or(active_area).adapt(&active_area);
  let mut pixels = pixels.as_f32();

  correct_blacklevel_cfa(pixels.to_mut(), params.width, params.height, &black_level, &white_level);
  let rgb = demosaic_ppg(&pixels, Dim2::new(params.width, params.height), cfa.clone(), active_area);

  let mut cropped_pixels = if raw_size.d != crop_area.d { rgb.crop(crop_area) } else { rgb };

  // Convert to sRGB from XYZ
  rgb_to_srgb_with_wb(&mut cropped_pixels, &wb_coeff, xyz2cam);

  // Flatten into Vec<f32>
  let srgb: Vec<f32> = cropped_pixels.into_inner().into_iter().flatten().collect();
  assert_eq!(srgb.len(), crop_area.d.w * crop_area.d.h * 3);
  Ok((srgb, crop_area.d))
}

fn develop_linearraw_srgb(pixels: &RawImageData, params: &DevelopParams) -> Result<(Vec<f32>, Dim2)> {
  let raw_size = Rect::new_with_points(Point::zero(), Point::new(params.width, params.height));
  let active_area = params.active_area.unwrap_or(raw_size);
  let crop_area = params.crop_area.unwrap_or(active_area).adapt(&active_area);
  let mut pixels = pixels.as_f32();
  correct_blacklevel(pixels.to_mut(), &params.blacklevel.as_vec(), &params.whitelevel.as_vec());

  match params.cpp {
    1 => {
      let clipped: Vec<f32> = pixels.iter().map(|x| clip_value(*x, 0.0, 1.0)).map(super::srgb::srgb_apply_gamma).collect();
      let rgb = Pix2D::new_with(clipped, params.width, params.height);
      let cropped_pixels = if raw_size.d != crop_area.d { rgb.crop(crop_area) } else { rgb };
      let srgb = cropped_pixels.into_inner();
      Ok((srgb, crop_area.d))
    }
    3 => {
      assert_eq!(params.blacklevel.levels.len(), params.cpp);
      let wb_coeff: [f32; 4] = params.wb_coeff;
      log::debug!("Develop raw, wb: {:?}", wb_coeff);
      let x = pixels.chunks_exact(3).map(|x| [x[0], x[1], x[2]]).collect();
      let rgb: RgbF32 = RgbF32::new_with(x, params.width, params.height);

      let mut cropped_pixels = if raw_size.d != crop_area.d { rgb.crop(crop_area) } else { rgb };

      //Color Space Conversion
      let xyz2cam = params
        .color_matrices
        .iter()
        .find(|m| m.illuminant == Illuminant::D65)
        .ok_or("Illuminant matrix D65 not found")?
        .matrix;

      // Convert to sRGB from XYZ
      rgb_to_srgb_with_wb(&mut cropped_pixels, &wb_coeff, xyz2cam);

      // Flatten into Vec<f32>
      let srgb: Vec<f32> = cropped_pixels.into_inner().into_iter().flatten().collect();

      assert_eq!(srgb.len(), crop_area.d.w * crop_area.d.h * 3);

      Ok((srgb, crop_area.d))
    }
    _ => todo!(),
  }
}

/// Develop a RAW image to sRGB
pub fn develop_raw_srgb(pixels: &RawImageData, params: &DevelopParams) -> Result<(Vec<f32>, Dim2)> {
  match &params.photometric {
    RawPhotometricInterpretation::BlackIsZero => todo!(),
    RawPhotometricInterpretation::Cfa(cfa) => develop_raw_cfa_srgb(pixels, params, cfa),
    RawPhotometricInterpretation::LinearRaw => develop_linearraw_srgb(pixels, params),
  }
}

/// Collect iterator into array
fn collect_array<T, I, const N: usize>(itr: I) -> [T; N]
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
