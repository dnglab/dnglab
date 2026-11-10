// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

use std::time::Instant;

use multiversion::multiversion;

use crate::{
  cfa::{CFA, PlaneColor},
  imgop::{
    Dim2,
    sensor::{
      Demosaic,
      border::interpolate_border,
      tiles::{HaloTile, halo_tiles, inner_area, par_tiles},
    },
  },
  pixarray::{Color2D, DisjointRegionMut, ImageRegion, ImageRegionMut},
};

/// Pixels at the edges that VNG leaves to [`interpolate_border`], and the halo
/// of its tiles: the reach of the gradients and of the bilinear values it
/// averages
const HALO: usize = 2;

/// Width and height of the windows of the tiles, with the halo
const TILE: usize = 256;

/// Part of the largest gradient by which a gradient may exceed the threshold
/// and still count as equal: more than the rounding errors of the sums of the
/// gradients, and less than one step of 16-bit integer values for gradients
/// up to 1.5 (with the same directions as the integers of dcraw)
const TIE_TOLERANCE: f32 = 1e-5;

/// Gradient terms of the `vng_interpolate` of dcraw: the positions (y1, x1)
/// and (y2, x2) relative to the pixel, the weight (1 doubles the difference)
/// and the gradients the difference is added to, bit 0 is NW and the bits
/// follow the directions clockwise up to W ([`DIRECTIONS`])
#[rustfmt::skip]
const TERMS: [[i16; 6]; 64] = [
  [-2,-2,0,-1,0,0x01], [-2,-2,0,0,1,0x01], [-2,-1,-1,0,0,0x01],
  [-2,-1,0,-1,0,0x02], [-2,-1,0,0,0,0x03], [-2,-1,0,1,1,0x01],
  [-2,0,0,-1,0,0x06], [-2,0,0,0,1,0x02], [-2,0,0,1,0,0x03],
  [-2,1,-1,0,0,0x04], [-2,1,0,-1,1,0x04], [-2,1,0,0,0,0x06],
  [-2,1,0,1,0,0x02], [-2,2,0,0,1,0x04], [-2,2,0,1,0,0x04],
  [-1,-2,-1,0,0,0x80], [-1,-2,0,-1,0,0x01], [-1,-2,1,-1,0,0x01],
  [-1,-2,1,0,1,0x01], [-1,-1,-1,1,0,0x88], [-1,-1,1,-2,0,0x40],
  [-1,-1,1,-1,0,0x22], [-1,-1,1,0,0,0x33], [-1,-1,1,1,1,0x11],
  [-1,0,-1,2,0,0x08], [-1,0,0,-1,0,0x44], [-1,0,0,1,0,0x11],
  [-1,0,1,-2,1,0x40], [-1,0,1,-1,0,0x66], [-1,0,1,0,1,0x22],
  [-1,0,1,1,0,0x33], [-1,0,1,2,1,0x10], [-1,1,1,-1,1,0x44],
  [-1,1,1,0,0,0x66], [-1,1,1,1,0,0x22], [-1,1,1,2,0,0x10],
  [-1,2,0,1,0,0x04], [-1,2,1,0,1,0x04], [-1,2,1,1,0,0x04],
  [0,-2,0,0,1,0x80], [0,-1,0,1,1,0x88], [0,-1,1,-2,0,0x40],
  [0,-1,1,0,0,0x11], [0,-1,2,-2,0,0x40], [0,-1,2,-1,0,0x20],
  [0,-1,2,0,0,0x30], [0,-1,2,1,1,0x10], [0,0,0,2,1,0x08],
  [0,0,2,-2,1,0x40], [0,0,2,-1,0,0x60], [0,0,2,0,1,0x20],
  [0,0,2,1,0,0x30], [0,0,2,2,1,0x10], [0,1,1,0,0,0x44],
  [0,1,1,2,0,0x10], [0,1,2,-1,1,0x40], [0,1,2,0,0,0x60],
  [0,1,2,1,0,0x20], [0,1,2,2,0,0x10], [1,-2,1,0,0,0x80],
  [1,-1,1,1,0,0x88], [1,0,1,2,0,0x08], [1,0,2,-1,0,0x40],
  [1,0,2,1,0,0x10],
];

