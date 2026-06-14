// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>
//
// Initial version written by Roman Kuraev 2026 [1]
// Unfortunately, it was incomplete and broken.
// This implementation is a revisted and cleaned up version to
// complete the algorithm.
// Ref [1] https://github.com/naorunaoru/demosaic/blob/5050131e998b7f6414eef02eeb60f74b99d77451/src/xtrans/markesteijn_impl.rs

use multiversion::multiversion;
use rayon::prelude::*;
use std::time::Instant;

use crate::{
  CFA,
  cfa::{CFA_COLOR_B, CFA_COLOR_G, CFA_COLOR_R, PlaneColor},
  imgop::{
    Rect,
    cielab::XYZ_to_lab,
    matrix::multiply_row1,
    sensor::Demosaic,
    xyz::{CIE_1931_TRISTIMULUS_D65, SRGB_TO_XYZ_D65},
  },
  pixarray::{Color2D, PixF32, SharedColor2D},
};

/// Markesteijn demosaicing implementation for Fujifilm X-Trans sensor data.
///
/// This implements the algorithm described by Markesteijn for demosaicing
/// X-Trans 6x6 CFA patterns. It supports both 1-pass and 3-pass modes:
///
/// - **1-pass**: Interpolates green in 4 directions, then reconstructs R/B channels.
///   Offers good quality with reasonable speed.
/// - **3-pass**: Uses 8 direction buffers with additional green recalculation and
///   cross-direction competition. Produces higher quality at the cost of more
///   computation.
///
/// The algorithm works on overlapping tiles (64x64 with 16px overlap) processed
/// in parallel via Rayon. For each tile, it:
/// 1. Interpolates green using hex-grid neighbor weights in 4 (or 8) directions
/// 2. Reconstructs R/B channels using color-difference interpolation
/// 3. Computes directional gradients in CIELab space
/// 4. Selects the best direction(s) via homogeneity voting over a 5x5 window
///
/// # Quality
///
/// Significantly better than bilinear interpolation. The 3-pass mode produces
/// results comparable to DCB or AMaZE for Bayer sensors. Edge detail and color
/// accuracy are both improved over simpler methods.
#[derive(Default)]
pub struct XTransMarkesteijnDemosaic {
  /// Number of passes (1 or 3). Higher values improve quality at the cost of speed.
  passes: usize,
}

impl XTransMarkesteijnDemosaic {
  /// Create a new Markesteijn demosaicer with the given number of passes.
  ///
  /// Use `passes = 1` for fast, good-quality results, or `passes = 3` for
  /// the highest quality with additional green recalculation.
  pub fn new(passes: usize) -> Self {
    Self { passes }
  }

  pub fn new_pass_1() -> Self {
    Self::new(1)
  }

  pub fn new_pass_3() -> Self {
    Self::new(3)
  }
}

impl Demosaic<f32, 3> for XTransMarkesteijnDemosaic {
  /// Debayer image by using Markesteijn (1-pass) method.
  fn demosaic(&self, pixels: &PixF32, cfa: &CFA, _colors: &PlaneColor, roi: Rect) -> Color2D<f32, 3> {
    if !cfa.is_rgb() {
      panic!("CFA pattern '{}' is not a RGB pattern, can not demosaic with Markesteijn", cfa);
    }
    // Measure time
    let now = Instant::now();
    let input = pixels.crop(roi);

    // The ROI changes the pattern if not perfectly aligned on the origin pattern
    let cfa_roi = cfa.shift(roi.p.x, roi.p.y);

    let result = demosaic_impl(&input.data, roi.width(), roi.height(), &cfa_roi, self.passes);
    log::debug!("X-Trans Markesteijn ({}-pass) debayer time: {:.5}s", self.passes, now.elapsed().as_secs_f32());
    result
  }
}

const TS: usize = 64; // Tile size

fn demosaic_impl(input: &[f32], width: usize, height: usize, cfa: &CFA, passes: usize) -> Color2D<f32, 3> {
  let mut output = Color2D::<f32, 3>::new(width, height);
  let ndir: usize = if passes > 1 { 8 } else { 4 };
  // Cascading padding: each stage adds 1 pixel beyond the prior because it reads
  // neighbors that must themselves be valid. Extra passes propagate edge errors
  // further, requiring +5 additional padding.
  let pad_lab: usize = if passes == 1 { 8 } else { 13 };
  let pad_drv: usize = pad_lab + 1;
  let pad_homo: usize = pad_drv + 1;
  let pad_tile: usize = pad_homo + 2; // +2 for the 5x5 homosum window
  let overlap: usize = pad_tile * 2;

  let allhex = build_hex_lut(cfa);
  let green_bounds = compute_green_bounds(input, width, height, cfa, &allhex);

  border_interpolate(input, width, height, cfa, &mut output, pad_tile);

  // Collect tile coordinates
  let stride = TS - overlap;
  let tiles: Vec<(usize, usize, usize, usize)> = (0..height)
    .step_by(stride)
    .flat_map(|top| {
      (0..width).step_by(stride).filter_map(move |left| {
        let tile_h = TS.min(height - top);
        let tile_w = TS.min(width - left);
        if tile_h > overlap && tile_w > overlap {
          Some((top, left, tile_h, tile_w))
        } else {
          None
        }
      })
    })
    .collect();

  // SAFETY: Each tile writes to a non-overlapping region of the output buffer.
  // Tiles step by `stride` (TS - overlap) but only write pixels within
  // pad_tile..tile_size-pad_tile, so adjacent tiles cover adjacent, non-overlapping pixel ranges.
  let shared_output = SharedColor2D::<f32, 3>::new(output);

  tiles.par_iter().for_each(|&(top, left, tile_h, tile_w)| {
    process_tile(
      input,
      width,
      height,
      cfa,
      &allhex,
      &green_bounds,
      top,
      left,
      tile_h,
      tile_w,
      passes,
      ndir,
      pad_lab,
      pad_drv,
      pad_homo,
      pad_tile,
      &shared_output,
    );
  });

  shared_output.into_inner()
}

