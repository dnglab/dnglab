// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Wavelet noise reduction of linear image data
//!
//! The white balanced values are stabilized by their square root, which makes
//! shot noise almost independent of the brightness, and transformed to one luma
//! and several chroma planes. Each plane is split into detail bands by an
//! à trous wavelet transform with a hat filter. The noise of a band is estimated
//! from its flattest blocks, because the noise of demosaiced data is correlated
//! and doesn't follow the fixed scale factors of white noise. Each coefficient
//! is shrunk by the energy of its neighborhood, which removes isolated noise
//! peaks but keeps edges.
//!
//! Removing variance from the stabilized values lowers the mean of the linear
//! values by the same amount, independent of their brightness: the shadows and
//! the color with the most noise would change the most. The mean of each block
//! of the image is restored at the end.

use std::time::{Duration, Instant};

use multiversion::multiversion;
use rayon::prelude::*;

use crate::pixarray::ImageRegionMut;

/// Maximum number of detail bands of a plane
const LEVELS: usize = 5;

/// Width and height of the blocks to estimate the noise from
const BLOCK_SIZE: usize = 32;

/// The noise of a band is the deviation of its blocks at this percentile
const BLOCK_PERCENTILE: usize = 25;

/// Blocks with a smaller deviation have no noise, e.g. in clipped highlights
const MIN_BLOCK_DEVIATION: f32 = 1e-6;

/// Added before the square root, to not amplify the read noise of dark values
pub(super) const STABILIZER_OFFSET: f32 = 1.0 / 4096.0;

/// Threshold of the luma plane for a strength of 1.0, in units of the noise
const LUMA_THRESHOLD: f32 = 1.5;

/// Threshold of the chroma planes for a strength of 1.0, in units of the noise
const CHROMA_THRESHOLD: f32 = 2.5;

/// Orthonormal map of three color planes to luma and chroma, the noise of the
/// planes stays comparable
const PLANES_3: [[f32; 4]; 4] = [
  [0.57735026, 0.57735026, 0.57735026, 0.0],
  [std::f32::consts::FRAC_1_SQRT_2, 0.0, -std::f32::consts::FRAC_1_SQRT_2, 0.0],
  [0.40824829, -0.81649658, 0.40824829, 0.0],
  [0.0, 0.0, 0.0, 0.0],
];

/// Orthonormal map of four color planes to luma and chroma
const PLANES_4: [[f32; 4]; 4] = [
  [0.5, 0.5, 0.5, 0.5],
  [std::f32::consts::FRAC_1_SQRT_2, -std::f32::consts::FRAC_1_SQRT_2, 0.0, 0.0],
  [0.40824829, 0.40824829, -0.81649658, 0.0],
  [0.28867513, 0.28867513, 0.28867513, -0.8660254],
];

/// Map of monochrome data, its only plane is luma
const PLANES_1: [[f32; 4]; 4] = [[1.0, 0.0, 0.0, 0.0], [0.0; 4], [0.0; 4], [0.0; 4]];

/// Strength of the noise reduction, the thresholds scale with the noise that is
/// measured in the image
///
/// 0.0 leaves the plane untouched and 1.0 is a full noise reduction. Larger
/// values remove more detail.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DenoiseStrength {
  /// Strength for the luma plane
  pub luma: f32,
  /// Strength for the chroma planes
  pub chroma: f32,
}

impl Default for DenoiseStrength {
  /// A little noise reduction: most of the chroma noise is removed, the luma
  /// grain and the fine detail are kept
  fn default() -> Self {
    Self { luma: 0.25, chroma: 0.5 }
  }
}

impl DenoiseStrength {
  /// Without any strength, the image data is not changed
  pub fn none() -> Self {
    Self { luma: 0.0, chroma: 0.0 }
  }

  pub fn is_active(self) -> bool {
    self.luma > 0.0 || self.chroma > 0.0
  }
}

/// Noise before and after the reduction: the deviation of the stabilized values
/// in the flattest blocks of the input
///
/// The same blocks are measured on both sides. After the reduction the flat
/// blocks can be empty, the flattest blocks of the output would be other ones.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct NoiseChange {
  pub(super) before: f32,
  pub(super) after: f32,
}

impl NoiseChange {
  /// Measure the noise in the flattest blocks of `before`, see [`block_deviations`]
  pub(super) fn new(before: &[f32], after: &[f32]) -> Self {
    let blocks = flat_blocks(before);
    Self {
      before: block_noise(before, &blocks),
      after: block_noise(after, &blocks),
    }
  }

  /// Share of the noise that is left, in percent
  pub(super) fn percent_left(self) -> f64 {
    if self.before > 0.0 {
      100.0 * f64::from(self.after) / f64::from(self.before)
    } else {
      100.0
    }
  }
}

impl std::fmt::Display for NoiseChange {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "noise {:.5} -> {:.5} ({:.1}% left)", self.before, self.after, self.percent_left())
  }
}

/// Statistics of a detail band
#[derive(Clone, Copy, Debug, PartialEq)]
struct LevelStats {
  /// Estimated noise, in the stabilized values
  sigma: f32,
  /// Threshold of the neighborhood energy of the coefficients
  threshold: f32,
  /// Number of coefficients that are removed, only if statistics are collected
  removed: usize,
  /// Noise of the band, only if statistics are collected
  noise: NoiseChange,
}

/// Statistics of a luma or chroma plane that is denoised
#[derive(Clone, Debug, PartialEq)]
struct PlaneStats {
  plane: usize,
  levels: Vec<LevelStats>,
  /// Sum of the squared changes of the stabilized values, only if statistics
  /// are collected
  squared_error: f64,
  /// Noise of all detail bands, only if statistics are collected
  noise: NoiseChange,
}

/// Statistics of a color plane of the image
#[derive(Clone, Copy, Debug, PartialEq)]
struct ColorStats {
  color: usize,
  /// Mean of the linear values before the reduction
  mean_before: f32,
  /// Mean of the linear values after the reduction
  mean_after: f32,
  /// Mean offset that was added to restore the mean of the blocks
  restored: f32,
  /// Noise of all detail bands
  noise: NoiseChange,
}

/// Statistics of a noise reduction. Only the estimated noise and the threshold
/// of the detail bands are known without collecting them.
#[derive(Clone, Debug, Default, PartialEq)]
struct DenoiseStats {
  planes: Vec<PlaneStats>,
  colors: Vec<ColorStats>,
}

/// PSNR of a mean squared error of stabilized values, with a peak of 1.0
fn psnr(mean_squared_error: f64) -> f64 {
  if mean_squared_error > 0.0 {
    -10.0 * mean_squared_error.log10()
  } else {
    f64::INFINITY
  }
}