/// Directions of the gradients, clockwise from NW to W, as (row, column)
const DIRECTIONS: [(isize, isize); 8] = [(-1, -1), (-1, 0), (-1, 1), (0, 1), (1, 1), (1, 0), (1, -1), (0, -1)];

/// VNG demosaic of CFA data of any pattern, with 3 or 4 planes
///
/// "Interpolation using a Threshold-based variable number of gradients" of
/// Ting Chen, extended by Dave Coffin to other CFAs than Bayer, in the form of
/// the `lin_interpolate` and `vng_interpolate` of dcraw:
///
/// 1. The image is interpolated bilinearly, from the samples of each color in
///    the 3x3 neighbourhood, the orthogonal neighbours with twice the weight of
///    the diagonal ones.
/// 2. For each pixel, 8 gradients (N, NE, E, ...) are sums of the differences
///    of samples of one color in the 5x5 neighbourhood.
/// 3. The directions whose gradient is at most the smallest plus half the
///    largest gradient are averaged: the bilinear values of the neighbour in
///    the direction, and for the plane of the pixel the mean of its sample and
///    the sample two pixels away, if that has its color. Each other plane is
///    the sample of the pixel plus the difference of the averages of the
///    plane and of the plane of the pixel.
///
/// The CFA can have any size, e.g. Bayer RGB, 4-color Bayer (CYGM, RGBE) and
/// X-Trans data. The image is processed in tiles with a halo of 2 pixels in
/// parallel, the 2 pixels at the edges are interpolated bilinearly
/// ([`interpolate_border`]).
///
/// Unlike dcraw, it computes with floats and doesn't clip values above
/// white. dcraw interpolates the pixels next to the edges with the weights of
/// step 1, [`interpolate_border`] without: for Bayer and 4-color data, where
/// each color is either only orthogonal or only diagonal to a pixel, both are
/// the same.
///
/// # Panics
///
/// Panics if `colors` doesn't have N planes or a CFA color has no plane.
pub struct VngDemosaic {
  /// Size of the cores of the tiles, the area each tile writes
  core: Dim2,
}

impl Default for VngDemosaic {
  fn default() -> Self {
    Self::new()
  }
}