/// Hex neighborhood offset lookup table built from orth/patt tables (matching dcraw/LibRaw).
/// hex[row%3][col%3][k][idx] = (dy, dx)
/// k=0: used for green interpolation and green bounds
/// k=1: used for 2x2 green block fill-in
struct HexLut {
  hex: [[[[(i32, i32); 8]; 2]; 3]; 3],
  /// Solitary green row
  sgrow: usize,
  /// Solitary green column
  sgcol: usize,
}

/// Build the hex neighborhood lookup table for the X-Trans CFA pattern.
///
/// The X-Trans 6x6 CFA repeats this pattern (G=green, R=red, B=blue):
///
/// ```text
///     col: 0  1  2  3  4  5
///  row 0:  G  b  G  G  r  G
///  row 1:  r  G  r  b  G  b
///  row 2:  G  b  G  G  r  G
///  row 3:  G  r  G  G  b  G
///  row 4:  b  G  b  r  G  r
///  row 5:  G  r  G  G  b  G
/// ```
///
/// Viewed modulo 3, the green/non-green structure simplifies to a checkerboard
/// with one special "solitary green" at (1,1):
///
/// ```text
///     col%3:  0   1   2
///  row%3=0:   G   X   G        X = non-green (R or B)
///  row%3=1:   X  [G]  X       [G] = solitary green (sgrow=1, sgcol=1)
///  row%3=2:   G   X   G
/// ```
///
/// Green pixels in X-Trans form a hexagonal-like grid. Around each non-green
/// pixel, there are 6 nearest green neighbors arranged in a hex ring.
///
/// # ORTH — Rotation matrices for cardinal direction probing
///
/// `ORTH` encodes 5 direction steps (d=0,2,4,6,8), each giving a 2x2
/// rotation matrix [ORTH[d], ORTH[d+1]; ORTH[d+2], ORTH[d+3]]:
///
/// ```text
///   d=0:  [ 1, 0]   identity     — probe South  (+1, 0)
///         [ 0, 1]
///
///   d=2:  [ 0, 1]   90° CCW      — probe West   ( 0,-1)
///         [-1, 0]
///
///   d=4:  [-1, 0]   180°         — probe North  (-1, 0)
///         [ 0,-1]
///
///   d=6:  [ 0,-1]   270° CCW     — probe East   ( 0,+1)
///         [ 1, 0]
///
///   d=8:  [ 1, 0]   back to 0°   — probe South  (+1, 0)
///         [ 0, 1]
/// ```
///
/// The algorithm probes neighbors in this S→W→N→E→S order, counting
/// consecutive non-green neighbors (`ng`). When `ng` reaches:
/// - `g + 1` (1 for non-green center, 2 for green center): the correct
///   orientation is found and PATT offsets are rotated by the current matrix.
/// - 4: this position is the "solitary green" pixel → sets `sgrow`/`sgcol`.
///
/// # PATT — Canonical hex neighbor offsets (before rotation)
///
/// `PATT[0]` — 6 green neighbors around a **non-green** pixel (+ 2 unused zeros):
///
/// ```text
///    8 (row,col) pairs:  (0,1) (0,-1) (2,0) (-1,0) (1,1) (1,-1) (0,0) (0,0)
///                         [0]    [1]    [2]   [3]    [4]    [5]    [6]   [7]
///
///    Canonical layout (before rotation):
///
///               [3](-1, 0)
///                  \
///       [1](0,-1) — X — [0](0,+1)
///                  / \
///     [5](+1,-1)     [4](+1,+1)
///                 |
///                [2](+2, 0)
/// ```
///
/// These 6 offsets form a hexagonal ring of green pixels around the non-green
/// center X. Slots [6],[7] are zero (unused in green interpolation but the
/// array is 8-wide for alignment with PATT[1]).
///
/// `PATT[1]` — 8 neighbor offsets around a **green** pixel:
///
/// ```text
///    8 (row,col) pairs:  (0,1) (0,-2) (1,0) (-2,0) (1,1) (-2,-2) (1,-1) (-1,1)
///                         [0]    [1]    [2]   [3]    [4]     [5]    [6]    [7]
/// ```
///
/// Used in pairs for the 2x2 green block fill-in step (step 3c of R/B
/// interpolation). Each pair [2i, 2i+1] gives two neighbor offsets for
/// direction buffer `i`.
///
/// # Slot permutation: `c ^ (g * 2 & d)`
///
/// The XOR expression remaps slot indices so that hex entries align with the
/// correct direction buffers across different rotations and pixel types.
/// For non-green (g=0): `c ^ 0 = c` (identity).
/// For green (g=1): `c ^ (2 & d)` flips bit 1 when d has bit 1 set,
/// swapping slot pairs to match the alternating row parity.
#[multiversion(targets("x86_64+avx512f+avx512bw+avx512vl", "x86_64+avx+avx2+fma", "x86_64+sse4.1", "aarch64+neon"))]
fn build_hex_lut(cfa: &CFA) -> HexLut {
  // See doc comment above for detailed explanation of these tables.
  const ORTH: [i32; 12] = [1, 0, 0, 1, -1, 0, 0, -1, 1, 0, 0, 1];
  const PATT: [[i32; 16]; 2] = [
    [0, 1, 0, -1, 2, 0, -1, 0, 1, 1, 1, -1, 0, 0, 0, 0],    // Arround non-green pixel
    [0, 1, 0, -2, 1, 0, -2, 0, 1, 1, -2, -2, 1, -1, -1, 1], // Arround green pixel
  ];

  let mut lut = HexLut {
    hex: [[[[(0i32, 0i32); 8]; 2]; 3]; 3],
    sgrow: 0,
    sgcol: 0,
  };

  for row in 0..3i32 {
    for col in 0..3i32 {
      let mut ng: i32 = 0; // count of consecutive non-green neighbors
      let mut d: usize = 0; // Direction
      while d < 10 {
        // g=1 if center is green, 0 otherwise → selects which PATT row to use
        let g: usize = if cfa.color_at(row as usize, col as usize) == CFA_COLOR_G { 1 } else { 0 };

        // Probe neighbor in current cardinal direction using ORTH rotation
        if cfa.color_at((row + ORTH[d]) as usize, (col + ORTH[d + 2]) as usize) == CFA_COLOR_G {
          ng = 0; // green neighbor resets the counter
        } else {
          ng += 1; // non-green neighbor increments
        }

        // 4 consecutive non-green neighbors → this is the solitary green pixel
        if ng == 4 {
          lut.sgrow = row as usize;
          lut.sgcol = col as usize;
        }

        // When ng reaches the trigger count (1 for non-green, 2 for green center),
        // we've found the correct orientation → rotate PATT offsets by ORTH[d..d+3]
        if ng == g as i32 + 1 {
          for c in 0..8 {
            // Apply 2x2 rotation matrix [ORTH[d], ORTH[d+1]; ORTH[d+2], ORTH[d+3]]
            // to canonical pattern coordinate (PATT[g][2c], PATT[g][2c+1])
            let v = ORTH[d] * PATT[g][c * 2] + ORTH[d + 1] * PATT[g][c * 2 + 1];
            let h = ORTH[d + 2] * PATT[g][c * 2] + ORTH[d + 3] * PATT[g][c * 2 + 1];
            // Slot permutation ensures direction buffer alignment across rotations
            let slot = c ^ (g * 2 & d);
            lut.hex[row as usize][col as usize][0][slot] = (v, h);
            lut.hex[row as usize][col as usize][1][slot] = (v, h);
          }
        }
        d += 2;
      }
    }
  }

  lut
}

