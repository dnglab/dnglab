// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::xyz::Illuminant;
use crate::bits::{LEu32, LookupTable};
use crate::cfa::{CFA, PlaneColor};
use crate::imgop::Rect;
use crate::pixarray::{Color2D, ImageView, ImageViewMut, PixF32};
use crate::rawimage::{BlackLevel, RawPhotometricInterpretation, WhiteLevel};

use multiversion::multiversion;
use rayon::prelude::*;
use zerocopy::IntoBytes;

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

/// White balance coefficients of the color planes and the limit of the
/// balanced values
#[derive(Clone, Debug, PartialEq)]
pub struct WhiteBalance {
  /// Coefficient of each color plane
  pub coeffs: [f32; 4],
  /// Limit of the white balanced values, `None` for no limit
  pub clip: Option<f32>,
}

impl WhiteBalance {
  /// White balanced value `v` of color plane `plane`
  #[inline]
  pub fn apply(&self, v: f32, plane: usize) -> f32 {
    let v = v * self.coeffs[plane];
    match self.clip {
      Some(clip) => v.min(clip),
      None => v,
    }
  }
}

/// Color planes of the positions in the repeating pattern of `cfa`, row by row
///
/// `colors` maps the colors of the CFA to color planes. Fails for a CFA color
/// without plane.
fn cfa_planes(cfa: &CFA, colors: &PlaneColor) -> crate::Result<Vec<usize>> {
  let lookup = colors.plane_lookup_table();
  (0..cfa.height * cfa.width)
    .map(|i| match lookup[cfa.color_at(i / cfa.width, i % cfa.width)] {
      255 => Err(format!("CFA {} has a color without plane in {:?}", cfa, colors).into()),
      plane => Ok(plane),
    })
    .collect()
}

fn mosaic_error(channels: usize) -> crate::RawlerError {
  format!("Mosaic data must have one channel, image has {}", channels).into()
}

/// White balance an image of color planes in place, channel i is multiplied
/// by the coefficient of plane i. Fails for more than four channels.
pub fn white_balance<'a>(image: impl Into<ImageViewMut<'a, f32>>, wb: &WhiteBalance) -> crate::Result<()> {
  let mut image = image.into();
  let channels = image.channels();
  if channels > 4 {
    return Err(format!("White balance supports up to four color planes, image has {}", channels).into());
  }
  image.as_mut_slice().par_chunks_mut(channels.max(1)).for_each(|pixel| {
    pixel.iter_mut().enumerate().for_each(|(plane, v)| *v = wb.apply(*v, plane));
  });
  Ok(())
}

/// White balance CFA mosaic data in place: each sample is multiplied by the
/// coefficient of the color plane of its position in `cfa`
///
/// `colors` maps the colors of the CFA to color planes. Fails if `image` has
/// more than one channel.
pub fn white_balance_mosaic<'a>(image: impl Into<ImageViewMut<'a, f32>>, cfa: &CFA, colors: &PlaneColor, wb: &WhiteBalance) -> crate::Result<()> {
  let mut image = image.into();
  if image.channels() != 1 {
    return Err(mosaic_error(image.channels()));
  }
  let planes = cfa_planes(cfa, colors)?;
  let width = image.width().max(1);
  image.as_mut_slice().par_chunks_mut(width).enumerate().for_each(|(row, samples)| {
    let row_planes = &planes[(row % cfa.height) * cfa.width..][..cfa.width];
    for (v, plane) in samples.iter_mut().zip(row_planes.iter().cycle()) {
      *v = wb.apply(*v, *plane);
    }
  });
  Ok(())
}

/// Map integer CFA mosaic data to linear reference values by the black and
/// white levels, and white balance it by `wb`, in one pass
///
/// The levels repeat in a 2x2 pattern (see [`BlackLevel::as_bayer_array`]),
/// values below black are 0.0. `colors` maps the colors of `cfa` to the color
/// planes of `wb`. Fails if `src` has more than one channel.
pub fn rescale_mosaic<'a>(
  src: impl Into<ImageView<'a, u16>>,
  blacklevel: &[f32; 4],
  whitelevel: &[f32; 4],
  cfa: &CFA,
  colors: &PlaneColor,
  wb: Option<&WhiteBalance>,
) -> crate::Result<PixF32> {
  let src = src.into();
  if src.channels() != 1 {
    return Err(mosaic_error(src.channels()));
  }
  let planes = cfa_planes(cfa, colors)?;
  let max: [f32; 4] = std::array::from_fn(|i| whitelevel[i] - blacklevel[i]);
  let clip = |v: f32| {
    if v.is_sign_negative() { 0.0 } else { v }
  };
  let width = src.width().max(1);
  let mut out = PixF32::new(src.width(), src.height());
  out
    .data
    .par_chunks_mut(width)
    .zip(src.as_slice().par_chunks(width))
    .enumerate()
    .for_each(|(row, (out, src))| {
      let level = (row % 2) * 2;
      let row_planes = &planes[(row % cfa.height) * cfa.width..][..cfa.width];
      for (col, ((out, v), plane)) in out.iter_mut().zip(src).zip(row_planes.iter().cycle()).enumerate() {
        let i = level + col % 2;
        let v = clip(f32::from(*v) - blacklevel[i]) / max[i];
        *out = match wb {
          Some(wb) => wb.apply(v, *plane),
          None => v,
        };
      }
    });
  Ok(out)
}