impl VngDemosaic {
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

impl<const N: usize> Demosaic<f32, N> for VngDemosaic
where
  [f32; N]: Default,
{
  fn demosaic(&self, pixels: ImageRegion<'_, f32>, cfa: &CFA, colors: &PlaneColor) -> Color2D<f32, N> {
    assert_eq!(pixels.channels(), 1, "Mosaic data must have one channel");
    assert_eq!(colors.plane_count(), N, "VNG for {} planes needs {} color planes", N, N);
    let now = Instant::now();
    // The ROI changes the pattern if not perfectly aligned on the origin pattern
    let roi = pixels.area();
    let cfa_roi = cfa.shift(roi.p.x, roi.p.y);
    let phases = Phases::new(&cfa_roi, colors, N);

    let mut out = Color2D::<f32, N>::new(roi.width(), roi.height());
    interpolate_border(pixels, cfa, colors, &mut out, HALO);
    let tiles = halo_tiles(roi.d, inner_area(roi.d, HALO), self.core, HALO);
    par_tiles(ImageRegionMut::from(out.view_mut()), &tiles, Buffers::default, |buffers, tile, region| {
      vng_tile::<N>(pixels, &phases, tile, buffers, region);
    });
    log::debug!("VNG debayer time: {:.5}s", now.elapsed().as_secs_f32());
    out
  }
}

/// Interpolation of the pixels of one phase of the CFA, with offsets of type
/// `O`: (row, column) relative to the pixel, or offsets in a tile buffer
#[derive(Clone, Debug, PartialEq)]
struct Phase<O> {
  /// Plane of the pixel
  own: usize,
  /// Neighbours in the 3x3 neighbourhood with another plane than the pixel:
  /// position, plane and weight, 2 for orthogonal and 1 for diagonal ones
  bilinear: Vec<(O, usize, f32)>,
  /// Inverse of the sum of the weights of each plane in `bilinear`, 0 for a
  /// plane without neighbour
  norm: Vec<f32>,
  /// Gradient terms with two positions of one color: the positions, the
  /// factor of their difference and the gradients it is added to
  terms: Vec<(O, O, f32, u8)>,
  /// For each direction the neighbour and, if the neighbour has another plane
  /// than the pixel and the pixel after it has the plane of the pixel, that
  /// pixel
  neighbours: [(O, Option<O>); 8],
}

/// [`Phase`]s of a CFA, row by row
#[derive(Clone, Debug)]
struct Phases {
  /// Rows and columns of the CFA pattern
  rows: usize,
  cols: usize,
  phases: Vec<Phase<(isize, isize)>>,
}

impl Phases {
  /// Phases of `cfa` with the CFA colors mapped to `planes` planes by `colors`
  ///
  /// # Panics
  /// Panics if a CFA color has no plane.
  fn new(cfa: &CFA, colors: &PlaneColor, planes: usize) -> Self {
    let plane_map = colors.plane_lookup_table();
    let (rows, cols) = (cfa.height.max(1), cfa.width.max(1));
    let phases = (0..rows)
      .flat_map(|row| (0..cols).map(move |col| (row, col)))
      .map(|(row, col)| {
        // The pattern repeats, color_at wraps the small negative positions
        let plane = |dy: isize, dx: isize| {
          let color = cfa.color_at(row.wrapping_add_signed(dy), col.wrapping_add_signed(dx));
          let plane = plane_map[color];
          assert!(plane < planes, "CFA color {} of '{}' has no plane in {} planes", color, cfa, planes);
          plane
        };
        let own = plane(0, 0);
        let mut bilinear = Vec::new();
        let mut weights = vec![0.0; planes];
        for (dy, dx) in (-1..=1).flat_map(|dy| (-1..=1).map(move |dx| (dy, dx))) {
          let p = plane(dy, dx);
          if p != own {
            let weight = if dy == 0 || dx == 0 { 2.0 } else { 1.0 };
            bilinear.push(((dy, dx), p, weight));
            weights[p] += weight;
          }
        }
        let norm = weights.iter().map(|&w| if w > 0.0 { 1.0 / w } else { 0.0 }).collect();
        let terms = TERMS
          .iter()
          .filter_map(|&[y1, x1, y2, x2, weight, gradients]| {
            let (y1, x1, y2, x2) = (y1 as isize, x1 as isize, y2 as isize, x2 as isize);
            let color = plane(y1, x1);
            if plane(y2, x2) != color {
              return None;
            }
            // Diagonal pairs of a color that is next to the pixel in a
            // checkerboard have a distance of 2, others of 1
            let diag = if plane(0, 1) == color && plane(1, 0) == color { 2 } else { 1 };
            if (y1 - y2).abs() == diag && (x1 - x2).abs() == diag {
              return None;
            }
            Some(((y1, x1), (y2, x2), if weight == 1 { 2.0 } else { 1.0 }, gradients as u8))
          })
          .collect();
        let neighbours = DIRECTIONS.map(|(dy, dx)| {
          let far = (plane(dy, dx) != own && plane(2 * dy, 2 * dx) == own).then_some((2 * dy, 2 * dx));
          ((dy, dx), far)
        });
        Phase {
          own,
          bilinear,
          norm,
          terms,
          neighbours,
        }
      })
      .collect();
    Self { rows, cols, phases }
  }