/// Compute per-pixel green value bounds used to clamp interpolated green estimates.
///
/// For each pixel in the image, this function computes a `[lo, hi]` range that
/// constrains the interpolated green value, preventing overshoots and ringing
/// artifacts at high-contrast edges.
///
/// # Green pixels (CFA color == 1)
///
/// The green value is already known from the sensor, so bounds are trivially
/// set to `[value, value]` (the interpolated green equals the measured green).
///
/// # Non-green pixels (R or B)
///
/// The bounds are derived from the 6 nearest green neighbors in the hex grid
/// (slots [0]-[5] from `allhex[row%3][col%3][0]`, see [`build_hex_lut`]).
/// The minimum and maximum green values among these neighbors define the
/// allowed range for the interpolated green at this position.
///
/// ```text
///    Example: non-green pixel X with its 6 hex green neighbors (G):
///
///                  G(0.45)
///                    \
///         G(0.42) —— X —— G(0.50)       bounds = [0.38, 0.50]
///                   / \                  lo = min(all G) = 0.38
///        G(0.38)       G(0.48)           hi = max(all G) = 0.50
///                  |
///              G(0.44)
/// ```
///
/// Any interpolated green value for X is clamped to `[0.38, 0.50]`, ensuring
/// it stays within the range of its observed green neighbors. This prevents
/// color fringing and overshoots, particularly at sharp edges where the
/// interpolation formulas might extrapolate beyond physically plausible values.
///
/// These bounds are used by:
/// - [`green_interpolation`]: clamping the 4 directional green estimates
/// - [`green_recalculation`]: clamping refined green in multi-pass mode
#[multiversion(targets("x86_64+avx512f+avx512bw+avx512vl", "x86_64+avx+avx2+fma", "x86_64+sse4.1", "aarch64+neon"))]
fn compute_green_bounds(input: &[f32], width: usize, height: usize, cfa: &CFA, allhex: &HexLut) -> Vec<[f32; 2]> {
  let npix = width * height;
  let mut bounds = vec![[0.0f32; 2]; npix];
  for y in 2..height - 2 {
    for x in 2..width - 2 {
      let idx = y * width + x;
      if cfa.color_at(y, x) == CFA_COLOR_G {
        bounds[idx] = [input[idx], input[idx]];
        continue;
      }

      let hex = &allhex.hex[y % 3][x % 3][0];
      let mut lo = f32::MAX;
      let mut hi = f32::MIN;

      // Use first 6 hex neighbors (matching reference)
      for i in 0..6 {
        let (dy, dx) = hex[i];
        if dy == 0 && dx == 0 {
          continue;
        }
        let ny = (y as i32 + dy) as usize;
        let nx = (x as i32 + dx) as usize;
        if ny < height && nx < width {
          let v = input[ny * width + nx];
          lo = lo.min(v);
          hi = hi.max(v);
        }
      }

      bounds[idx] = [lo, hi];
    }
  }
  bounds
}