/// Buffers of the size of a plane, they are reused for all planes
struct Buffers {
  /// Plane to split, then the low-pass result of the last level
  current: Vec<f32>,
  lowpass: Vec<f32>,
  scratch: Vec<f32>,
  /// Sum of the shrunk detail bands
  result: Vec<f32>,
  /// Shrunk detail band, only used to collect statistics
  shrunk: Vec<f32>,
}

impl Buffers {
  fn new(len: usize, collect: bool) -> Self {
    Self {
      current: vec![0.0; len],
      lowpass: vec![0.0; len],
      scratch: vec![0.0; len],
      result: vec![0.0; len],
      shrunk: if collect { vec![0.0; len] } else { Vec::new() },
    }
  }
}

/// Reduce the noise of the linear values in `region`, which has 1, 3 or 4 color
/// planes
///
/// `wb` are the white balance coefficients to reach neutral colors, all ones if
/// the values are already white balanced. Only the first coefficients for the
/// planes of `region` are used. The values stay in their white balance and the
/// mean of each block of the image is kept. Negative values are set to zero. The
/// statistics of the transform are logged at the debug level.
pub fn wavelet_denoise(mut region: ImageRegionMut<'_, f32>, strength: DenoiseStrength, wb: &[f32; 4]) -> crate::Result<()> {
  let start = Instant::now();
  let collect = log::log_enabled!(log::Level::Debug);
  let stats = denoise_region(region.reborrow(), strength, wb, collect)?;
  if collect {
    log_statistics(&region, strength, &stats, start.elapsed());
  }
  Ok(())
}

/// Denoise `region`, see [`wavelet_denoise`]. The statistics that are not needed
/// for the result are only calculated with `collect`, they have no effect on it.
fn denoise_region(mut region: ImageRegionMut<'_, f32>, strength: DenoiseStrength, wb: &[f32; 4], collect: bool) -> crate::Result<DenoiseStats> {
  let (width, height, channels) = (region.width(), region.height(), region.channels());
  let map = match channels {
    1 => &PLANES_1,
    3 => &PLANES_3,
    4 => &PLANES_4,
    _ => return Err(input_error!("Noise reduction of {} color planes is not supported", channels)),
  };
  if !strength.is_active() || width == 0 || height == 0 {
    return Ok(DenoiseStats::default());
  }
  let wb = wb.map(|c| if c.is_finite() && c > 0.0 { c } else { 1.0 });

  let means = block_means(&region);
  let mut buffers = Buffers::new(width * height, collect);
  let colors_before: Vec<(f32, Vec<f32>)> = if collect {
    (0..channels).map(|color| measure_color(&region, color, wb[color], &mut buffers)).collect()
  } else {
    Vec::new()
  };

  region
    .par_rows_mut()
    .for_each(|row| row.chunks_exact_mut(channels).for_each(|pixel| to_planes(pixel, map, &wb)));

  let mut planes = Vec::with_capacity(channels);
  for plane in 0..channels {
    let factor = if plane == 0 {
      strength.luma * LUMA_THRESHOLD
    } else {
      strength.chroma * CHROMA_THRESHOLD
    };
    // Also skips a strength that is not a number
    let active = factor > 0.0;
    if !active {
      continue;
    }
    let deviations_before = collect.then(|| {
      read_samples(&region, plane, &mut buffers.current);
      high_pass_deviations(&mut buffers, width, height)
    });
    read_samples(&region, plane, &mut buffers.current);
    let levels = denoise_plane(&mut buffers, width, height, factor, collect);
    let squared_error = region
      .par_rows_mut()
      .zip(buffers.result.par_chunks_exact(width))
      .map(|(row, values)| {
        let mut error = 0.0_f64;
        for (sample, value) in row.iter_mut().skip(plane).step_by(channels).zip(values) {
          if collect {
            error += f64::from(*sample - *value).powi(2);
          }
          *sample = *value;
        }
        error
      })
      .sum();
    let noise = deviations_before.map_or_else(NoiseChange::default, |before| {
      read_samples(&region, plane, &mut buffers.current);
      NoiseChange::new(&before, &high_pass_deviations(&mut buffers, width, height))
    });
    planes.push(PlaneStats {
      plane,
      levels,
      squared_error,
      noise,
    });
  }

  region
    .par_rows_mut()
    .for_each(|row| row.chunks_exact_mut(channels).for_each(|pixel| from_planes(pixel, map, &wb)));
  let restored = restore_block_means(&mut region, &means);

  let colors = colors_before
    .into_iter()
    .enumerate()
    .map(|(color, (mean_before, before))| {
      let (mean_after, after) = measure_color(&region, color, wb[color], &mut buffers);
      ColorStats {
        color,
        mean_before,
        mean_after,
        restored: restored[color],
        noise: NoiseChange::new(&before, &after),
      }
    })
    .collect();
  Ok(DenoiseStats { planes, colors })
}

/// Size of the blocks of an image and the number of full blocks per row and column
fn block_grid(width: usize, height: usize) -> (usize, usize, usize, usize) {
  let (block_width, block_height) = (BLOCK_SIZE.min(width), BLOCK_SIZE.min(height));
  (block_width, block_height, width / block_width, height / block_height)
}

/// Mean of the samples of each block of `region`, for all color planes of a block
fn block_means(region: &ImageRegionMut<'_, f32>) -> Vec<f32> {
  let channels = region.channels();
  let (block_width, block_height, columns, rows) = block_grid(region.width(), region.height());
  let samples = (block_width * block_height) as f64;
  let region = region.as_region();
  (0..rows)
    .into_par_iter()
    .flat_map_iter(|block_row| {
      let mut sums = vec![0.0_f64; columns * channels];
      for y in block_row * block_height..(block_row + 1) * block_height {
        let line = &region.row(y)[..columns * block_width * channels];
        for (block, sums) in line.chunks_exact(block_width * channels).zip(sums.chunks_exact_mut(channels)) {
          for pixel in block.chunks_exact(channels) {
            sums.iter_mut().zip(pixel).for_each(|(sum, value)| *sum += f64::from(*value));
          }
        }
      }
      sums.into_iter().map(move |sum| (sum / samples) as f32)
    })
    .collect()
}

/// Neighbor blocks of position `index` and the weight of the second one, for
/// the interpolation between the centers of `count` blocks of size `block`
fn block_position(index: usize, block: usize, count: usize) -> (usize, usize, f32) {
  let position = ((index as f32 + 0.5) / block as f32 - 0.5).clamp(0.0, (count - 1) as f32);
  let first = position as usize;
  (first, (first + 1).min(count - 1), position - first as f32)
}

