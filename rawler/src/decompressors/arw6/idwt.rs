// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Nikolay Amiantov <ab@fmap.me>

//! The inverse LeGall 5/3 wavelet (IDWT).

use crate::Result;

use super::Plane;

/// Whole-sample edge: clamp a lattice index to the valid range [0, n).
#[inline(always)]
fn clamp(i: isize, n: usize) -> usize {
  i.clamp(0, n as isize - 1) as usize
}

/// One inverse 5/3 lift along rows: interleave the coarse (`low`) and detail
/// (`high`) sub-bands back onto a single grid, row by row. `flip` selects if
/// the coarse sub-band lands on the EVEN output rows or on the ODD ones.
fn idwt53_rows(low: &Plane, high: &Plane, flip: bool) -> Result<Plane> {
  if low.w != high.w {
    return Err(format!("ARW6: IDWT sub-band widths differ: {} vs {}", low.w, high.w).into());
  }
  let w = low.w;
  let (n_low, n_high) = (low.h, high.h);
  let n_out = n_low + n_high;
  let mut out = Plane::new(w, n_out);
  let c = flip as usize; // coarse-sample output parity; detail takes 1 - c
  for n in 0..n_low {
    // undo update: coarse from its two flanking details
    let pos = 2 * n + c;
    if pos >= n_out {
      // a trailing sample past the grid edge
      continue;
    }
    let h0 = high.row(clamp(n as isize - 1 + c as isize, n_high));
    let h1 = high.row(clamp(n as isize + c as isize, n_high));
    let low_row = low.row(n);
    for i in 0..w {
      out.data[pos * w + i] = low_row[i] - ((h0[i] + h1[i] + 2) >> 2);
    }
  }
  for n in 0..n_high {
    // undo predict: detail from its two flanking coarses
    let pos = 2 * n + 1 - c;
    if pos >= n_out {
      continue;
    }
    let e0 = 2 * clamp(n as isize - c as isize, n_low) + c;
    let e1 = 2 * clamp(n as isize + 1 - c as isize, n_low) + c;
    let high_row = high.row(n);
    for i in 0..w {
      out.data[pos * w + i] = high_row[i] + ((out.data[e0 * w + i] + out.data[e1 * w + i]) >> 1);
    }
  }
  Ok(out)
}

/// One inverse 5/3 lift along columns, over the [0..h) x [0..w_pair) crop of
/// both sub-bands: `low` lands on the even output columns (the horizontal
/// passes never flip).
fn idwt53_cols(low: &Plane, high: &Plane, h: usize, w_pair: usize) -> Plane {
  // both sub-bands contribute w_pair columns
  let mut out = Plane::new(2 * w_pair, h);
  for r in 0..h {
    let low_row = low.row(r);
    let high_row = high.row(r);
    let out_row = out.row_mut(r);
    for n in 0..w_pair {
      // undo update: coarse from its two flanking details
      out_row[2 * n] = low_row[n] - ((high_row[clamp(n as isize - 1, w_pair)] + high_row[clamp(n as isize, w_pair)] + 2) >> 2);
    }
    for n in 0..w_pair {
      // undo predict: detail from its two flanking coarses
      let e0 = out_row[2 * clamp(n as isize, w_pair)];
      let e1 = out_row[2 * clamp(n as isize + 1, w_pair)];
      out_row[2 * n + 1] = high_row[n] + ((e0 + e1) >> 1);
    }
  }
  out
}

/// One inverse 2D 5/3 level: vertical pass with the given flip (columns
/// LL|LH and HL|HH; the flip is the parity of the reconstructed column's top
/// coordinate, see [`super::geometry::vflip`]), then a horizontal pass
/// (always the standard phase, left edge at column 0).
pub(super) fn idwt2d(ll: &Plane, lh: &Plane, hl: &Plane, hh: &Plane, flip: bool) -> Result<Plane> {
  let left = idwt53_rows(ll, lh, flip)?;
  let right = idwt53_rows(hl, hh, flip)?;
  let n_rows = left.h.min(right.h);
  let n_cols = left.w.min(right.w);
  Ok(idwt53_cols(&left, &right, n_rows, n_cols))
}

/// One inverse 5/3 level over a quincunx (diamond) lattice: interleave the
/// [0..h) x [0..w) crops of `lo` and `hi` into an h x 2w plane for
/// diamond-placed Bayer greens.
///
/// In the packed h x 2w plane one column of travel is a diagonal step on the
/// underlying grid. A sample's four nearest neighbours therefore lie in two
/// adjacent packed rows, which makes the lift a diamond over both rows rather
/// than two separable 1-D passes.
///
/// See also: <https://arxiv.org/html/2209.00932> ("Edge-Aware Extended
/// Star-Tetrix Transforms for CFA-Sampled Raw Camera Image Compression").
pub(super) fn diamond_idwt(lo: &Plane, hi: &Plane, h: usize, w: usize) -> Plane {
  // Low-pass with the update undone.
  let mut low = Plane::new(w, h);
  let mut out = Plane::new(2 * w, h);
  // Undo the update: subtract the four detail samples around each low-pass
  // one (this row and the one above it). Doubling before the final shift
  // rounds the half away from zero.
  for r in 0..h {
    let r0 = clamp(r as isize - 1, h);
    let hi_row = hi.row(r);
    let hi_row0 = hi.row(r0);
    let lo_row = lo.row(r);
    let low_row = low.row_mut(r);
    for n in 0..w {
      let n1 = clamp(n as isize + 1, w);
      low_row[n] = (2 * lo_row[n] - ((hi_row[n] + hi_row[n1] + hi_row0[n] + hi_row0[n1]) >> 2)) >> 1;
    }
  }
  // Undo the predict and interleave: the even columns are the four low-pass
  // samples around them (this row and the next) plus their own detail sample,
  // the odd columns are the low-pass itself.
  for r in 0..h {
    let low_next = low.row(clamp(r as isize + 1, h));
    let low_row = low.row(r);
    let hi_row = hi.row(r);
    let out_row = out.row_mut(r);
    for i in 0..w {
      let i0 = clamp(i as isize - 1, w);
      out_row[2 * i] = ((low_next[i0] + low_next[i] + low_row[i0] + low_row[i]) >> 2) + hi_row[i];
      out_row[2 * i + 1] = low_row[i];
    }
  }
  out
}