fn border_interpolate(input: &[f32], width: usize, height: usize, cfa: &CFA, output: &mut Color2D<f32, 3>, border: usize) {
  let hb = height.min(border);
  let wb = width.min(border);

  // Top and bottom strips
  for y in (0..hb).chain(height.saturating_sub(border)..height) {
    for x in 0..width {
      interpolate_border_pixel(input, width, height, cfa, output, y, x);
    }
  }
  // Left and right strips (excluding corners already done)
  for y in hb..height.saturating_sub(border) {
    for x in (0..wb).chain(width.saturating_sub(border)..width) {
      interpolate_border_pixel(input, width, height, cfa, output, y, x);
    }
  }
}

#[inline]
#[multiversion(targets("x86_64+avx512f+avx512bw+avx512vl", "x86_64+avx+avx2+fma", "x86_64+sse4.1", "aarch64+neon"))]
fn interpolate_border_pixel(input: &[f32], width: usize, height: usize, cfa: &CFA, output: &mut Color2D<f32, 3>, y: usize, x: usize) {
  let mut rgb = [0.0f32; 3];
  let mut count = [0u32; 3];

  let y_lo = y.saturating_sub(1);
  let y_hi = (y + 1).min(height - 1);
  let x_lo = x.saturating_sub(1);
  let x_hi = (x + 1).min(width - 1);

  for ny in y_lo..=y_hi {
    for nx in x_lo..=x_hi {
      let ch = cfa.color_at(ny, nx);
      rgb[ch] += input[ny * width + nx];
      count[ch] += 1;
    }
  }

  *output.at_mut(y, x) = [
    if count[0] > 0 { rgb[0] / count[0] as f32 } else { 0.0 },
    if count[1] > 0 { rgb[1] / count[1] as f32 } else { 0.0 },
    if count[2] > 0 { rgb[2] / count[2] as f32 } else { 0.0 },
  ];
}