/// Add the change of the block means to the samples of `region`, so the blocks
/// get back the means `before`, see [`block_means`]. The offsets are interpolated
/// between the centers of the blocks and negative values are set to zero. Returns
/// the mean offset of each color plane.
///
/// A strong reduction blurs strong edges, which moves brightness across the edge
/// without any bias. It is part of the offset of the blocks at the edge, so at the
/// highest strengths, dark areas next to a strong edge can get slightly too dark.
/// A median of the offsets would avoid this, but it loses the true offset of
/// textured blocks, which is a larger error in real images.
fn restore_block_means(region: &mut ImageRegionMut<'_, f32>, before: &[f32]) -> Vec<f32> {
  let (width, height, channels) = (region.width(), region.height(), region.channels());
  let (block_width, block_height, columns, rows) = block_grid(width, height);
  let offsets: Vec<f32> = before.iter().zip(block_means(region)).map(|(before, after)| before - after).collect();
  let x_positions: Vec<(usize, usize, f32)> = (0..width).map(|x| block_position(x, block_width, columns)).collect();
  let offsets_ref = &offsets;
  let x_positions_ref = &x_positions;
  region.par_rows_mut().enumerate().for_each(|(y, row)| {
    let (first, second, weight) = block_position(y, block_height, rows);
    let line = |block_row: usize| &offsets_ref[block_row * columns * channels..(block_row + 1) * columns * channels];
    let row_offsets: Vec<f32> = line(first).iter().zip(line(second)).map(|(a, b)| a * (1.0 - weight) + b * weight).collect();
    for (pixel, (left, right, weight)) in row.chunks_exact_mut(channels).zip(x_positions_ref) {
      for (color, sample) in pixel.iter_mut().enumerate() {
        let offset = row_offsets[left * channels + color] * (1.0 - weight) + row_offsets[right * channels + color] * weight;
        *sample = (*sample + offset).max(0.0);
      }
    }
  });
  (0..channels)
    .map(|color| {
      let sum: f64 = offsets.iter().skip(color).step_by(channels).map(|offset| f64::from(*offset)).sum();
      (sum / (columns * rows) as f64) as f32
    })
    .collect()
}

/// Copy the samples of `plane` of the pixels of `region`
fn read_samples(region: &ImageRegionMut<'_, f32>, plane: usize, out: &mut [f32]) {
  let channels = region.channels();
  out
    .par_chunks_exact_mut(region.width())
    .zip(region.as_region().par_rows())
    .for_each(|(out, row)| out.iter_mut().zip(row.iter().skip(plane).step_by(channels)).for_each(|(o, v)| *o = *v));
}

/// Mean of the linear values of a color plane and the deviations of the blocks
/// of its stabilized values, see [`high_pass_deviations`]
fn measure_color(region: &ImageRegionMut<'_, f32>, color: usize, coeff: f32, buffers: &mut Buffers) -> (f32, Vec<f32>) {
  read_samples(region, color, &mut buffers.current);
  let sum: f64 = buffers.current.par_iter().map(|v| f64::from(*v)).sum();
  let mean = (sum / buffers.current.len() as f64) as f32;
  buffers
    .current
    .par_iter_mut()
    .for_each(|v| *v = ((*v * coeff).max(0.0) + STABILIZER_OFFSET).sqrt());
  (mean, high_pass_deviations(buffers, region.width(), region.height()))
}

/// Deviations of the blocks of the values in `current` without their low-pass
/// result of all levels, which is the noise of all detail bands. This changes
/// the buffers.
fn high_pass_deviations(buffers: &mut Buffers, width: usize, height: usize) -> Vec<f32> {
  let Buffers {
    current,
    lowpass,
    scratch,
    result,
    ..
  } = buffers;
  result.copy_from_slice(current);
  for level in 0..level_count(width, height) {
    smooth(current, lowpass, scratch, width, height, 1 << level);
    std::mem::swap(current, lowpass);
  }
  result.par_iter_mut().zip(current.par_iter()).for_each(|(value, low)| *value -= *low);
  block_deviations(result, width, height)
}

/// Deviations of the blocks of the detail of `values`, see [`high_pass_deviations`]
pub(super) fn detail_deviations(values: &[f32], width: usize, height: usize) -> Vec<f32> {
  let mut buffers = Buffers::new(values.len(), false);
  buffers.current.copy_from_slice(values);
  high_pass_deviations(&mut buffers, width, height)
}

/// Stabilized luma and chroma values of a pixel of linear values
fn to_planes(pixel: &mut [f32], map: &[[f32; 4]; 4], wb: &[f32; 4]) {
  let mut stabilized = [0.0_f32; 4];
  for ((value, sample), coeff) in stabilized.iter_mut().zip(pixel.iter()).zip(wb) {
    *value = ((sample * coeff).max(0.0) + STABILIZER_OFFSET).sqrt();
  }
  for (sample, row) in pixel.iter_mut().zip(map) {
    *sample = row.iter().zip(&stabilized).map(|(a, b)| a * b).sum();
  }
}

/// Linear values of a pixel of stabilized luma and chroma values
fn from_planes(pixel: &mut [f32], map: &[[f32; 4]; 4], wb: &[f32; 4]) {
  let mut stabilized = [0.0_f32; 4];
  for (color, value) in stabilized.iter_mut().enumerate().take(pixel.len()) {
    *value = pixel.iter().zip(map).map(|(sample, row)| sample * row[color]).sum();
  }
  for ((sample, value), coeff) in pixel.iter_mut().zip(&stabilized).zip(wb) {
    *sample = (value.max(0.0).powi(2) - STABILIZER_OFFSET).max(0.0) / coeff;
  }
}

/// Number of levels of a plane, the distance of the filter must stay below its size
fn level_count(width: usize, height: usize) -> usize {
  let size = width.min(height);
  (0..LEVELS).take_while(|level| (1_usize << level) < size).count()
}

