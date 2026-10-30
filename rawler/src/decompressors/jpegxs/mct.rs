// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026

//! Output stage of the Nikon "High Efficiency" NEF decoder: inverse colour
//! transform, decompanding and CFA mosaic assembly.
//!
//! The four decoded planes are not the Bayer channels. They are a star-tetrix
//! (TETRIX) decorrelation of them, `(Y, Cr, Δ, Cb)`, even though the header
//! says `Cpih = 0`. The `CTS` and `CRG` markers that would carry the
//! transform parameters are missing, so the parameters are fixed here to
//! `Cf = 3`, `e1 = e2 = 1` and CFA pattern type 0 (RGGB).
//!
//! Nikon needs three things on top of the standard inverse:
//!
//! * the chroma planes come out low by a small constant factor, so they are
//!   scaled up first: Cb by 4339/4096, Cr by 4178/4096;
//! * the planes are stored as `(Y, Cr, Δ, Cb)` while the standard inverse
//!   wants `(Y, Cb, Cr, Δ)`, so they are permuted `0312`;
//! * the coded samples are square-root companded rather than sensor values,
//!   and are decompanded at the very end as `x = (v + V0)² / D + C`, with
//!   `V0 = 16158`, `D = 191488` and `C = 1050`, close to the 1008 black
//!   level, clamped to the component depth. That replaces the standard linear
//!   `(v + 2^(Bw-1) + 2^(Fq-1)) >> Fq` output scaling. `v` is the
//!   [`super::dwt`] output after the inverse colour transform, still in the
//!   zero-centred `Bw` domain.
//!
//! The chroma gains and the three companding constants are fitted against
//! lossless shots of a single scene rather than read from the stream, so they
//! are probably about right rather than exact. Whether they hold for other
//! cameras is untested.

use super::header::Header;
use crate::RawlerError;
use crate::Result;

/// Chroma gains in 1/4096 units, applied before the inverse star-tetrix.
const CB_GAIN: i64 = 4339; // 1.0594
const CR_GAIN: i64 = 4178; // 1.0200

/// Square-root companding constants. The encoder was fed
/// `sqrt((x - C) * D) - V0`, level-shifted to an ordinary 14-bit sample.
const NLT_V0: i64 = 16158;
const NLT_D: i64 = 191488;
const NLT_C: i64 = 1050;

/// Displacement of each spec-order component on the CFA grid (ISO/IEC 21122-1
/// Table F.10, CFA pattern type 0): `(Δx, Δy)` for `(Y, Cb, Cr, Δ)`.
const DISPLACEMENT: [(i32, i32); 4] = [(0, 1), (1, 1), (0, 0), (1, 0)];

/// Spec-order component index by CFA grid parity (Table F.11, pattern type
/// 0), indexed `[x % 2][y % 2]`.
const COMPONENT_AT: [[usize; 2]; 2] = [[2, 0], [3, 1]];

/// The CFA-grid neighbour read all four inverse steps share (Table F.12).
///
/// Component `c`'s sample `(x, y)` sits at grid position `(2x + Δx, 2y + Δy)`;
/// the step reads the grid sample offset `(rx, ry)` from there, reflecting at
/// the plane edges. With `Cf = 3` a vertical offset also reflects whenever it
/// would leave the component's own quad row (`ry + Δy` outside `0..=1`), which
/// keeps the vertical taps inside the two grid rows the quad spans. The
/// landing parity picks the plane, per [`COMPONENT_AT`].
#[inline]
fn access(comps: &[Vec<i32>; 4], c: usize, x: usize, y: usize, w: usize, h: usize, mut rx: i32, mut ry: i32) -> i32 {
  let (dx, dy) = DISPLACEMENT[c];
  if 2 * x as i32 + rx + dx < 0 || 2 * x as i32 + rx + dx >= 2 * w as i32 {
    rx = -rx;
  }
  if ry + dy < 0 || ry + dy > 1 || 2 * y as i32 + ry + dy < 0 || 2 * y as i32 + ry + dy >= 2 * h as i32 {
    ry = -ry;
  }
  let gx = 2 * x as i32 + rx + dx;
  let gy = 2 * y as i32 + ry + dy;
  let plane = COMPONENT_AT[(gx % 2) as usize][(gy % 2) as usize];
  comps[plane][(gy / 2) as usize * w + (gx / 2) as usize]
}

/// Sum of component `c`'s four diagonal grid neighbours.
#[inline]
fn diagonals(comps: &[Vec<i32>; 4], c: usize, x: usize, y: usize, w: usize, h: usize) -> i32 {
  access(comps, c, x, y, w, h, -1, -1) + access(comps, c, x, y, w, h, 1, -1) + access(comps, c, x, y, w, h, -1, 1) + access(comps, c, x, y, w, h, 1, 1)
}