#[allow(clippy::too_many_arguments)]
#[multiversion(targets("x86_64+avx512f+avx512bw+avx512vl", "x86_64+avx+avx2+fma", "x86_64+sse4.1", "aarch64+neon"))]
fn process_tile(
  input: &[f32],
  width: usize,
  height: usize,
  cfa: &CFA,
  allhex: &HexLut,
  green_bounds: &[[f32; 2]],
  top: usize,
  left: usize,
  tile_h: usize,
  tile_w: usize,
  passes: usize,
  ndir: usize,
  pad_lab: usize,
  pad_drv: usize,
  pad_homo: usize,
  pad_tile: usize,
  output: &SharedColor2D<f32, 3>,
) {
  let tpix = tile_h * tile_w;
  let mut rgb = vec![[0.0f32; 3]; ndir * tpix];
  let mut lab = vec![[0.0f32; 3]; tpix];
  let mut drv = vec![0.0f32; ndir * tpix];
  let mut homo = vec![0u8; ndir * tpix];

  // Step 1: Green interpolation in 4 directions (dirs 0-3)
  green_interpolation(input, width, height, cfa, allhex, green_bounds, top, left, tile_h, tile_w, &mut rgb);

  // Step 2: Multi-pass R/B interpolation
  for pass in 0..passes {
    let dir_base = if pass == 0 { 0 } else { 4 };

    if pass == 1 {
      // Copy dirs 0-3 to dirs 4-7
      for d in 0..4 {
        for i in 0..tpix {
          rgb[(4 + d) * tpix + i] = rgb[d * tpix + i];
        }
      }
    }

    // Recalculate green for passes > 0 using interpolated color differences
    if pass > 0 {
      green_recalculation(width, cfa, allhex, green_bounds, top, left, tile_h, tile_w, dir_base, &mut rgb);
    }

    // R/B interpolation (3 sub-steps) for dirs dir_base..dir_base+3
    rb_interpolation(input, width, height, cfa, allhex, top, left, tile_h, tile_w, dir_base, passes, &mut rgb);
  }

  // Step 3: Directional gradient computation in CIELab space
  //
  // For each direction buffer, compute a Laplacian-based gradient magnitude
  // in CIELab space. This measures how smoothly the interpolated colors
  // vary along the direction's axis — lower values indicate better
  // interpolation quality, used later in homogeneity voting.
  //
  // The 4 direction offsets correspond to:
  //   d=0: horizontal     (+1 pixel)
  //   d=1: vertical       (+tile_w pixels)
  //   d=2: diagonal NW-SE (+tile_w+1 pixels)
  //   d=3: diagonal NE-SW (+tile_w-1 pixels)
  let dir_offsets: [i32; 4] = [1, tile_w as i32, tile_w as i32 + 1, tile_w as i32 - 1];

  for d in 0..ndir {
    let base = d * tpix;

    // Convert this direction's RGB tile to CIELab
    for ty in pad_lab..tile_h.saturating_sub(pad_lab) {
      for tx in pad_lab..tile_w.saturating_sub(pad_lab) {
        let ti = ty * tile_w + tx;
        lab[ti] = rgb_to_lab(&rgb[base + ti]);
      }
    }

    // Compute directional Laplacian with CIELab cross-channel coupling.
    //
    // The discrete Laplacian along direction f for each Lab channel:
    //   ∇²L = 2·L_center - L_plus - L_minus
    //   ∇²a = 2·a_center - a_plus - a_minus
    //   ∇²b = 2·b_center - b_plus - b_minus
    //
    // In CIELab, the a* and b* channels share a dependency on luminance Y
    // through the definition:
    //   L* = 116 · f(Y/Yn) - 16     → L* depends on Y
    //   a* = 500 · [f(X/Xn) - f(Y/Yn)]  → a* depends on both X and Y
    //   b* = 200 · [f(Y/Yn) - f(Z/Zn)]  → b* depends on both Y and Z
    //
    // A pure luminance edge (only Y changes, X and Z constant) produces
    // Laplacians in a* and b* even though the chrominance hasn't changed.
    // To isolate the true chrominance variation, the L* Laplacian (g) is
    // used to subtract out the luminance-coupled component:
    //
    //   a*_corrected = ∇²a - g · (500/232)
    //   b*_corrected = ∇²b + g · (500/580)
    //
    // where the coupling constants derive from the CIELab coefficients:
    //   500/232 = 500 / (2·116)   — ratio of a*'s Y-sensitivity to L*'s
    //   500/580 = 200 / (2·116)   — ratio of b*'s Y-sensitivity to L*'s
    //                               (equivalently: 200/232 = 500/580)
    //
    // The final gradient magnitude is: g² + a_corrected² + b_corrected²
    let f = dir_offsets[d & 3];
    for ty in pad_drv..tile_h.saturating_sub(pad_drv) {
      for tx in pad_drv..tile_w.saturating_sub(pad_drv) {
        let ti = ty * tile_w + tx;
        let lix = lab[ti];
        let plus = lab[(ti as i32 + f) as usize];
        let minus = lab[(ti as i32 - f) as usize];

        let g = 2.0 * lix[0] - plus[0] - minus[0];
        let a_diff = 2.0 * lix[1] - plus[1] - minus[1] + g * (500.0 / 232.0);
        let b_diff = 2.0 * lix[2] - plus[2] - minus[2] - g * (500.0 / 580.0);

        drv[base + ti] = g * g + a_diff * a_diff + b_diff * b_diff;
      }
    }
  }

  // Build homogeneity maps
  for ty in pad_homo..tile_h.saturating_sub(pad_homo) {
    for tx in pad_homo..tile_w.saturating_sub(pad_homo) {
      let ti = ty * tile_w + tx;

      // Homogeneity threshold: 8x the minimum gradient across all directions.
      // Directions with gradient <= threshold are considered "smooth enough"
      // and receive a vote. The factor of 8 controls tolerance — a direction
      // is acceptable if its gradient is within one order of magnitude of the
      // best direction at this pixel.
      let mut tr = f32::MAX;
      for d in 0..ndir {
        let val = drv[d * tpix + ti];
        if tr > val {
          tr = val;
        }
      }
      tr *= 8.0;

      // Count votes in a 3x3 neighborhood: for each direction, how many
      // of the 9 pixels (center + 8 neighbors) have gradient <= threshold.
      // Higher vote count → more spatially consistent smoothness.
      for d in 0..ndir {
        let base = d * tpix;
        let mut votes = 0u8;
        for v in -1i32..=1 {
          for h in -1i32..=1 {
            let ni = (ti as i32 + v * tile_w as i32 + h) as usize;
            if drv[base + ni] <= tr {
              votes += 1;
            }
          }
        }
        homo[base + ti] = votes;
      }
    }
  }

  // Final averaging with 5x5 homogeneity summation
  for ty in pad_tile..tile_h.saturating_sub(pad_tile) {
    let iy = top + ty;
    if iy >= height {
      break;
    }

    for tx in pad_tile..tile_w.saturating_sub(pad_tile) {
      let ix = left + tx;
      if ix >= width {
        break;
      }
      let ti = ty * tile_w + tx;

      // Sum homogeneity over 5x5 window for each direction
      let mut hm = [0u16; 8];
      for d in 0..ndir {
        let base = d * tpix;
        for v in -2i32..=2 {
          for h in -2i32..=2 {
            let ni = (ti as i32 + v * tile_w as i32 + h) as usize;
            hm[d] += homo[base + ni] as u16;
          }
        }
      }

      // Cross-direction competition for ndir=8 (matching reference)
      for d in 0..ndir.saturating_sub(4) {
        if hm[d] < hm[d + 4] {
          hm[d] = 0;
        } else if hm[d] > hm[d + 4] {
          hm[d + 4] = 0;
        }
      }

      let mut max_hm = hm[0];
      for d in 1..ndir {
        if max_hm < hm[d] {
          max_hm = hm[d];
        }
      }
      let threshold = max_hm - (max_hm >> 3);

      let mut sum = [0.0f32; 3];
      let mut cnt = 0u32;
      for d in 0..ndir {
        if hm[d] >= threshold {
          let v = rgb[d * tpix + ti];
          sum[0] += v[0];
          sum[1] += v[1];
          sum[2] += v[2];
          cnt += 1;
        }
      }

      if cnt > 0 {
        let inv = 1.0 / cnt as f32;
        // SAFETY: Each tile writes to a unique region of the output; no concurrent writes to same index.
        unsafe {
          *output.inner_mut().at_mut(iy, ix) = [
            sum[0] * inv, // Keep
            sum[1] * inv, // Keep
            sum[2] * inv, // Keep
          ];
        }
      }
    }
  }
}