  /// The phases with offsets in a buffer of `width` pixels per row
  fn offsets(&self, width: usize) -> Vec<Phase<isize>> {
    let offset = |(dy, dx): (isize, isize)| dy * width as isize + dx;
    self
      .phases
      .iter()
      .map(|phase| Phase {
        own: phase.own,
        bilinear: phase.bilinear.iter().map(|&(pos, plane, weight)| (offset(pos), plane, weight)).collect(),
        norm: phase.norm.clone(),
        terms: phase
          .terms
          .iter()
          .map(|&(a, b, factor, gradients)| (offset(a), offset(b), factor, gradients))
          .collect(),
        neighbours: phase.neighbours.map(|(near, far)| (offset(near), far.map(offset))),
      })
      .collect()
  }
}

/// Buffers of a tile, reused for the following tiles of a worker
///
/// They are not cleared between the tiles: each step reads only values that
/// the steps before have written for the tile.
#[derive(Default)]
struct Buffers<const N: usize> {
  /// Samples of the window
  raw: Vec<f32>,
  /// Bilinear interpolation of the window
  lin: Vec<[f32; N]>,
  /// Phases with the offsets for windows of `offsets_width` pixels per row
  offsets: Vec<Phase<isize>>,
  offsets_width: usize,
}

/// Demosaic `tile` of `pixels` into `region`, its core
///
/// The bilinear interpolation covers the core and 1 pixel around it, it reads
/// 1 pixel further, which is the window. `phases` are aligned to the top-left
/// pixel of `pixels`.
#[multiversion(targets("x86_64+avx512f+avx512bw+avx512vl", "x86_64+avx+avx2", "x86_64+sse4.1", "aarch64+neon"))]
fn vng_tile<const N: usize>(pixels: ImageRegion<f32>, phases: &Phases, tile: &HaloTile, buffers: &mut Buffers<N>, region: &mut DisjointRegionMut<f32>) {
  let (tw, th) = (tile.window.d.w, tile.window.d.h);
  let core = tile.core_in_window();
  assert!(
    core.p.x == HALO && core.p.y == HALO && core.d.w + 2 * HALO == tw && core.d.h + 2 * HALO == th,
    "Window of {:?} has no halo of {} pixels",
    tile,
    HALO
  );
  if buffers.offsets_width != tw || buffers.offsets.is_empty() {
    buffers.offsets = phases.offsets(tw);
    buffers.offsets_width = tw;
  }
  let Buffers { raw, lin, offsets, .. } = buffers;
  pixels.region(tile.window).copy_into(raw);
  lin.resize(tw * th, [0.0; N]);
  // Phase of a pixel of the window
  let (left, top) = (tile.window.p.x, tile.window.p.y);
  let phase = |row: usize, col: usize| &offsets[((top + row) % phases.rows) * phases.cols + (left + col) % phases.cols];
  let at = |i: usize, offset: isize| i.wrapping_add_signed(offset);

  // Interpolate bilinearly
  for row in 1..th - 1 {
    for col in 1..tw - 1 {
      let i = row * tw + col;
      let phase = phase(row, col);
      let mut pixel = [0.0; N];
      for &(offset, plane, weight) in &phase.bilinear {
        pixel[plane] += weight * raw[at(i, offset)];
      }
      for (value, norm) in pixel.iter_mut().zip(&phase.norm) {
        *value *= norm;
      }
      pixel[phase.own] = raw[i];
      lin[i] = pixel;
    }
  }

  // Average the neighbours in the directions of the small gradients
  for (row, line) in (HALO..).zip(region.rows_mut()) {
    for (col, pixel) in (HALO..).zip(line.as_chunks_mut::<N>().0) {
      let i = row * tw + col;
      let phase = phase(row, col);
      let mut gradients = [0.0_f32; 8];
      for &(a, b, factor, mut mask) in &phase.terms {
        let difference = (raw[at(i, a)] - raw[at(i, b)]).abs() * factor;
        while mask != 0 {
          gradients[mask.trailing_zeros() as usize] += difference;
          mask &= mask - 1;
        }
      }
      let (min, max) = gradients.iter().fold((f32::MAX, f32::MIN), |(min, max), &g| (min.min(g), max.max(g)));
      if max == 0.0 {
        *pixel = lin[i];
        continue;
      }
      // Gradients equal to the threshold are common in data of a low bit
      // depth, the rounding errors of the sums must not exclude them
      let threshold = min + max / 2.0 + max * TIE_TOLERANCE;
      let (own, sample) = (phase.own, raw[i]);
      let mut sum = [0.0; N];
      let mut count = 0;
      for (&gradient, &(near, far)) in gradients.iter().zip(&phase.neighbours) {
        if gradient > threshold {
          continue;
        }
        let neighbour = &lin[at(i, near)];
        for (plane, sum) in sum.iter_mut().enumerate() {
          *sum += match far {
            Some(far) if plane == own => (sample + raw[at(i, far)]) / 2.0,
            _ => neighbour[plane],
          };
        }
        count += 1;
      }
      for (plane, value) in pixel.iter_mut().enumerate() {
        *value = if plane == own {
          sample
        } else {
          (sample + (sum[plane] - sum[own]) / count as f32).max(0.0)
        };
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::pixarray::ImageView;

  const XTRANS: &str = "GGRGGBGGBGGRBRGRBGGGBGGRGGRGGBRBGBRG";

  /// CFA patterns with their plane colors and the number of planes
  const PATTERNS: [(&str, &str); 6] = [
    ("RGGB", "RGB"),
    ("BGGR", "RGB"),
    ("GRBG", "RGB"),
    ("GBRG", "RGB"),
    (XTRANS, "RGB"),
    ("CYGM", "CYGM"),
  ];

  fn mosaic(width: usize, height: usize) -> Vec<f32> {
    (0..width * height).map(|i| (i * 7919 % 1000) as f32 / 1000.0).collect()
  }

  fn demosaic<const N: usize>(algorithm: &VngDemosaic, samples: &[f32], width: usize, height: usize, pattern: &str, colors: &str) -> Color2D<f32, N>
  where
    [f32; N]: Default,
  {
    algorithm.demosaic(ImageView::new(samples, width, height, 1).into(), &CFA::new(pattern), &PlaneColor::new(colors))
  }

  /// Run `check` for each pattern with the number of planes of its colors
  fn for_each_pattern(check: impl Fn(&str, &str, &dyn Fn(&[f32], usize, usize, Option<usize>) -> Vec<Vec<f32>>)) {
    for (pattern, colors) in PATTERNS {
      // Pixels of the demosaic as vectors of their planes, with tiles of the size or the default
      let run = |samples: &[f32], width: usize, height: usize, tile: Option<usize>| -> Vec<Vec<f32>> {
        let algorithm = tile.map_or_else(VngDemosaic::new, VngDemosaic::with_tile);
        if colors.len() == 4 {
          demosaic::<4>(&algorithm, samples, width, height, pattern, colors)
            .pixels()
            .iter()
            .map(|p| p.to_vec())
            .collect()
        } else {
          demosaic::<3>(&algorithm, samples, width, height, pattern, colors)
            .pixels()
            .iter()
            .map(|p| p.to_vec())
            .collect()
        }
      };
      check(pattern, colors, &run);
    }
  }

  #[test]
  fn tiling_doesnt_change_the_result() {
    // Tiles with cores of 1, 5 and 16 pixels against one tile for the image
    let (width, height) = (53, 41);
    let samples = mosaic(width, height);
    for_each_pattern(|pattern, _, run| {
      let expected = run(&samples, width, height, Some(100));
      for tile in [5, 9, 20] {
        assert!(
          run(&samples, width, height, Some(tile)) == expected,
          "{} with tiles of {} pixels",
          pattern,
          tile
        );
      }
    });
  }

  #[test]
  fn small_images_keep_their_samples() {
    // Images up to 4 pixels high or wide are border only
    for_each_pattern(|pattern, colors, run| {
      let (cfa, plane_map) = (CFA::new(pattern), PlaneColor::new(colors).plane_lookup_table());
      for (width, height) in (1..=9).flat_map(|width| (1..=9).map(move |height| (width, height))) {
        let samples = mosaic(width, height);
        let pixels = run(&samples, width, height, None);
        assert_eq!(pixels.len(), width * height);
        for (i, pixel) in pixels.iter().enumerate() {
          let (row, col) = (i / width, i % width);
          assert_eq!(
            pixel[plane_map[cfa.color_at(row, col)]],
            samples[i],
            "{} {}x{} at {}:{}",
            pattern,
            width,
            height,
            col,
            row
          );
        }
      }
    });
  }

  #[test]
  fn flat_colors_stay_flat() {
    let (width, height) = (40, 30);
    let values = [0.2, 0.5, 0.3, 0.7];
    for_each_pattern(|pattern, colors, run| {
      let (cfa, plane_map) = (CFA::new(pattern), PlaneColor::new(colors).plane_lookup_table());
      let samples: Vec<f32> = (0..width * height).map(|i| values[plane_map[cfa.color_at(i / width, i % width)]]).collect();
      // The pixels of VNG, at the edges the 3x3 neighbourhood of an X-Trans
      // corner has no red or blue pixel for interpolate_border
      let inner = inner_area(Dim2::new(width, height), HALO);
      let is_inner = |x: usize, y: usize| (inner.p.x..inner.p.x + inner.d.w).contains(&x) && (inner.p.y..inner.p.y + inner.d.h).contains(&y);
      for (i, pixel) in run(&samples, width, height, None)
        .iter()
        .enumerate()
        .filter(|(i, _)| is_inner(i % width, i / width))
      {
        for (plane, value) in pixel.iter().enumerate() {
          assert!((value - values[plane]).abs() < 1e-6, "{} {:?} at {}:{}", pattern, pixel, i % width, i / width);
        }
      }
    });
  }

  #[test]
  fn scaled_data_gives_a_scaled_result() {
    // The gradients and the threshold scale with the data. Data of few levels
    // has many gradients equal to the threshold, the rounding errors of other
    // scales must not change the directions of their interpolation.
    let (width, height) = (48, 36);
    let levels: Vec<f32> = (0..width * height).map(|i| ((i * 7919 + i / 5) % 13) as f32).collect();
    for_each_pattern(|pattern, _, run| {
      let (a, b) = (1.0 / 1023.0, 1.0 / 993.0);
      let scaled = |scale: f32| levels.iter().map(|v| v * scale).collect::<Vec<f32>>();
      let (first, second) = (run(&scaled(a), width, height, None), run(&scaled(b), width, height, None));
      for (i, (p, q)) in first.iter().zip(&second).enumerate() {
        for (v, w) in p.iter().zip(q) {
          assert!((v / a - w / b).abs() < 1e-3, "{} {:?} != {:?} at {}:{}", pattern, p, q, i % width, i / width);
        }
      }
    });
  }

  #[test]
  fn phases_of_bayer_data() {
    let phases = Phases::new(&CFA::new("RGGB"), &PlaneColor::new("RGB"), 3);
    assert_eq!((phases.rows, phases.cols), (2, 2));
    let (red, green) = (&phases.phases[0], &phases.phases[1]);
    assert_eq!((red.own, green.own), (0, 1));
    // Red has green orthogonal and blue diagonal neighbours, the weights sum up to 8 and 4
    assert_eq!(red.bilinear.len(), 8);
    assert_eq!(red.norm, [0.0, 1.0 / 8.0, 1.0 / 4.0]);
    // Green has red in its row and blue in its column
    assert_eq!(green.bilinear.len(), 4);
    assert_eq!(green.norm, [1.0 / 4.0, 0.0, 1.0 / 4.0]);
    // Red is two pixels away in all directions, green only orthogonally
    assert!(red.neighbours.iter().all(|(_, far)| far.is_some()));
    let orthogonal: Vec<bool> = green.neighbours.iter().map(|(_, far)| far.is_some()).collect();
    assert_eq!(orthogonal, [false, true, false, true, false, true, false, true]);
    // Terms have two positions of one color, diagonal neighbours of green
    // pixels at a red pixel are no term
    for phase in &phases.phases {
      assert!(!phase.terms.is_empty());
      assert!(phase.terms.iter().all(|&(a, b, _, gradients)| a != b && gradients != 0));
    }
    assert!(
      !red
        .terms
        .iter()
        .any(|&((y1, x1), (y2, x2), _, _)| (y1 - y2).abs() == 2 && (x1 - x2).abs() == 2 && (y1 + x1) % 2 != 0)
    );
  }
}