/// Sum of component `c`'s four orthogonal grid neighbours.
#[inline]
fn cross(comps: &[Vec<i32>; 4], c: usize, x: usize, y: usize, w: usize, h: usize) -> i32 {
  access(comps, c, x, y, w, h, -1, 0) + access(comps, c, x, y, w, h, 1, 0) + access(comps, c, x, y, w, h, 0, -1) + access(comps, c, x, y, w, h, 0, 1)
}

/// Inverse average step (Table F.5): Y regains the Δ diagonal average.
fn inv_avg_step(comps: &mut [Vec<i32>; 4], w: usize, h: usize) {
  for y in 0..h {
    for x in 0..w {
      let s = diagonals(comps, 0, x, y, w, h);
      comps[0][y * w + x] -= s >> 3;
    }
  }
}

/// Inverse delta step (Table F.6): Δ regains the Y diagonal average.
fn inv_delta_step(comps: &mut [Vec<i32>; 4], w: usize, h: usize) {
  for y in 0..h {
    for x in 0..w {
      let s = diagonals(comps, 3, x, y, w, h);
      comps[3][y * w + x] += s >> 2;
    }
  }
}

/// Inverse Y step (Table F.7): both green positions shed their chroma
/// cross-average. The general step is
/// `(2^e2 (b_l + b_r) + 2^e1 (r_t + r_b)) >> 3`, which with `e1 = e2 = 1` is
/// exactly `sum >> 2`. A stream carrying real CTS markers would default to
/// `e1 = e2 = 0` instead. Neither green reads the other, so the order of the
/// two is free.
fn inv_y_step(comps: &mut [Vec<i32>; 4], w: usize, h: usize) {
  for c in [0, 3] {
    for y in 0..h {
      for x in 0..w {
        let s = cross(comps, c, x, y, w, h);
        comps[c][y * w + x] -= s >> 2;
      }
    }
  }
}

/// Inverse CbCr step (Table F.8): Cb and Cr regain the green cross-average.
fn inv_cbcr_step(comps: &mut [Vec<i32>; 4], w: usize, h: usize) {
  for c in [1, 2] {
    for y in 0..h {
      for x in 0..w {
        let s = cross(comps, c, x, y, w, h);
        comps[c][y * w + x] += s >> 2;
      }
    }
  }
}

/// Inverse star-tetrix transform (Table F.4) for `Cf = 3`, `e1 = e2 = 1`,
/// CFA pattern type 0. Consumes planes in spec order `(Y, Cb, Cr, Δ)` and
/// leaves them as the four Bayer quad planes `(R, G1, G2, B)`, quad positions
/// `(0,0), (0,1), (1,0), (1,1)` in `(row, column)` terms.
fn inverse_star_tetrix(comps: &mut [Vec<i32>; 4], w: usize, h: usize) {
  inv_avg_step(comps, w, h);
  inv_delta_step(comps, w, h);
  inv_y_step(comps, w, h);
  inv_cbcr_step(comps, w, h);
  // The transform leaves R at parity (0, 0) -> slot 2 and B at (1, 1) ->
  // slot 1 (see COMPONENT_AT); swap into R, G1, G2, B order.
  comps.swap(0, 2);
  comps.swap(1, 3);
}

/// Scale one chroma plane by `gain / 4096`, rounding to nearest.
fn scale_chroma(plane: &mut [i32], gain: i64) {
  for v in plane.iter_mut() {
    *v = ((*v as i64 * gain + (1 << 11)) >> 12) as i32;
  }
}

/// Undo the square-root companding of one `Bw`-domain sample, producing a
/// sensor value clamped to `depth` bits.
#[inline]
fn decompand(v: i32, max: i64) -> u16 {
  let t = (v as i64 + NLT_V0).max(0);
  let x = (t * t + NLT_D / 2) / NLT_D + NLT_C;
  x.clamp(0, max) as u16
}