#[multiversion(targets("x86_64+avx512f+avx512bw+avx512vl", "x86_64+avx+avx2+fma", "x86_64+sse4.1", "aarch64+neon"))]
fn green_interpolation(
  input: &[f32],
  width: usize,
  height: usize,
  cfa: &CFA,
  allhex: &HexLut,
  green_bounds: &[[f32; 2]],
  top: usize,
  left: usize,
  tile_h: usize,
  tile_w: usize,
  rgb: &mut [[f32; 3]],
) {
  let tpix = tile_h * tile_w;
  let sgrow = allhex.sgrow as i32;

  // Helper: read pixel from input at (row, col)
  let pix = |row: i32, col: i32| -> f32 { input[row as usize * width + col as usize] };

  for ty in 0..tile_h {
    let iy = top + ty;
    if iy >= height {
      break;
    }

    for tx in 0..tile_w {
      let ix = left + tx;
      if ix >= width {
        break;
      }

      let ti = ty * tile_w + tx;
      let img_idx = iy * width + ix;
      let f = cfa.color_at(iy, ix);

      // Set known color channel for all directions
      let v = input[img_idx];
      for d in 0..4 {
        rgb[d * tpix + ti][f as usize] = v;
      }

      if f == 1 {
        // Green pixel: green is known
        for d in 0..4 {
          rgb[d * tpix + ti][1] = v;
        }
        continue;
      }

      if iy >= 3 && iy + 3 < height && ix >= 3 && ix + 3 < width {
        let hex = &allhex.hex[iy % 3][ix % 3][0];
        let [lo, hi] = green_bounds[img_idx];
        let row = iy as i32;
        let col = ix as i32;

        // Direction 0: symmetric bilinear with 2nd-order correction
        // color[1][0] = 174*(hex1_g + hex0_g) - 46*(2*hex1_g + 2*hex0_g)
        let (h0v, h0h) = hex[0];
        let (h1v, h1h) = hex[1];
        let c0 = (174.0 * (pix(row + h1v, col + h1h) + pix(row + h0v, col + h0h)) // Keep
                - 46.0 * (pix(row + 2 * h1v, col + 2 * h1h) + pix(row + 2 * h0v, col + 2 * h0h))) // Keep
                * (1.0 / 256.0); // Keep

        // Direction 1: asymmetric + own-channel gradient correction
        // color[1][1] = 223*hex3_g + 33*hex2_g + 92*(own_ch - own_ch_at_-hex2)
        let (h2v, h2h) = hex[2];
        let (h3v, h3h) = hex[3];
        let c1 = (223.0 * pix(row + h3v, col + h3h) // Keep
                + 33.0 * pix(row + h2v, col + h2h) // Keep
                + 92.0 * (v - pix(row - h2v, col - h2h))) // Keep
                * (1.0 / 256.0); // Keep

        // Directions 2,3: diagonal with 2nd derivative of own channel
        // color[1][2+c] = 164*hex4/5_g + 92*(-2*hex4/5)_g + 33*(2*own - own_at_3*hex - own_at_-3*hex)
        let (h4v, h4h) = hex[4];
        let c2 = (164.0 * pix(row + h4v, col + h4h) // Keep
                + 92.0 * pix(row - 2 * h4v, col - 2 * h4h) // Keep
                + 33.0 * (2.0 * v - pix(row + 3 * h4v, col + 3 * h4h) // Keep
                                   - pix(row - 3 * h4v, col - 3 * h4h))) // Keep
                * (1.0 / 256.0); // Keep

        let (h5v, h5h) = hex[5];
        let c3 = (164.0 * pix(row + h5v, col + h5h) // Keep
                + 92.0 * pix(row - 2 * h5v, col - 2 * h5h) // Keep
                + 33.0 * (2.0 * v - pix(row + 3 * h5v, col + 3 * h5h) // Keep
                                   - pix(row - 3 * h5v, col - 3 * h5h))) // Keep
                * (1.0 / 256.0); // Keep

        // Map formula index to direction buffer via parity XOR (matching reference)
        // c ^ !((row - sgrow) % 3): swaps pairs (0,1) and (2,3) every third row
        let xor_val = if (row - sgrow).rem_euclid(3) == 0 { 1usize } else { 0usize };
        let color = [c0, c1, c2, c3];
        for c in 0..4usize {
          let d = c ^ xor_val;
          rgb[d * tpix + ti][1] = color[c].max(lo).min(hi);
        }
      } else {
        // Near border: use raw value as green estimate
        for d in 0..4 {
          rgb[d * tpix + ti][1] = v;
        }
      }
    }
  }
}

