// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::xyz::Illuminant;
use crate::bits::{LEu32, LookupTable};
use crate::cfa::{CFA, PlaneColor};
use crate::imgop::Rect;
use crate::pixarray::{Color2D, ImageViewMut};
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
pub(crate) fn cfa_planes(cfa: &CFA, colors: &PlaneColor) -> crate::Result<Vec<usize>> {
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