/// Apply a DNG LinearizationTable to integer samples, with dithering.
///
/// Values beyond the table are mapped to the last table entry.
/// Each row of `row_width` samples is dithered with its own seed.
pub fn apply_linearization_table(samples: &mut [u16], row_width: usize, table: &[u16]) {
  if table.is_empty() || row_width == 0 {
    return;
  }
  let table = LookupTable::new_with_bits(table, 16);
  samples.par_chunks_exact_mut(row_width).for_each(|row| {
    let mut random = if row.len() >= 2 { LEu32(row.as_bytes(), 0) } else { 0 };
    row.iter_mut().for_each(|p| {
      *p = table.dither(*p, &mut random);
    })
  });
}

/// Map four color planes to three planes by the 3x4 `reduction` matrix
#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
pub(crate) fn map_4ch_to_3ch(src: &Color2D<f32, 4>, reduction: &[[f32; 4]; 3]) -> Color2D<f32, 3> {
  let mut out = Vec::with_capacity(src.data.len());

  src
    .pixels()
    .par_iter()
    .map(|pix| reduction.map(|row| row[0] * pix[0] + row[1] * pix[1] + row[2] * pix[2] + row[3] * pix[3]))
    .collect_into_vec(&mut out);

  Color2D::new_with(out, src.width, src.height)
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

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn rescale_mosaic_like_correct_blacklevel_cfa() -> crate::Result<()> {
    let (cfa, colors) = (CFA::new("RGGB"), PlaneColor::new("RGB"));
    let samples: Vec<u16> = vec![100, 200, 300, 400, 50, 1100, 600, 700];
    let (blacklevel, whitelevel) = ([100.0, 100.0, 100.0, 60.0], [1100.0; 4]);
    let mut expected: Vec<f32> = samples.iter().copied().map(f32::from).collect();
    correct_blacklevel_cfa(&mut expected, 4, 2, &blacklevel, &whitelevel);
    let rescaled = rescale_mosaic(ImageView::new(&samples, 4, 2, 1), &blacklevel, &whitelevel, &cfa, &colors, None)?;
    assert_eq!(rescaled.data, expected);

    // White balancing in the same pass gives the same result as a separate pass
    let wb = WhiteBalance {
      coeffs: [2.0, 1.0, 1.5, f32::NAN],
      clip: Some(1.0),
    };
    let balanced = rescale_mosaic(ImageView::new(&samples, 4, 2, 1), &blacklevel, &whitelevel, &cfa, &colors, Some(&wb))?;
    white_balance_mosaic(ImageViewMut::new(&mut expected, 4, 2, 1), &cfa, &colors, &wb)?;
    assert_eq!(balanced.data, expected);
    // Red is doubled, blue of 1.5 is limited to 1.0
    let close = |a: f32, b: f32| (a - b).abs() < 1e-6;
    assert!(
      close(balanced.data[2], 0.4) && close(balanced.data[3], 0.3) && balanced.data[5] == 1.0,
      "{:?}",
      balanced.data
    );

    // Mosaic data has one channel
    assert!(rescale_mosaic(ImageView::new(&samples, 2, 2, 2), &blacklevel, &whitelevel, &cfa, &colors, None).is_err());
    Ok(())
  }

  #[test]
  fn white_balance_mosaic_by_plane_colors() -> crate::Result<()> {
    let wb = WhiteBalance {
      coeffs: [1.0, 2.0, 3.0, 4.0],
      clip: None,
    };
    // The coefficients are indexed by the color planes, not by the CFA colors
    for (colors, expected) in [("CYGM", [1.0, 2.0, 3.0, 4.0]), ("GMCY", [3.0, 4.0, 1.0, 2.0])] {
      let mut samples = vec![1.0_f32; 8];
      white_balance_mosaic(ImageViewMut::new(&mut samples, 4, 2, 1), &CFA::new("CYGM"), &PlaneColor::new(colors), &wb)?;
      assert_eq!(
        samples,
        [
          expected[0],
          expected[1],
          expected[0],
          expected[1],
          expected[2],
          expected[3],
          expected[2],
          expected[3]
        ],
        "{}",
        colors
      );
    }
    Ok(())
  }

  #[test]
  fn white_balance_of_color_planes() -> crate::Result<()> {
    let wb = WhiteBalance {
      coeffs: [2.0, 1.0, 1.5, f32::NAN],
      clip: Some(1.0),
    };
    let mut samples = vec![0.25, 0.5, 0.5, 0.75, 0.5, 0.25_f32];
    white_balance(ImageViewMut::new(&mut samples, 2, 1, 3), &wb)?;
    assert_eq!(samples, [0.5, 0.5, 0.75, 1.0, 0.5, 0.375]);
    assert!(white_balance(ImageViewMut::new(&mut [0.0; 5], 1, 1, 5), &wb).is_err());
    Ok(())
  }
}