/// Run the whole output stage on the planes [`super::dwt::Idwt::finish`]
/// returns, in the component order of the codestream: chroma gains, the
/// `0312` permutation into spec order, the inverse star-tetrix and the
/// square-root decompanding. Returns the four Bayer quad planes `(R, G1, G2,
/// B)` as sensor values; [`interleave`] assembles them into the mosaic.
pub fn transform(planes: Vec<Vec<i32>>, header: &Header) -> Result<Vec<Vec<u16>>> {
  let (w, h) = (header.plane_width(), header.plane_height());
  let planes: [Vec<i32>; 4] = planes
    .try_into()
    .map_err(|p: Vec<Vec<i32>>| RawlerError::DecoderFailed(format!("JPEG XS: the star-tetrix inverse needs 4 planes, got {}", p.len())))?;

  let depths: Vec<u8> = header.components.iter().map(|c| c.bit_depth).collect();
  let depth = depths.first().copied().unwrap_or(0);
  if depths.len() != 4 || depths.iter().any(|&d| d != depth) || depth == 0 || depth > 15 {
    return Err(RawlerError::DecoderFailed(format!(
      "JPEG XS: the star-tetrix inverse needs 4 components of one depth in 1..=15, got {:?}",
      depths
    )));
  }
  if w == 0 || h == 0 || planes.iter().any(|p| p.len() != w * h) {
    return Err(RawlerError::DecoderFailed(format!(
      "JPEG XS: planes do not match the {}x{} component grid",
      w, h
    )));
  }

  // Nikon order (Y, Cr, Δ, Cb) -> spec order (Y, Cb, Cr, Δ).
  let [y, cr, delta, cb] = planes;
  let mut comps = [y, cb, cr, delta];
  scale_chroma(&mut comps[1], CB_GAIN);
  scale_chroma(&mut comps[2], CR_GAIN);
  inverse_star_tetrix(&mut comps, w, h);

  let max = (1i64 << depth) - 1;
  Ok(comps.into_iter().map(|plane| plane.into_iter().map(|v| decompand(v, max)).collect()).collect())
}

/// Interleave the four Bayer quad planes, each `w` x `h`, into the `2w` x
/// `2h` RGGB mosaic: plane 0 fills quad position (0,0), plane 1 (0,1),
/// plane 2 (1,0) and plane 3 (1,1), positions in `(row, column)` terms.
pub fn interleave(planes: &[Vec<u16>], w: usize, h: usize) -> Result<Vec<u16>> {
  if planes.len() != 4 || planes.iter().any(|p| p.len() != w * h) {
    return Err(RawlerError::DecoderFailed(format!(
      "JPEG XS: mosaic assembly wants 4 planes of {} samples, got {:?}",
      w * h,
      planes.iter().map(|p| p.len()).collect::<Vec<_>>()
    )));
  }
  let mut mosaic = vec![0u16; 4 * w * h];
  for y in 0..h {
    let (top, bottom) = (2 * y * 2 * w, (2 * y + 1) * 2 * w);
    for x in 0..w {
      mosaic[top + 2 * x] = planes[0][y * w + x];
      mosaic[top + 2 * x + 1] = planes[1][y * w + x];
      mosaic[bottom + 2 * x] = planes[2][y * w + x];
      mosaic[bottom + 2 * x + 1] = planes[3][y * w + x];
    }
  }
  Ok(mosaic)
}

#[cfg(test)]
mod tests {
  use super::super::header::parse;
  use super::super::header::testdata::SAMPLE;
  use super::super::reference;
  use super::*;

  /// A header for synthetic four-component tests of the given plane size.
  fn nikon_header(w: u16, h: u16) -> Header {
    let mut header = parse(&SAMPLE).expect("header parses");
    header.grid_width = 2 * w;
    header.grid_height = 2 * h;
    header
  }

  #[test]
  fn decompands_known_values() {
    // v = 0 sits near mid-grey: (16158² + 95744) / 191488 + 1050.
    assert_eq!(decompand(0, 16383), 2413);
    // The zero level of the compander: t = 0 collapses to the C offset.
    assert_eq!(decompand(-16158, 16383), 1050);
    // Anything below is clamped before squaring, not folded back up.
    assert_eq!(decompand(-100_000, 16383), 1050);
    // Large values clamp to the component depth.
    assert_eq!(decompand(100_000, 16383), 16383);
    // The largest value that still decompands below the clamp.
    assert_eq!(decompand(38_026, 16383), 16382);
    assert_eq!(decompand(38_027, 16383), 16383);
  }

  #[test]
  fn scales_chroma_with_symmetric_rounding_bias() {
    let mut plane = [4096, -4096, 1, -1, 0, 2048];
    scale_chroma(&mut plane, CB_GAIN);
    // (v * 4339 + 2048) >> 12, an arithmetic shift: rounds to nearest,
    // half away from zero for positives, half towards zero for negatives.
    assert_eq!(plane, [4339, -4339, 1, -1, 0, 2170]);
    let mut plane = [4096, -4096];
    scale_chroma(&mut plane, CR_GAIN);
    assert_eq!(plane, [4178, -4178]);
  }