/// Green recalculation for passes > 0 of Markesteijn 3-pass.
/// Uses interpolated R/B values to refine green estimates via hex neighbors.
#[allow(clippy::too_many_arguments)]
#[multiversion(targets("x86_64+avx512f+avx512bw+avx512vl", "x86_64+avx+avx2+fma", "x86_64+sse4.1", "aarch64+neon"))]
fn green_recalculation(
  width: usize,
  cfa: &CFA,
  allhex: &HexLut,
  green_bounds: &[[f32; 2]],
  top: usize,
  left: usize,
  tile_h: usize,
  tile_w: usize,
  dir_base: usize,
  rgb: &mut [[f32; 3]],
) {
  let tpix = tile_h * tile_w;
  let tw = tile_w as i32;
  let sgrow = allhex.sgrow as i32;
  const PAD_G_RECALC: usize = 6;

  for ty in PAD_G_RECALC..tile_h.saturating_sub(PAD_G_RECALC) {
    let iy = top + ty;
    let row = iy as i32;
    for tx in PAD_G_RECALC..tile_w.saturating_sub(PAD_G_RECALC) {
      let ix = left + tx;
      let f = cfa.color_at(iy, ix);
      if f == CFA_COLOR_G {
        continue; // green pixel, skip
      }
      let f = f as usize;
      let ti = (ty * tile_w + tx) as i32;
      let img_idx = iy * width + ix;
      let [lo, hi] = green_bounds[img_idx];

      let hex = &allhex.hex[iy % 3][ix % 3][1]; // second set of hex offsets
      let xor_val: usize = if (row - sgrow).rem_euclid(3) == 0 { 1 } else { 0 };

      for d in 3..6usize {
        let dir_idx = (d - 2) ^ xor_val;
        let base = (dir_base + dir_idx) * tpix;

        // Convert hex[d] (dy, dx) to tile offset
        let (dy, dx) = hex[d];
        let hex_off = dy * tw + dx;

        let rix_center = rgb[(base as i32 + ti) as usize];
        let rix_plus = rgb[(base as i32 + ti + hex_off) as usize];
        let rix_minus2 = rgb[(base as i32 + ti - 2 * hex_off) as usize];

        // val = rix[-2*hex[d]][1] + 2*rix[hex[d]][1]
        //     - rix[-2*hex[d]][f] - 2*rix[hex[d]][f] + 3*rix[0][f]
        let val = rix_minus2[1] + 2.0 * rix_plus[1] // Keep
                - rix_minus2[f] - 2.0 * rix_plus[f] // Keep
                + 3.0 * rix_center[f]; // Keep

        rgb[(base as i32 + ti) as usize][1] = (val / 3.0).max(lo).min(hi);
      }
    }
  }
}