/// Denoise the plane in `current` to `result`, with a threshold of `factor` times
/// the noise of each band
fn denoise_plane(buffers: &mut Buffers, width: usize, height: usize, factor: f32, collect: bool) -> Vec<LevelStats> {
  let Buffers {
    current,
    lowpass,
    scratch,
    result,
    shrunk,
  } = buffers;
  result.fill(0.0);
  let levels = (0..level_count(width, height))
    .map(|level| {
      smooth(current, lowpass, scratch, width, height, 1 << level);
      current.par_iter_mut().zip(lowpass.par_iter()).for_each(|(detail, low)| *detail -= *low);
      let deviations = block_deviations(current, width, height);
      let sigma = noise_of_blocks(&deviations);
      let threshold = factor * sigma;
      let (removed, noise) = if collect {
        shrunk.fill(0.0);
        let removed = shrink_band(current, scratch, shrunk, width, height, threshold * threshold, true);
        result.par_iter_mut().zip(shrunk.par_iter()).for_each(|(sum, detail)| *sum += *detail);
        (removed, NoiseChange::new(&deviations, &block_deviations(shrunk, width, height)))
      } else {
        let removed = shrink_band(current, scratch, result, width, height, threshold * threshold, false);
        (removed, NoiseChange::default())
      };
      std::mem::swap(current, lowpass);
      LevelStats {
        sigma,
        threshold,
        removed,
        noise,
      }
    })
    .collect();
  result.par_iter_mut().zip(current.par_iter()).for_each(|(sum, low)| *sum += *low);
  levels
}

/// Position `index` of a row or column of `len` values, mirrored at the borders
fn mirror(index: isize, len: usize) -> usize {
  let index = index.unsigned_abs();
  if index < len { index } else { 2 * len - 2 - index }
}

/// Filter `src` by the hat filter with a distance of `step` samples, see [`hat_row`]
fn smooth(src: &[f32], dst: &mut [f32], scratch: &mut [f32], width: usize, height: usize, step: usize) {
  scratch
    .par_chunks_exact_mut(width)
    .zip(src.par_chunks_exact(width))
    .for_each(|(out, row)| hat_row(row, out, step));
  let scratch = &*scratch;
  dst.par_chunks_exact_mut(width).enumerate().for_each(|(y, out)| {
    let row = |y: usize| &scratch[y * width..(y + 1) * width];
    let above = row(mirror(y as isize - step as isize, height));
    let below = row(mirror((y + step) as isize, height));
    hat_column(row(y), above, below, out);
  });
}

/// Hat filter [1 2 1] / 4 of a row, with `step` samples between the taps
#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn hat_row(row: &[f32], out: &mut [f32], step: usize) {
  let len = row.len();
  // Positions with both neighbors inside of the row
  let inner = len.saturating_sub(2 * step);
  for i in (0..step).chain(step + inner..len) {
    let (left, right) = (mirror(i as isize - step as isize, len), mirror((i + step) as isize, len));
    out[i] = (2.0 * row[i] + row[left] + row[right]) * 0.25;
  }
  if inner > 0 {
    let center = &row[step..step + inner];
    for (((o, c), l), r) in out[step..step + inner].iter_mut().zip(center).zip(&row[..inner]).zip(&row[2 * step..]) {
      *o = (2.0 * c + l + r) * 0.25;
    }
  }
}

/// Hat filter [1 2 1] / 4 of the rows above and below a row
#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn hat_column(row: &[f32], above: &[f32], below: &[f32], out: &mut [f32]) {
  for (((o, c), a), b) in out.iter_mut().zip(row).zip(above).zip(below) {
    *o = (2.0 * c + a + b) * 0.25;
  }
}

/// Standard deviation of the values of each block of a band
fn block_deviations(band: &[f32], width: usize, height: usize) -> Vec<f32> {
  let (block_width, block_height, columns, rows) = block_grid(width, height);
  let samples = (block_width * block_height) as f32;
  (0..rows)
    .into_par_iter()
    .flat_map_iter(|block_row| {
      let mut sums = vec![(0.0_f32, 0.0_f32); columns];
      for y in block_row * block_height..(block_row + 1) * block_height {
        let line = &band[y * width..y * width + columns * block_width];
        for (block, (sum, squares)) in line.chunks_exact(block_width).zip(&mut sums) {
          for value in block {
            *sum += value;
            *squares += value * value;
          }
        }
      }
      sums
        .into_iter()
        .map(move |(sum, squares)| ((squares - sum * sum / samples) / samples).max(0.0).sqrt())
    })
    .collect()
}

/// Noise of a band: the deviation of its blocks at [`BLOCK_PERCENTILE`], so it is
/// estimated from the flattest parts of the image. Blocks without any variation
/// are ignored.
fn noise_of_blocks(deviations: &[f32]) -> f32 {
  let mut deviations: Vec<f32> = deviations.iter().copied().filter(|deviation| *deviation > MIN_BLOCK_DEVIATION).collect();
  if deviations.is_empty() {
    return 0.0;
  }
  let index = deviations.len() * BLOCK_PERCENTILE / 100;
  let (_, deviation, _) = deviations.select_nth_unstable_by(index, f32::total_cmp);
  *deviation
}

/// Positions of the blocks at or below the deviation of [`noise_of_blocks`]
fn flat_blocks(deviations: &[f32]) -> Vec<usize> {
  let limit = noise_of_blocks(deviations);
  (0..deviations.len())
    .filter(|block| deviations[*block] > MIN_BLOCK_DEVIATION && deviations[*block] <= limit)
    .collect()
}

/// Root mean square of the deviations of `blocks`
fn block_noise(deviations: &[f32], blocks: &[usize]) -> f32 {
  if blocks.is_empty() {
    return 0.0;
  }
  let sum: f64 = blocks.iter().map(|block| f64::from(deviations[*block]).powi(2)).sum();
  (sum / blocks.len() as f64).sqrt() as f32
}

/// Add the coefficients of `band` to `result`, shrunk by the energy of their
/// neighborhood of 3x3 pixels. Returns the number of removed coefficients, if
/// they are counted with `count`.
fn shrink_band(band: &[f32], scratch: &mut [f32], result: &mut [f32], width: usize, height: usize, threshold: f32, count: bool) -> usize {
  scratch
    .par_chunks_exact_mut(width)
    .zip(band.par_chunks_exact(width))
    .for_each(|(out, row)| row_energy(row, out));
  let scratch = &*scratch;
  result
    .par_chunks_exact_mut(width)
    .zip(band.par_chunks_exact(width))
    .enumerate()
    .map(|(y, (out, detail))| {
      let row = |y: usize| &scratch[y * width..(y + 1) * width];
      let above = row(mirror(y as isize - 1, height));
      let below = row(mirror(y as isize + 1, height));
      shrink_row(detail, row(y), above, below, out, threshold);
      if count { count_removed(row(y), above, below, threshold) } else { 0 }
    })
    .sum()
}