  #[test]
  fn a_flat_grey_frame_stays_flat() {
    // With zero chroma and zero delta the inverse transform must reproduce
    // the Y value on all four Bayer positions: avg and cbcr steps add zero,
    // the delta step rebuilds G2 = Y, and the y step subtracts zero chroma.
    let (w, h) = (6, 4);
    let header = nikon_header(w as u16, h as u16);
    let grey = 123 << header.fq;
    let planes = vec![vec![grey; w * h], vec![0; w * h], vec![0; w * h], vec![0; w * h]];
    let out = transform(planes, &header).expect("transform runs");
    let expected = decompand(grey, 16383);
    for (ci, plane) in out.iter().enumerate() {
      assert!(plane.iter().all(|&v| v == expected), "plane {} is not flat", ci);
    }
  }

  #[test]
  fn interleaves_the_quad_planes_rggb() {
    let planes: Vec<Vec<u16>> = (0..4).map(|c| (0..6).map(|i| 100 * c + i).collect()).collect();
    let mosaic = interleave(&planes, 3, 2).expect("interleaves");
    #[rustfmt::skip]
    assert_eq!(mosaic, [
      0,   100, 1,   101, 2,   102,
      200, 300, 201, 301, 202, 302,
      3,   103, 4,   104, 5,   105,
      203, 303, 204, 304, 205, 305,
    ]);
    // Plane count and sizes are validated.
    assert!(interleave(&planes[..3], 3, 2).is_err());
    assert!(interleave(&planes, 2, 2).is_err());
  }

  #[test]
  fn rejects_wrong_component_counts_and_sizes() {
    let header = nikon_header(4, 4);
    assert!(transform(vec![vec![0; 16]; 3], &header).is_err(), "3 planes must fail");
    assert!(transform(vec![vec![0; 15]; 4], &header).is_err(), "short planes must fail");
    let mut header12 = nikon_header(4, 4);
    header12.components[2].bit_depth = 12;
    assert!(transform(vec![vec![0; 16]; 4], &header12).is_err(), "mixed depths must fail");
  }

  // Whole-pipeline comparison against the patched SVT-JPEG-XS decoder:
  // entropy -> IDWT -> inverse MCT -> decompanding, against its final planes.
  // See [`super::super::reference`] for how the tests find the data.
  // Regenerate with:
  //
  //   python3 tools/nefhe.py samples/same-scene/NZ6_3368.NEF \
  //       --extract he_star.jxs
  //   env NIKON_TOLERANT=1 NIKON_PORDER=1 NIKON_SPLIT=6 NIKON_FIRSTPREC=0 \
  //       NIKON_CORDER=0123 NIKON_WGT=2 \
  //     SvtJpegxsDecApp -i he_star.jxs -o ref_final.planes -v 0
  //
  // ref_final.planes is 4 planes (R, G1, G2, B) of 3032x2020 u16le --
  // 48997120 bytes exactly. Anything else is a truncated write and fails the
  // test rather than skipping it.
  #[test]
  fn matches_the_c_reference_planes() {
    let Some(dir) = reference::dir("matches_the_c_reference_planes") else {
      return;
    };
    let stream = reference::read(&dir, "he_star.jxs");
    let want_planes = reference::read(&dir, "ref_final.planes");

    let (header, planes) = super::super::decode_planes(&stream).expect("stream decodes");
    let (w, h) = (header.plane_width(), header.plane_height());
    assert_eq!((w, h), (3032, 2020));
    assert_eq!(want_planes.len(), 4 * w * h * 2, "ref_final.planes is truncated or misshapen");

    for (ci, (plane, want)) in planes.iter().zip(want_planes.chunks_exact(w * h * 2)).enumerate() {
      for (i, (&g, e)) in plane.iter().zip(want.chunks_exact(2)).enumerate() {
        let e = u16::from_le_bytes([e[0], e[1]]);
        assert_eq!(g, e, "plane {} diverges at x={} y={}", ci, i % w, i / w);
      }
    }
  }

  #[test]
  fn decodes_the_full_frame_to_a_mosaic() {
    let Some(dir) = reference::dir("decodes_the_full_frame_to_a_mosaic") else {
      return;
    };
    let stream = reference::read(&dir, "he_star.jxs");
    let (mosaic, width, height) = super::super::decode(&stream).expect("stream decodes");
    assert_eq!((width, height), (6064, 4040));
    assert_eq!(mosaic.len(), width * height);
    // The mosaic is the interleave of the plane decode: spot-check the four
    // corners of the first quad against decode_planes.
    let (_, planes) = super::super::decode_planes(&stream).expect("stream decodes");
    assert_eq!(mosaic[0], planes[0][0]);
    assert_eq!(mosaic[1], planes[1][0]);
    assert_eq!(mosaic[width], planes[2][0]);
    assert_eq!(mosaic[width + 1], planes[3][0]);
  }
}
