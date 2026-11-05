// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Estimation of the white balance

use rayon::prelude::*;

use crate::{
  imgop::{Dim2, Point, Rect, raw::cfa_planes},
  rawimage::{RawImage, RawImageData, RawPhotometricInterpretation},
};

/// Size of the blocks of [`auto_white_balance`] in pixels, a multiple of the
/// size of all CFA patterns
const AUTO_WB_BLOCK: usize = 12;

/// Linear value from which a sample is clipped for [`auto_white_balance`]
const AUTO_WB_CLIPPED: f32 = 0.98;

/// Smallest mean of a color plane for [`auto_white_balance`], darker images
/// don't tell their colors
const AUTO_WB_MIN_MEAN: f64 = 1.0 / 4096.0;

/// White balance coefficients of `planes` color planes, estimated from the
/// image data as read from the file
///
/// The DefaultCrop is assumed to be neutral on average: the coefficients are
/// the reciprocal of the mean of the linear values of each color plane. Blocks
/// with clipped samples are ignored, the clipping would change their colors.
/// `None` for images without color planes or usable blocks.
pub fn auto_white_balance(rawimage: &RawImage, planes: usize) -> Option<[f32; 4]> {
  let (width, height, cpp) = (rawimage.width, rawimage.height, rawimage.cpp);
  let samples = match &rawimage.data {
    RawImageData::Integer(samples) => samples.len(),
    RawImageData::Float(samples) => samples.len(),
  };
  if !(3..=4).contains(&planes) || samples != width * height * cpp {
    return None;
  }
  let cfa = match &rawimage.photometric {
    RawPhotometricInterpretation::Cfa(config) if cpp == 1 => Some((&config.cfa, cfa_planes(&config.cfa, &config.colors).ok()?)),
    _ if cpp == planes => None,
    _ => return None,
  };
  let full = Rect::new(Point::zero(), Dim2::new(width, height));
  let active_area = rawimage.active_area.unwrap_or(full);
  let area = rawimage.crop_area.or(rawimage.active_area).unwrap_or(full).intersect(&full)?;
  let table = rawimage.linearization_table.as_deref().filter(|table| !table.is_empty());
  let value = |index: usize| match &rawimage.data {
    RawImageData::Integer(samples) => {
      let sample = samples[index] as usize;
      table.map_or(sample as f32, |table| table[sample.min(table.len() - 1)] as f32)
    }
    RawImageData::Float(samples) => samples[index],
  };
  let (bottom, right) = (area.p.y + area.d.h, area.p.x + area.d.w);
  let block_rows: Vec<usize> = (area.p.y..bottom).step_by(AUTO_WB_BLOCK).collect();
  // Summed row by row in a fixed order, so the result doesn't depend on the threads
  let rows: Vec<([f64; 4], [u64; 4])> = block_rows
    .par_iter()
    .map(|&top| {
      let (mut sums, mut counts) = ([0.0; 4], [0; 4]);
      'blocks: for left in (area.p.x..right).step_by(AUTO_WB_BLOCK) {
        let (mut block_sums, mut block_counts) = ([0.0; 4], [0; 4]);
        for row in top..(top + AUTO_WB_BLOCK).min(bottom) {
          for col in left..(left + AUTO_WB_BLOCK).min(right) {
            for sample in 0..cpp {
              let plane = cfa
                .as_ref()
                .map_or(sample, |(cfa, planes)| planes[(row % cfa.height) * cfa.width + col % cfa.width]);
              let black = rawimage.blacklevel.level(row, col, sample, active_area);
              let range = rawimage.whitelevel.0[sample] as f32 - black;
              let linear = (value((row * width + col) * cpp + sample) - black) / range;
              if linear >= AUTO_WB_CLIPPED {
                continue 'blocks;
              }
              block_sums[plane] += linear.max(0.0) as f64;
              block_counts[plane] += 1;
            }
          }
        }
        for plane in 0..planes {
          sums[plane] += block_sums[plane];
          counts[plane] += block_counts[plane];
        }
      }
      (sums, counts)
    })
    .collect();
  let mut coeffs = [f32::NAN; 4];
  for (plane, coeff) in coeffs.iter_mut().enumerate().take(planes) {
    let sum: f64 = rows.iter().map(|(sums, _)| sums[plane]).sum();
    let count: u64 = rows.iter().map(|(_, counts)| counts[plane]).sum();
    let mean = sum / count.max(1) as f64;
    if mean < AUTO_WB_MIN_MEAN {
      return None;
    }
    *coeff = (1.0 / mean) as f32;
  }
  log::debug!("Auto white balance: {:?}", coeffs);
  Some(coeffs)
}