/// Mean of the squares of each sample and its two neighbors, `row` has at least two samples
#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn row_energy(row: &[f32], out: &mut [f32]) {
  let len = row.len();
  out[0] = (row[0] * row[0] + 2.0 * row[1] * row[1]) / 3.0;
  out[len - 1] = (row[len - 1] * row[len - 1] + 2.0 * row[len - 2] * row[len - 2]) / 3.0;
  for (o, samples) in out[1..len - 1].iter_mut().zip(row.windows(3)) {
    *o = (samples[0] * samples[0] + samples[1] * samples[1] + samples[2] * samples[2]) / 3.0;
  }
}

/// Add the coefficients of a row, scaled by max(0, 1 - threshold / energy), to `out`
#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn shrink_row(detail: &[f32], row: &[f32], above: &[f32], below: &[f32], out: &mut [f32], threshold: f32) {
  for ((((o, d), r), a), b) in out.iter_mut().zip(detail).zip(row).zip(above).zip(below) {
    let energy = (r + a + b) / 3.0;
    let gain = if energy > threshold { 1.0 - threshold / energy } else { 0.0 };
    *o += gain * d;
  }
}

/// Number of coefficients of a row that [`shrink_row`] removes
fn count_removed(row: &[f32], above: &[f32], below: &[f32], threshold: f32) -> usize {
  row.iter().zip(above).zip(below).filter(|((r, a), b)| (*r + *a + *b) / 3.0 <= threshold).count()
}

/// Name of color plane `color` of an image with `channels` color planes
fn color_name(color: usize, channels: usize) -> String {
  match channels {
    1 => "gray".to_string(),
    3 => ["R", "G", "B"][color].to_string(),
    _ => format!("color {}", color + 1),
  }
}

/// Log the statistics of a noise reduction of `region`
fn log_statistics(region: &ImageRegionMut<'_, f32>, strength: DenoiseStrength, stats: &DenoiseStats, elapsed: Duration) {
  let (pixels, channels) = (region.width() * region.height(), region.channels());
  log::debug!("Denoise: the noise is the deviation of the stabilized values in the flattest blocks of the input");
  for plane in &stats.planes {
    let name = if plane.plane == 0 {
      "luma".to_string()
    } else {
      format!("chroma {}", plane.plane)
    };
    for (level, level_stats) in plane.levels.iter().enumerate() {
      log::debug!(
        "Denoise {} level {}: {}, threshold {:.5}, {:.1}% of the coefficients removed",
        name,
        level + 1,
        level_stats.noise,
        level_stats.threshold,
        100.0 * level_stats.removed as f64 / pixels.max(1) as f64
      );
    }
    log::debug!(
      "Denoise {}: {}, PSNR {:.2} dB to the input",
      name,
      plane.noise,
      psnr(plane.squared_error / pixels.max(1) as f64)
    );
  }
  for color in &stats.colors {
    log::debug!(
      "Denoise {}: {}, mean of the linear values {:.5} -> {:.5}, offset {:+.5} restored",
      color_name(color.color, channels),
      color.noise,
      color.mean_before,
      color.mean_after,
      color.restored
    );
  }
  let average = average_noise(&stats.colors);
  log::debug!("Denoise average of the color planes: {}", average);
  log::debug!(
    "Denoise of {}x{} pixels, {} planes, strength luma {} chroma {}: PSNR {:.2} dB to the input (the amount of the change, not the quality), {:.3} s",
    region.width(),
    region.height(),
    channels,
    strength.luma,
    strength.chroma,
    psnr(overall_error(&stats.planes) / (pixels * channels).max(1) as f64),
    elapsed.as_secs_f64()
  );
}

/// Mean of the noise of the color planes
fn average_noise(colors: &[ColorStats]) -> NoiseChange {
  let count = colors.len().max(1) as f32;
  NoiseChange {
    before: colors.iter().map(|color| color.noise.before).sum::<f32>() / count,
    after: colors.iter().map(|color| color.noise.after).sum::<f32>() / count,
  }
}