#[multiversion(targets("x86_64+avx512f+avx512bw+avx512vl", "x86_64+avx+avx2+fma", "x86_64+sse4.1", "aarch64+neon"))]
fn rb_interpolation(
  _input: &[f32],
  _width: usize,
  _height: usize,
  cfa: &CFA,
  allhex: &HexLut,
  top: usize,
  left: usize,
  tile_h: usize,
  tile_w: usize,
  dir_base: usize,
  passes: usize,
  rgb: &mut [[f32; 3]],
) {
  let tpix = tile_h * tile_w;
  let tw = tile_w as i32;
  let sgrow = allhex.sgrow as i32;
  let sgcol = allhex.sgcol as i32;

  // Padding values matching darktable/dcraw reference
  let pad_rb_g: usize = if passes == 1 { 6 } else { 5 };
  let pad_rb_br: usize = if passes == 1 { 6 } else { 5 };
  let pad_g2x2: usize = if passes == 1 { 8 } else { 4 };

  // Step 3a: Interpolate R/B at solitary green pixels
  // These are green pixels on the sparse grid at (row-sgrow)%3==0 && (col-sgcol)%3==0
  {
    // First row/col >= pad aligned to sgrow/sgcol mod 3 grid
    let row_start = ((top as i32 - sgrow + pad_rb_g as i32 + 2) / 3 * 3 + sgrow) as usize - top;
    let col_start = ((left as i32 - sgcol + pad_rb_g as i32 + 2) / 3 * 3 + sgcol) as usize - left;

    let mut ty = row_start;
    while ty + pad_rb_g < tile_h {
      let iy = (top + ty) as i32;
      let mut tx = col_start;
      while tx + pad_rb_g < tile_w {
        let ix = (left + tx) as i32;
        let ti = (ty * tile_w + tx) as i32;

        if cfa.color_at(iy as usize, ix as usize) != CFA_COLOR_G {
          tx += 3;
          continue;
        }

        // h = channel of pixel to the right (0=R or 2=B)
        let mut h = cfa.color_at(iy as usize, (ix + 1) as usize) as usize;
        if h == CFA_COLOR_G {
          tx += 3;
          continue;
        }

        let mut color_est = [[0.0f32; 6]; 3]; // color_est[channel][d]
        let mut diff = [0.0f32; 6];
        let mut i: i32 = 1; // horizontal step in tile (alternates with tile_w)
        let mut buf_idx = dir_base; // which direction buffer we're reading/writing

        for d in 0..6usize {
          for c in 0..2usize {
            let step = i << c; // i*1 or i*2
            let base = buf_idx * tpix;
            let center_g = rgb[(base as i32 + ti) as usize][1];
            let plus_g = rgb[(base as i32 + ti + step) as usize][1];
            let minus_g = rgb[(base as i32 + ti - step) as usize][1];
            let plus_h = rgb[(base as i32 + ti + step) as usize][h];
            let minus_h = rgb[(base as i32 + ti - step) as usize][h];

            let g = 2.0 * center_g - plus_g - minus_g;
            color_est[h][d] = g + plus_h + minus_h;

            if d > 1 {
              diff[d] += (plus_g - minus_g - plus_h + minus_h).powi(2) + g * g;
            }
            h ^= 2; // toggle between R(0) and B(2)
          }
          // Pick lower variance between competing pairs
          if d > 1 && (d & 1) != 0 {
            if diff[d - 1] < diff[d] {
              for c in 0..2usize {
                color_est[c * 2][d] = color_est[c * 2][d - 1];
              }
            }
          }
          // Write result to tile buffer
          if d < 2 || (d & 1) != 0 {
            let base = buf_idx * tpix;
            for c in 0..2usize {
              rgb[(base as i32 + ti) as usize][c * 2] = (color_est[c * 2][d] * 0.5).max(0.0);
            }
            buf_idx += 1;
          }
          i ^= tw ^ 1; // toggle between 1 (horizontal) and tile_w (vertical)
          h ^= 2;
        }

        tx += 3;
      }
      ty += 3;
    }
  }

  // Step 3b: Interpolate red for blue pixels and vice versa
  for ty in pad_rb_br..tile_h.saturating_sub(pad_rb_br) {
    let iy = (top + ty) as i32;
    for tx in pad_rb_br..tile_w.saturating_sub(pad_rb_br) {
      let ix = (left + tx) as i32;
      let ti = ty * tile_w + tx;

      // f = the channel we need to interpolate: 2-fcol gives R(0) for B pixels, B(2) for R pixels
      let fc = cfa.color_at(iy as usize, ix as usize);
      let f = match fc {
        CFA_COLOR_R => 2usize, // R pixel: need B
        CFA_COLOR_B => 0usize, // B pixel: need R
        _ => continue,         // green pixel: skip
      };

      // Choose interpolation direction based on green gradient
      let c: i32 = if ((iy - sgrow).rem_euclid(3)) != 0 { tw } else { 1 };
      let h: i32 = 3 * (c ^ tw ^ 1);

      for d in 0..4 {
        let base = (dir_base + d) * tpix;
        let rix = |off: i32| -> &[f32; 3] { &rgb[(base as i32 + ti as i32 + off) as usize] };

        let dir = if d > 1 || ((d as i32 ^ c) & 1) != 0 // Keep
          || ((rix(0)[1] - rix(c)[1]).abs() + (rix(0)[1] - rix(-c)[1]).abs()) // Keep
            < 2.0 * ((rix(0)[1] - rix(h)[1]).abs() + (rix(0)[1] - rix(-h)[1]).abs())
        // Keep
        {
          c
        } else {
          h
        };

        let val = (rix(dir)[f] + rix(-dir)[f] // Keep
          + 2.0 * rix(0)[1]
          - rix(dir)[1]
          - rix(-dir)[1])
          * 0.5; // Keep
        rgb[base + ti][f] = val.max(0.0);
      }
    }
  }

  // Step 3c: Fill in red and blue for 2x2 blocks of green
  for ty in pad_g2x2..tile_h.saturating_sub(pad_g2x2) {
    let iy = (top + ty) as i32;
    if ((iy - sgrow).rem_euclid(3)) == 0 {
      continue;
    }
    for tx in pad_g2x2..tile_w.saturating_sub(pad_g2x2) {
      let ix = (left + tx) as i32;
      if ((ix - sgcol).rem_euclid(3)) == 0 {
        continue;
      }
      let ti = (ty * tile_w + tx) as i32;
      let hex = &allhex.hex[iy as usize % 3][ix as usize % 3][1];

      for dd in 0..4usize {
        let hd = dd * 2;
        let base = ((dir_base + dd) * tpix) as i32;
        let (h0v, h0h) = hex[hd];
        let (h1v, h1h) = hex[hd + 1];
        let off0 = h0v * tw + h0h;
        let off1 = h1v * tw + h1h;

        if off0 + off1 != 0 {
          // Asymmetric: weights 2:1
          let g = 3.0 * rgb[(base + ti) as usize][1] // Keep
                - 2.0 * rgb[(base + ti + off0) as usize][1] // Keep
                - rgb[(base + ti + off1) as usize][1]; // Keep
          for ch in (0..3).step_by(2) {
            rgb[(base + ti) as usize][ch] = ((g
              + 2.0 * rgb[(base + ti + off0) as usize][ch] // Keep
              + rgb[(base + ti + off1) as usize][ch])
              / 3.0)
              .max(0.0); // Keep
          }
        } else {
          // Symmetric: equal weights
          let g = 2.0 * rgb[(base + ti) as usize][1] // Keep
                - rgb[(base + ti + off0) as usize][1] // Keep
                - rgb[(base + ti + off1) as usize][1]; // Keep
          for ch in (0..3).step_by(2) {
            rgb[(base + ti) as usize][ch] = ((g // Keep
              + rgb[(base + ti + off0) as usize][ch] // Keep
              + rgb[(base + ti + off1) as usize][ch])
              * 0.5)
              .max(0.0); // Keep
          }
        }
      }
    }
  }
}

/// Convert linear RGB to CIELab.
///
/// Uses a fixed sRGB/D65 matrix for the RGB→XYZ step. This is used by
/// Markesteijn for homogeneity comparison — exact colorimetric accuracy
/// is not required, only consistent relative distances.
#[inline(always)]
pub fn rgb_to_lab(rgb: &[f32; 3]) -> [f32; 3] {
  let xyz = multiply_row1(&SRGB_TO_XYZ_D65, &rgb);
  XYZ_to_lab(&xyz, &CIE_1931_TRISTIMULUS_D65)
}