/// Squared change of all stabilized values, the map to the planes is orthonormal
fn overall_error(planes: &[PlaneStats]) -> f64 {
  planes.iter().map(|plane| plane.squared_error).sum()
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::imgop::{Dim2, Point, Rect};
  use crate::pixarray::ImageViewMut;

  const FULL: DenoiseStrength = DenoiseStrength { luma: 1.0, chroma: 1.0 };
  const NO_WB: [f32; 4] = [1.0; 4];

  /// Random values of a normal distribution, from a fixed seed
  struct Normal(u64);

  impl Normal {
    fn uniform(&mut self) -> f64 {
      self.0 ^= self.0 << 13;
      self.0 ^= self.0 >> 7;
      self.0 ^= self.0 << 17;
      ((self.0 >> 11) as f64 + 0.5) / (1_u64 << 53) as f64
    }

    fn next(&mut self) -> f32 {
      ((-2.0 * self.uniform().ln()).sqrt() * (std::f64::consts::TAU * self.uniform()).cos()) as f32
    }
  }

  /// Image with shot noise on the values of `base`, for a position and color plane
  fn noisy_image(width: usize, height: usize, channels: usize, base: impl Fn(usize, usize, usize) -> f32) -> Vec<f32> {
    let mut normal = Normal(0x9e37_79b9_7f4a_7c15);
    let mut data = Vec::with_capacity(width * height * channels);
    for y in 0..height {
      for x in 0..width {
        for c in 0..channels {
          let value = base(x, y, c);
          data.push(value + (0.001 * value).sqrt() * normal.next());
        }
      }
    }
    data
  }

  fn region(data: &mut [f32], width: usize, height: usize, channels: usize) -> ImageRegionMut<'_, f32> {
    ImageRegionMut::from(ImageViewMut::new(data, width, height, channels))
  }

  /// Mean and standard deviation of plane `c` in columns `columns` of the image
  fn stats(data: &[f32], width: usize, channels: usize, c: usize, columns: std::ops::Range<usize>) -> (f32, f32) {
    let values: Vec<f32> = data
      .chunks_exact(width * channels)
      .flat_map(|row| row[columns.start * channels..columns.end * channels].iter().skip(c).step_by(channels).copied())
      .collect();
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / values.len() as f32;
    (mean, variance.sqrt())
  }

  /// Stabilized luma and chroma values of all pixels
  fn planes_of(data: &[f32], channels: usize, map: &[[f32; 4]; 4]) -> Vec<f32> {
    let mut planes = data.to_vec();
    planes.chunks_exact_mut(channels).for_each(|pixel| to_planes(pixel, map, &NO_WB));
    planes
  }

  #[test]
  fn default_strength_is_active() {
    assert!(DenoiseStrength::default().is_active());
    assert!(!DenoiseStrength::none().is_active());
    assert!(DenoiseStrength { luma: 0.0, chroma: 0.1 }.is_active());
    assert!(!DenoiseStrength { luma: f32::NAN, chroma: 0.0 }.is_active());
  }

  #[test]
  fn noise_is_reduced_and_mean_and_edges_are_kept() -> crate::Result<()> {
    let (width, height) = (128, 128);
    let base = |x: usize, _: usize, c: usize| (if x < 64 { 0.1 } else { 0.5 }) * (1.0 - 0.1 * c as f32);
    let input = noisy_image(width, height, 3, base);
    let mut output = input.clone();
    wavelet_denoise(region(&mut output, width, height, 3), FULL, &NO_WB)?;
    for c in 0..3 {
      for columns in [8..56, 72..120] {
        let (mean, deviation) = stats(&output, width, 3, c, columns.clone());
        let (input_mean, input_deviation) = stats(&input, width, 3, c, columns);
        assert!(deviation < 0.7 * input_deviation, "{} of {}", deviation, input_deviation);
        assert!((mean - input_mean).abs() < 0.02 * input_mean, "{} of {}", mean, input_mean);
      }
      // The edge stays between two columns
      let (left, _) = stats(&output, width, 3, c, 62..63);
      let (right, _) = stats(&output, width, 3, c, 65..66);
      assert!(right - left > 0.75 * (base(100, 0, c) - base(0, 0, c)), "{} {}", left, right);
    }
    Ok(())
  }

  #[test]
  fn region_is_denoised_like_a_copy_of_it() -> crate::Result<()> {
    let (width, height) = (64, 48);
    let area = Rect::new(Point::new(8, 6), Dim2::new(40, 32));
    let input = noisy_image(width, height, 3, |x, y, _| 0.1 + 0.005 * x as f32 + 0.002 * y as f32);
    let mut output = input.clone();
    wavelet_denoise(ImageRegionMut::new(&mut output, width, height, 3, area), FULL, &NO_WB)?;

    let inside = |x: usize, y: usize| (area.p.x..area.p.x + area.d.w).contains(&x) && (area.p.y..area.p.y + area.d.h).contains(&y);
    let mut copy = Vec::new();
    for y in 0..height {
      for x in 0..width {
        let pixel = (y * width + x) * 3;
        if inside(x, y) {
          copy.extend_from_slice(&input[pixel..pixel + 3]);
        } else {
          assert_eq!(output[pixel..pixel + 3], input[pixel..pixel + 3], "{} {}", x, y);
        }
      }
    }
    wavelet_denoise(region(&mut copy, area.d.w, area.d.h, 3), FULL, &NO_WB)?;
    let denoised: Vec<f32> = (area.p.y..area.p.y + area.d.h)
      .flat_map(|y| output[(y * width + area.p.x) * 3..(y * width + area.p.x + area.d.w) * 3].to_vec())
      .collect();
    assert_eq!(denoised, copy);
    assert_ne!(denoised, input[..denoised.len()]);
    Ok(())
  }

  #[test]
  fn strengths_are_separate_for_luma_and_chroma() -> crate::Result<()> {
    let (width, height) = (64, 64);
    let input = noisy_image(width, height, 3, |_, _, c| 0.2 + 0.1 * c as f32);
    let before = planes_of(&input, 3, &PLANES_3);
    for (strength, changed) in [
      (DenoiseStrength { luma: 1.0, chroma: 0.0 }, [true, false, false]),
      (DenoiseStrength { luma: 0.0, chroma: 1.0 }, [false, true, true]),
    ] {
      let mut output = input.clone();
      wavelet_denoise(region(&mut output, width, height, 3), strength, &NO_WB)?;
      let after = planes_of(&output, 3, &PLANES_3);
      for (plane, changed) in changed.iter().enumerate() {
        let largest = before.iter().zip(&after).skip(plane).step_by(3).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        assert_eq!(largest > 1e-3, *changed, "{:?}: plane {} changed by {}", strength, plane, largest);
      }
    }
    Ok(())
  }

  #[test]
  fn none_leaves_the_image_unchanged() -> crate::Result<()> {
    let input = noisy_image(32, 32, 3, |_, _, _| 0.2);
    let mut output = input.clone();
    wavelet_denoise(region(&mut output, 32, 32, 3), DenoiseStrength::none(), &NO_WB)?;
    assert_eq!(input, output);
    Ok(())
  }

  #[test]
  fn monochrome_and_four_planes() -> crate::Result<()> {
    for channels in [1, 4] {
      let input = noisy_image(64, 64, channels, |_, _, c| 0.2 + 0.05 * c as f32);
      let mut output = input.clone();
      wavelet_denoise(region(&mut output, 64, 64, channels), FULL, &NO_WB)?;
      for c in 0..channels {
        let (_, deviation) = stats(&output, 64, channels, c, 0..64);
        let (_, input_deviation) = stats(&input, 64, channels, c, 0..64);
        assert!(deviation < 0.8 * input_deviation, "{} planes: {} of {}", channels, deviation, input_deviation);
      }
    }
    assert!(wavelet_denoise(region(&mut [0.0; 8], 2, 2, 2), FULL, &NO_WB).is_err());
    Ok(())
  }

  #[test]
  fn plane_maps_are_orthonormal() {
    for (map, planes) in [(&PLANES_1, 1), (&PLANES_3, 3), (&PLANES_4, 4)] {
      for (i, a) in map.iter().enumerate().take(planes) {
        for (j, b) in map.iter().enumerate().take(planes) {
          let product: f32 = a.iter().zip(b).map(|(a, b)| a * b).sum();
          assert!(
            (product - if i == j { 1.0 } else { 0.0 }).abs() < 1e-5,
            "{} planes: {} {}: {}",
            planes,
            i,
            j,
            product
          );
        }
      }
    }
  }

  #[test]
  fn small_regions() -> crate::Result<()> {
    for (width, height) in [(1, 1), (2, 1), (1, 2), (2, 2), (3, 3), (31, 5), (5, 31), (33, 33)] {
      let input = noisy_image(width, height, 3, |_, _, _| 0.3);
      let mut output = input.clone();
      wavelet_denoise(region(&mut output, width, height, 3), FULL, &NO_WB)?;
      assert!(output.iter().all(|v| v.is_finite()), "{}x{}", width, height);
    }
    // Without a detail band, the values are only transformed and back
    let input = noisy_image(1, 1, 3, |_, _, _| 0.3);
    let mut output = input.clone();
    wavelet_denoise(region(&mut output, 1, 1, 3), FULL, &NO_WB)?;
    assert!(input.iter().zip(&output).all(|(a, b)| (a - b).abs() < 1e-5), "{:?} {:?}", input, output);
    Ok(())
  }

  #[test]
  fn constant_image_is_kept() -> crate::Result<()> {
    let mut output = vec![0.25_f32; 64 * 64 * 3];
    let stats = denoise_region(region(&mut output, 64, 64, 3), FULL, &NO_WB, true)?;
    assert!(output.iter().all(|v| (v - 0.25).abs() < 1e-5));
    assert!(stats.planes.iter().flat_map(|plane| &plane.levels).all(|level| level.sigma == 0.0));
    assert!(stats.colors.iter().all(|color| color.noise == NoiseChange::default()));
    assert_eq!(average_noise(&stats.colors).percent_left(), 100.0);
    Ok(())
  }

  #[test]
  fn white_balance_is_undone() -> crate::Result<()> {
    let wb = [2.0, 1.0, 1.5, f32::NAN];
    let base = |_: usize, _: usize, c: usize| 0.2 / [2.0, 1.0, 1.5][c];
    let input = noisy_image(64, 64, 3, base);
    let mut output = input.clone();
    wavelet_denoise(region(&mut output, 64, 64, 3), FULL, &wb)?;
    for c in 0..3 {
      let (mean, _) = stats(&output, 64, 3, c, 0..64);
      assert!((mean - base(0, 0, c)).abs() < 0.02 * base(0, 0, c), "{}: {}", c, mean);
    }
    Ok(())
  }

  #[test]
  fn noise_estimate_ignores_flat_blocks() {
    let (width, height) = (256, 256);
    let mut normal = Normal(0x1234_5678_9abc_def1);
    // The left half has no noise, like clipped highlights
    let band: Vec<f32> = (0..width * height)
      .map(|i| if i % width < width / 2 { 0.0 } else { 0.01 * normal.next() })
      .collect();
    let estimate_noise = |band: &[f32], width, height| noise_of_blocks(&block_deviations(band, width, height));
    let sigma = estimate_noise(&band, width, height);
    assert!((sigma - 0.01).abs() < 0.002, "{}", sigma);
    assert_eq!(estimate_noise(&vec![0.5; width * height], width, height), 0.0);
    // Smaller than a block
    let band: Vec<f32> = (0..16 * 8).map(|_| 0.01 * normal.next()).collect();
    assert!((estimate_noise(&band, 16, 8) - 0.01).abs() < 0.003);
  }

  #[test]
  fn psnr_of_known_errors() {
    assert!((psnr(1e-4) - 40.0).abs() < 1e-9);
    assert!((psnr(1.0)).abs() < 1e-9);
    assert_eq!(psnr(0.0), f64::INFINITY);
  }

  #[test]
  fn statistics_have_no_effect_on_the_result() -> crate::Result<()> {
    let input = noisy_image(64, 64, 3, |_, _, c| 0.2 + 0.1 * c as f32);
    let (mut with, mut without) = (input.clone(), input.clone());
    let stats_with = denoise_region(region(&mut with, 64, 64, 3), FULL, &NO_WB, true)?;
    let stats_without = denoise_region(region(&mut without, 64, 64, 3), FULL, &NO_WB, false)?;
    assert_eq!(with, without);
    assert_eq!((stats_with.planes.len(), stats_with.colors.len()), (3, 3));
    assert!(stats_with.planes.iter().all(|plane| plane.squared_error > 0.0));
    assert!(stats_with.planes.iter().flat_map(|plane| &plane.levels).any(|level| level.removed > 0));
    assert!(stats_without.colors.is_empty());
    assert!(
      stats_without
        .planes
        .iter()
        .all(|plane| plane.squared_error == 0.0 && plane.noise == NoiseChange::default())
    );
    assert!(stats_without.planes.iter().flat_map(|plane| &plane.levels).all(|level| level.removed == 0));
    assert_eq!(
      stats_with
        .planes
        .iter()
        .flat_map(|plane| &plane.levels)
        .map(|level| level.sigma)
        .collect::<Vec<f32>>(),
      stats_without
        .planes
        .iter()
        .flat_map(|plane| &plane.levels)
        .map(|level| level.sigma)
        .collect::<Vec<f32>>()
    );
    Ok(())
  }

  #[test]
  fn plane_errors_add_up_to_the_change_of_the_colors() -> crate::Result<()> {
    let input = noisy_image(64, 64, 3, |_, _, c| 0.2 + 0.1 * c as f32);
    let mut output = input.clone();
    let stats = denoise_region(region(&mut output, 64, 64, 3), FULL, &NO_WB, true)?;
    let stabilize = |v: f32| (f64::from(v.max(0.0)) + f64::from(STABILIZER_OFFSET)).sqrt();
    let expected: f64 = input.iter().zip(&output).map(|(a, b)| (stabilize(*a) - stabilize(*b)).powi(2)).sum();
    let error = overall_error(&stats.planes);
    assert!((error - expected).abs() < 0.01 * expected, "{} {}", error, expected);
    Ok(())
  }

  #[test]
  fn statistics_show_the_noise_before_and_after() -> crate::Result<()> {
    let input = noisy_image(128, 128, 3, |_, _, c| 0.2 + 0.1 * c as f32);
    let mut output = input.clone();
    let stats = denoise_region(region(&mut output, 128, 128, 3), FULL, &NO_WB, true)?;
    // Luma and chroma planes, and all of their detail bands
    assert_eq!(stats.planes.len(), 3);
    for plane in &stats.planes {
      assert!(
        plane.noise.before > 0.0 && plane.noise.after < 0.7 * plane.noise.before,
        "{}: {}",
        plane.plane,
        plane.noise
      );
      assert_eq!(plane.levels.len(), LEVELS);
      for level in &plane.levels {
        assert!(
          level.noise.before > 0.0 && level.noise.after < level.noise.before,
          "{}: {}",
          plane.plane,
          level.noise
        );
      }
    }
    // Color planes: the noise matches the input and the mean is kept
    assert_eq!(stats.colors.len(), 3);
    for color in &stats.colors {
      assert!(
        color.noise.before > 0.0 && color.noise.after < 0.7 * color.noise.before,
        "{}: {}",
        color.color,
        color.noise
      );
      let expected = 0.2 + 0.1 * color.color as f32;
      assert!((color.mean_before - expected).abs() < 0.01, "{}", color.mean_before);
      assert!(
        (color.mean_after - color.mean_before).abs() < 2e-4,
        "{} {}",
        color.mean_after,
        color.mean_before
      );
      // Removed variance lowers the mean, the offset brings it back
      assert!(color.restored > 0.0, "{}", color.restored);
    }
    let average = average_noise(&stats.colors);
    let sum: f32 = stats.colors.iter().map(|color| color.noise.before).sum();
    assert!((average.before - sum / 3.0).abs() < 1e-6);
    assert!(average.percent_left() < 70.0);

    // Only planes with a strength are denoised, all colors are measured
    let mut output = input.clone();
    let strength = DenoiseStrength { luma: 0.0, chroma: 1.0 };
    let stats = denoise_region(region(&mut output, 128, 128, 3), strength, &NO_WB, true)?;
    assert_eq!(stats.planes.iter().map(|plane| plane.plane).collect::<Vec<usize>>(), [1, 2]);
    assert_eq!(stats.colors.len(), 3);
    Ok(())
  }

  /// A strong reduction empties the flat blocks. The noise after it must not be
  /// measured on the textured blocks, which are the flattest ones of the output.
  #[test]
  fn noise_after_is_measured_on_the_flat_blocks_of_the_input() -> crate::Result<()> {
    let base = |x: usize, y: usize, _: usize| {
      if x < 128 {
        0.2
      } else {
        0.3 + 0.25 * ((x as f32 / 3.0).sin() * (y as f32 / 3.0).cos())
      }
    };
    let input = noisy_image(256, 128, 3, base);
    let mut output = input.clone();
    let strength = DenoiseStrength { luma: 4.0, chroma: 4.0 };
    let stats = denoise_region(region(&mut output, 256, 128, 3), strength, &NO_WB, true)?;
    for plane in &stats.planes {
      assert!(plane.noise.after < 0.5 * plane.noise.before, "{}: {}", plane.plane, plane.noise);
      for level in &plane.levels {
        assert!(level.noise.after < 0.5 * level.noise.before, "{}: {}", plane.plane, level.noise);
      }
    }
    for color in &stats.colors {
      assert!(color.noise.after < 0.5 * color.noise.before, "{}: {}", color.color, color.noise);
    }
    Ok(())
  }

  /// The mean of dark values and of the noisiest color is kept
  #[test]
  fn local_means_are_kept() -> crate::Result<()> {
    let tones = [0.02, 0.1, 0.4];
    let (width, height) = (32 * 6, 32 * 4);
    // Vertical stripes of 64 pixels with one tone
    let base = |x: usize, _: usize, _: usize| tones[(x / 64).min(2)];
    let input = noisy_image(width, height, 3, base);
    // See restore_block_means for the highest strengths
    for strength in [DenoiseStrength::default(), FULL, DenoiseStrength { luma: 2.0, chroma: 2.0 }] {
      let mut output = input.clone();
      wavelet_denoise(region(&mut output, width, height, 3), strength, &NO_WB)?;
      for (stripe, tone) in tones.iter().enumerate() {
        // Without the borders between the stripes, where the blocks of two tones are interpolated
        let columns = stripe * 64 + 24..stripe * 64 + 40;
        for c in 0..3 {
          let (mean, _) = stats(&output, width, 3, c, columns.clone());
          let (input_mean, _) = stats(&input, width, 3, c, columns.clone());
          assert!(
            (mean - input_mean).abs() < 0.01 * tone,
            "{:?}: tone {}: {} of {}",
            strength,
            tone,
            mean,
            input_mean
          );
        }
      }
    }
    Ok(())
  }

  #[test]
  fn block_means_are_restored() {
    let (width, height) = (64 + 7, 32 + 3);
    let original: Vec<f32> = (0..width * height * 3).map(|i| 0.2 + 0.0001 * (i % 97) as f32).collect();
    let means = block_means(&region(&mut original.clone(), width, height, 3));
    assert_eq!(means.len(), 2 * 3);
    // A constant change of each color plane is undone exactly, also for the
    // pixels outside of the full blocks
    let mut changed: Vec<f32> = original
      .chunks_exact(3)
      .flat_map(|pixel| [pixel[0] - 0.01, pixel[1] - 0.02, pixel[2] + 0.03])
      .collect();
    let restored = restore_block_means(&mut region(&mut changed, width, height, 3), &means);
    assert!(restored.iter().zip([0.01, 0.02, -0.03]).all(|(a, b)| (a - b).abs() < 1e-5), "{:?}", restored);
    assert!(original.iter().zip(&changed).all(|(a, b)| (a - b).abs() < 1e-5));
    // Negative values are set to zero
    let mut changed = vec![0.001_f32; 32 * 32];
    let restored = restore_block_means(&mut region(&mut changed, 32, 32, 1), &[0.0]);
    assert!((restored[0] + 0.001).abs() < 1e-6);
    assert!(changed.iter().all(|v| *v == 0.0));
  }

  #[test]
  fn block_positions_of_interpolation() {
    // Before the first center and after the last one, the offset of the block is used
    assert_eq!(block_position(0, 32, 3), (0, 1, 0.0));
    assert_eq!(block_position(15, 32, 3).0, 0);
    assert_eq!(block_position(95, 32, 3), (2, 2, 0.0));
    // Half way between two centers
    let (first, second, weight) = block_position(32, 32, 3);
    assert_eq!((first, second), (0, 1));
    assert!((weight - 0.515625).abs() < 1e-6, "{}", weight);
    assert_eq!(block_position(5, 8, 1), (0, 0, 0.0));
  }

  #[test]
  fn flat_blocks_of_deviations() {
    let deviations = [0.0, 0.01, 0.02, 0.03, 0.04, 0.05, 0.06, 0.07, 0.08];
    // Without the block that has no variation, the fourth part of the eight others
    assert_eq!(noise_of_blocks(&deviations), 0.03);
    assert_eq!(flat_blocks(&deviations), [1, 2, 3]);
    let noise = block_noise(&deviations, &flat_blocks(&deviations));
    assert!((noise - ((0.0001_f32 + 0.0004 + 0.0009) / 3.0).sqrt()).abs() < 1e-6, "{}", noise);
    assert_eq!(block_noise(&deviations, &[]), 0.0);
    assert!(flat_blocks(&[0.0; 4]).is_empty());
  }

  #[test]
  fn noise_change_display() {
    let noise = NoiseChange { before: 0.02, after: 0.005 };
    assert_eq!(noise.percent_left(), 25.0);
    assert_eq!(noise.to_string(), "noise 0.02000 -> 0.00500 (25.0% left)");
    assert_eq!(NoiseChange::default().percent_left(), 100.0);
  }
}
