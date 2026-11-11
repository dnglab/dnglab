// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026

//! Inverse discrete wavelet transform: reconstructs each component plane from
//! its decoded, dequantised bands, one precinct at a time.
//!
//! Nikon uses one vertical decomposition level and up to five horizontal
//! ones, all with the reversible 5/3 lifting filter. Bands `0..=NLx` are the
//! horizontal decomposition of the vertical low-pass and carry one line per
//! precinct; the last two bands are the horizontal split of the vertical
//! high-pass.
//!
//! So a precinct reconstructs two plane lines, except that the vertical
//! lifting needs the *next* precinct's low-pass to finish the second of them.
//! Output therefore lags input by one precinct: precinct 0 completes line 0
//! only, precinct `p` completes lines `2p-1` and `2p`, and the last precinct
//! also closes line `2N-1` with the boundary filter.
//!
//! A component that skips the wavelet (Nikon's Δ plane) is copied straight
//! through: its single full-width band is plane content, two lines per
//! precinct. Which component that is comes from the component itself, never
//! from its index, because plain JPEG XS assumes it is the last one and
//! Nikon's is not.
//!
//! The `Fq` fractional bits are applied here, as a left shift on every
//! coefficient as it widens to 32 bits, so the lifting runs unshifted. The
//! reference decoder does the same. Planes come out in the `Bw` domain, still
//! scaled by `2^Fq` and centred on zero, and the output stage removes both.
//!
//! Inverse quantisation happens in the same widening step, which is where the
//! reference decoder puts it. `Qpih` picks the rule: 0 is the deadzone
//! quantiser, 1 the uniform one. Nikon uses 1.

use super::entropy::BandData;
use super::entropy::Precinct;
use super::entropy::SIGN_BIT;
use super::header::Header;
use super::pi::Topology;
use crate::RawlerError;
use crate::Result;

/// One level of inverse horizontal 5/3 lifting, interleaving the low-pass
/// (even samples) and high-pass (odd samples) halves of one line.
///
/// The edges use symmetric extension, as the reference decoder does: the
/// first even sample sees the first high-pass twice, and the tail depends on
/// whether the output length is odd or even. Inputs arrive already shifted,
/// and nothing is scaled here.
fn idwt_horizontal(lf: &[i32], hf: &[i32], out: &mut [i32]) {
  let len = out.len();
  debug_assert!(len >= 2);
  debug_assert_eq!(lf.len(), len - len / 2);
  debug_assert_eq!(hf.len(), len / 2);

  out[0] = lf[0] - ((hf[0] + 1) >> 1);
  for k in 0..(len - 2) / 2 {
    out[2 * k + 2] = lf[k + 1] - ((hf[k] + hf[k + 1] + 2) >> 2);
    out[2 * k + 1] = hf[k] + ((out[2 * k] + out[2 * k + 2]) >> 1);
  }
  if len % 2 == 1 {
    out[len - 1] = lf[lf.len() - 1] - ((hf[hf.len() - 1] + 1) >> 1);
    out[len - 2] = hf[hf.len() - 1] + ((out[len - 3] + out[len - 1]) >> 1);
  } else {
    out[len - 1] = hf[hf.len() - 1] + out[len - 2];
  }
}

/// The inverse quantiser and the coefficient scaling, which every band line
/// is read through.
#[derive(Debug, Clone, Copy)]
struct Quant {
  /// Coefficients per code group (`Ng`), the granularity GCLIs apply at.
  ng: usize,
  /// `Qpih = 1`: uniform inverse quantisation rather than deadzone.
  uniform: bool,
  /// Fractional bits (`Fq`), applied as a left shift on every coefficient.
  fq: u8,
}

/// Dequantise one band line and widen it to 32 bits with the `Fq` shift
/// applied. [`Quant::uniform`] selects `Qpih = 1` (fold the magnitude down by
/// the coded width, `gcli - gtli + 1` bits at a time) over `Qpih = 0`
/// (deadzone: set bit `gtli - 1`); both only touch non-zero coefficients of
/// groups that coded at least one bit plane, and both are no-ops at
/// `gtli = 0`.
fn extract_line(data: &BandData, width: usize, line: usize, q: Quant, out: &mut [i32]) {
  let Quant { ng, uniform, fq } = q;
  let gtli = data.gtli;
  let coeff = &data.coeff[line * width..(line + 1) * width];
  let gclis = &data.gcli[line * width.div_ceil(ng)..];
  for (i, (o, &v)) in out.iter_mut().zip(coeff).enumerate() {
    let mut mag = (v & !SIGN_BIT) as u32;
    let gcli = gclis[i / ng];
    if gtli > 0 && gcli > gtli && mag != 0 {
      if uniform {
        let scale = gcli - gtli + 1;
        let mut val = mag;
        mag = 0;
        while val > 0 {
          mag += val;
          val >>= scale;
        }
      } else {
        mag |= 1 << (gtli - 1);
      }
    }
    let signed = if v & SIGN_BIT != 0 { -(mag as i32) } else { mag as i32 };
    *o = signed << fq;
  }
}

/// Per-component reconstruction state.
struct Component {
  /// Global id of this component's first band; its bands are contiguous.
  first_band: usize,
  /// Band widths in local band order (band 0 most decomposed).
  widths: Vec<usize>,
  suppressed: bool,
  /// Vertical high-pass line of the previous precinct, reconstructed
  /// horizontally. Meaningless before the first precinct and for suppressed
  /// components.
  hf_prev: Vec<i32>,
}

/// Streaming inverse wavelet transform. Feed the frame's precincts in order
/// with [`push_precinct`](Self::push_precinct), then take the planes with
/// [`finish`](Self::finish).
pub struct Idwt {
  plane_width: usize,
  precinct_count: usize,
  precinct_height: usize,
  quant: Quant,
  /// Index of the next precinct expected.
  next: usize,
  comps: Vec<Component>,
  /// One `Bw`-domain plane per component, `plane_width * plane_height`.
  planes: Vec<Vec<i32>>,
  /// Ping-pong pair for the horizontal cascade; `lo` ends up holding the
  /// vertical low-pass line.
  lo: Vec<i32>,
  swap: Vec<i32>,
  /// High-pass band line of the current cascade stage.
  hi: Vec<i32>,
  /// Vertical high-pass line of the current precinct; swapped into the
  /// component's `hf_prev` once used.
  hf: Vec<i32>,
}

impl Idwt {
  /// Check the geometry and allocate the plane and scratch buffers.
  ///
  /// Only layouts that [`super::pi::build`] produces are accepted, and the
  /// plane height has to divide exactly by the precinct height. Nikon streams
  /// always do; a ragged last precinct would need the odd-height boundary
  /// filter.
  pub fn new(header: &Header, topology: &Topology) -> Result<Self> {
    let w = header.plane_width();
    let h = header.plane_height();
    let precinct_height = header.precinct_height();
    let precinct_count = header.precinct_count();
    if header.qpih > 1 {
      return Err(RawlerError::DecoderFailed(format!("JPEG XS: unknown inverse quantiser Qpih = {}", header.qpih)));
    }
    if header.group_size == 0 {
      return Err(RawlerError::DecoderFailed("JPEG XS: zero code group size".into()));
    }
    if w < 2 || h < 2 {
      return Err(RawlerError::DecoderFailed(format!("JPEG XS: plane {}x{} is too small for the wavelet", w, h)));
    }
    if h != precinct_count * precinct_height {
      return Err(RawlerError::DecoderFailed(format!(
        "JPEG XS: plane height {} is not a multiple of the precinct height {}",
        h, precinct_height
      )));
    }

    let mut comps = Vec::with_capacity(header.components.len());
    for (ci, comp) in header.components.iter().enumerate() {
      let range = topology.component_bands[ci].clone();
      let widths: Vec<usize> = topology.bands[range.clone()].iter().map(|b| b.width).collect();
      if comp.suppressed() {
        if widths != [w] {
          return Err(RawlerError::DecoderFailed(format!(
            "JPEG XS: suppressed component {} has band width {:?}, expected the plane width {}",
            ci, widths, w
          )));
        }
      } else {
        // Each stage of the cascade needs its low-pass half to be the
        // ceiling half of that stage's output, and the whole thing has to sum
        // to the plane width. That follows from how pi::build splits the
        // widths, but check it anyway so a corrupt header fails here instead
        // of panicking on a wrong-length slice mid-transform.
        let nlx = comp.nlx as usize;
        let ok = widths.len() == nlx + 3 && {
          let mut lo_len = widths[0];
          let mut ok = true;
          for &wb in &widths[1..=nlx] {
            let len = lo_len + wb;
            ok &= len >= 2 && lo_len == len - len / 2;
            lo_len = len;
          }
          ok && lo_len == w && widths[nlx + 1] == w - w / 2 && widths[nlx + 2] == w / 2
        };
        if !ok {
          return Err(RawlerError::DecoderFailed(format!(
            "JPEG XS: component {} band widths {:?} do not form a {}-level split of {}",
            ci, widths, comp.nlx, w
          )));
        }
      }
      comps.push(Component {
        first_band: range.start,
        widths,
        suppressed: comp.suppressed(),
        hf_prev: vec![0; if comp.suppressed() { 0 } else { w }],
      });
    }

    Ok(Self {
      plane_width: w,
      precinct_count,
      precinct_height,
      quant: Quant {
        ng: header.group_size as usize,
        uniform: header.qpih == 1,
        fq: header.fq,
      },
      next: 0,
      planes: vec![vec![0; w * h]; comps.len()],
      comps,
      lo: vec![0; w],
      swap: vec![0; w],
      hi: vec![0; w],
      hf: vec![0; w],
    })
  }

  /// Transform one precinct, completing the plane lines its data closes.
  /// Precincts must arrive in frame order, exactly as
  /// [`super::entropy::decode_precinct`] returned them.
  pub fn push_precinct(&mut self, precinct: &Precinct) -> Result<()> {
    let p = self.next;
    if p >= self.precinct_count {
      return Err(RawlerError::DecoderFailed(format!(
        "JPEG XS: precinct {} pushed into a frame of {}",
        p, self.precinct_count
      )));
    }
    self.next += 1;

    let w = self.plane_width;
    let q = self.quant;
    for ci in 0..self.comps.len() {
      if self.comps[ci].suppressed {
        // No wavelet: the band is plane content, one row per precinct line.
        let band = &precinct.bands[self.comps[ci].first_band];
        let base = p * self.precinct_height;
        for line in 0..self.precinct_height {
          extract_line(band, w, line, q, &mut self.planes[ci][(base + line) * w..(base + line + 1) * w]);
        }
        continue;
      }

      // Horizontal pass: cascade bands 0..=NLx up to the vertical low-pass
      // line, then rebuild the vertical high-pass line from the last two
      // bands.
      let first = self.comps[ci].first_band;
      let widths = &self.comps[ci].widths;
      let nlx = widths.len() - 3;
      let mut lo_len = widths[0];
      extract_line(&precinct.bands[first], lo_len, 0, q, &mut self.lo[..lo_len]);
      for b in 1..=nlx {
        let wb = self.comps[ci].widths[b];
        extract_line(&precinct.bands[first + b], wb, 0, q, &mut self.hi[..wb]);
        idwt_horizontal(&self.lo[..lo_len], &self.hi[..wb], &mut self.swap[..lo_len + wb]);
        std::mem::swap(&mut self.lo, &mut self.swap);
        lo_len += wb;
      }
      let (wl, wh) = (self.comps[ci].widths[nlx + 1], self.comps[ci].widths[nlx + 2]);
      extract_line(&precinct.bands[first + nlx + 1], wl, 0, q, &mut self.swap[..wl]);
      extract_line(&precinct.bands[first + nlx + 2], wh, 0, q, &mut self.hi[..wh]);
      idwt_horizontal(&self.swap[..wl], &self.hi[..wh], &mut self.hf);

      // Vertical pass. `lo` is the vertical low-pass line p, `hf` the high-pass
      // line p and `hf_prev` line p-1; plane line 2p-2 is already final.
      let plane = &mut self.planes[ci];
      let (lf, hf, hf_prev) = (&self.lo, &self.hf, &self.comps[ci].hf_prev);
      if p == 0 {
        for x in 0..w {
          plane[x] = lf[x] - ((hf[x] + 1) >> 1);
        }
      } else {
        let even = 2 * p;
        for x in 0..w {
          plane[even * w + x] = lf[x] - ((hf_prev[x] + hf[x] + 2) >> 2);
        }
        // Line 2p-1 sits between two finished even lines.
        let (head, tail) = plane.split_at_mut((even - 1) * w);
        let above = &head[(even - 2) * w..];
        let (odd, below) = tail.split_at_mut(w);
        for x in 0..w {
          odd[x] = hf_prev[x] + ((above[x] + below[x]) >> 1);
        }
      }
      if p == self.precinct_count - 1 {
        // Bottom boundary: the final line extends symmetrically off the last
        // even one. With a single precinct that even line is line 0.
        let (head, last) = plane.split_at_mut((2 * p + 1) * w);
        let even = &head[2 * p * w..];
        for x in 0..w {
          last[x] = hf[x] + even[x];
        }
      }
      std::mem::swap(&mut self.comps[ci].hf_prev, &mut self.hf);
    }
    Ok(())
  }

  /// Take the reconstructed planes, one per component in component order,
  /// each `plane_width * plane_height` in the `Bw` domain.
  pub fn finish(self) -> Result<Vec<Vec<i32>>> {
    if self.next != self.precinct_count {
      return Err(RawlerError::DecoderFailed(format!(
        "JPEG XS: only {} of {} precincts were transformed",
        self.next, self.precinct_count
      )));
    }
    Ok(self.planes)
  }
}

#[cfg(test)]
mod tests {
  use super::super::entropy::SIGN_BIT;
  use super::super::header::Component;
  use super::super::pi;
  use super::*;

  /// Forward 5/3 lifting with symmetric extension, the exact inverse of
  /// [`idwt_horizontal`]. Test-only: the decoder never needs it, but the
  /// transform is reversible, so round-tripping arbitrary data through it
  /// pins down every boundary case of the inverse.
  fn fwd53(x: &[i32]) -> (Vec<i32>, Vec<i32>) {
    let len = x.len();
    let mut hf = Vec::with_capacity(len / 2);
    for n in 0..len / 2 {
      let right = if 2 * n + 2 < len { x[2 * n + 2] } else { x[len - 2] };
      hf.push(x[2 * n + 1] - ((x[2 * n] + right) >> 1));
    }
    let mut lf = Vec::with_capacity(len - len / 2);
    for n in 0..len - len / 2 {
      let left = hf[n.saturating_sub(1)];
      let right = hf[n.min(hf.len() - 1)];
      lf.push(x[2 * n] + ((left + right + 2) >> 2));
    }
    (lf, hf)
  }

  /// Small deterministic pseudo-random values in a wavelet-plausible range.
  fn noise(n: usize, seed: u64) -> Vec<i32> {
    let mut state = seed | 1;
    (0..n)
      .map(|_| {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((state >> 33) as i32 & 0x3fff) - 0x2000
      })
      .collect()
  }

  #[test]
  fn horizontal_lifting_round_trips_every_parity() {
    for len in [2usize, 3, 4, 5, 6, 7, 95, 189, 190, 379, 758, 3032] {
      let x = noise(len, len as u64);
      let (lf, hf) = fwd53(&x);
      let mut out = vec![0; len];
      idwt_horizontal(&lf, &hf, &mut out);
      assert_eq!(out, x, "length {}", len);
    }
  }

  #[test]
  fn dequantises_while_widening() {
    // gtli = 2, one group with gcli = 5 (three coded planes, magnitudes are
    // multiples of 4) and one fully truncated group.
    let band = BandData {
      gtli: 2,
      gcli: vec![5, 2],
      coeff: vec![8, 0, SIGN_BIT | 16, 4, 0, 0, 0, 0],
    };
    // Uniform (Qpih = 1): fold the magnitude down by gcli - gtli + 1 = 4 bits
    // at a time, so 16 gains its own tail (16 >> 4 = 1) and smaller values
    // do not.
    let mut out = [0i32; 8];
    extract_line(&band, 8, 0, Quant { ng: 4, uniform: true, fq: 0 }, &mut out);
    assert_eq!(out, [8, 0, -17, 4, 0, 0, 0, 0]);
    // Deadzone (Qpih = 0): set bit gtli - 1 on the same coefficients.
    extract_line(&band, 8, 0, Quant { ng: 4, uniform: false, fq: 0 }, &mut out);
    assert_eq!(out, [10, 0, -18, 6, 0, 0, 0, 0]);
    // The Fq shift applies after reconstruction.
    extract_line(&band, 8, 0, Quant { ng: 4, uniform: true, fq: 4 }, &mut out);
    assert_eq!(out, [128, 0, -272, 64, 0, 0, 0, 0]);
  }

  #[test]
  fn a_constant_line_survives_the_lifting() {
    // Constant input decomposes to constant low-pass and zero high-pass.
    let mut out = vec![0; 7];
    idwt_horizontal(&[42; 4], &[0; 3], &mut out);
    assert_eq!(out, [42; 7]);
  }

  /// A Header/Topology pair for a synthetic single-component frame. `nlx = 0`
  /// makes the component suppressed (no wavelet).
  fn synthetic(grid: u16, nlx: u8) -> (Header, Topology) {
    let nly = if nlx == 0 { 0 } else { 1 };
    let mut header = Header {
      lcod: 0,
      grid_width: grid,
      grid_height: grid,
      precinct_width: 0,
      slice_height: 16,
      group_size: 4,
      sig_group_size: 8,
      bw: 18,
      fq: 0,
      br: 4,
      ppoc: 1,
      cpih: 0,
      nlx: 5,
      nly: 1,
      qpih: 1,
      fs: 1,
      rm: 0,
      lh: 0,
      rl: 0,
      components: vec![Component {
        bit_depth: 14,
        sx: 1,
        sy: 1,
        nlx,
        nly,
      }],
      weights: Vec::new(),
      first_slice: 0,
    };
    header.weights = vec![(0, 0); header.total_bands()];
    let topology = pi::build(&header).expect("topology builds");
    (header, topology)
  }

  /// Sign-magnitude encoding of a small signed coefficient.
  fn sm(v: i32) -> u16 {
    if v < 0 { SIGN_BIT | (-v) as u16 } else { v as u16 }
  }

  /// Forward-transform a plane into per-precinct [`Precinct`]s laid out the
  /// way the entropy decoder produces them: bands `0..=nlx` are the recursive
  /// horizontal split of the vertical low-pass, the last two bands the single
  /// horizontal split of the vertical high-pass, one line of each per
  /// precinct.
  fn forward_plane(plane: &[i32], w: usize, h: usize, topology: &Topology) -> Vec<Precinct> {
    // Vertical split on each column.
    let mut vlf = vec![0; w * (h - h / 2)];
    let mut vhf = vec![0; w * (h / 2)];
    for x in 0..w {
      let column: Vec<i32> = (0..h).map(|y| plane[y * w + x]).collect();
      let (lf, hf) = fwd53(&column);
      for (y, v) in lf.iter().enumerate() {
        vlf[y * w + x] = *v;
      }
      for (y, v) in hf.iter().enumerate() {
        vhf[y * w + x] = *v;
      }
    }

    let widths: Vec<usize> = topology.bands.iter().map(|b| b.width).collect();
    let nlx = widths.len() - 3;
    let precincts = h / 2;
    (0..precincts)
      .map(|p| {
        // Horizontal cascade on the vertical low-pass line.
        let mut rows: Vec<Vec<i32>> = Vec::new();
        let mut lo = vlf[p * w..(p + 1) * w].to_vec();
        for _ in 0..nlx {
          let (lf, hf) = fwd53(&lo);
          rows.push(hf);
          lo = lf;
        }
        rows.push(lo);
        rows.reverse(); // band 0 first
        // One horizontal split of the vertical high-pass line.
        let (lf, hf) = fwd53(&vhf[p * w..(p + 1) * w]);
        rows.push(lf);
        rows.push(hf);

        let bands = rows
          .iter()
          .zip(&widths)
          .map(|(row, &width)| {
            assert_eq!(row.len(), width);
            BandData {
              gtli: 0,
              gcli: vec![0; width.div_ceil(4)],
              coeff: row.iter().map(|&v| sm(v)).collect(),
            }
          })
          .collect();
        Precinct { bands }
      })
      .collect()
  }

  #[test]
  fn reconstructs_a_decomposed_plane_exactly() {
    // The 5/3 wavelet is reversible, so a forward transform of a random
    // plane must invert exactly, across several sizes so the first, middle
    // and last precincts and both horizontal parities are exercised.
    for (grid, nlx) in [(12u16, 2u8), (12, 1), (4, 1), (20, 3)] {
      let (header, topology) = synthetic(grid, nlx);
      let (w, h) = (header.plane_width(), header.plane_height());
      let plane = noise(w * h, grid as u64 * 31 + nlx as u64);
      let mut idwt = Idwt::new(&header, &topology).expect("Idwt builds");
      for precinct in forward_plane(&plane, w, h, &topology) {
        idwt.push_precinct(&precinct).expect("precinct transforms");
      }
      let planes = idwt.finish().expect("all precincts arrived");
      assert_eq!(planes[0], plane, "grid {} nlx {}", grid, nlx);
    }
  }

  #[test]
  fn copies_a_suppressed_component_with_the_fq_shift() {
    let (mut header, topology) = synthetic(12, 0);
    header.fq = 4;
    let (w, h) = (header.plane_width(), header.plane_height());
    let plane = noise(w * h, 7);
    let mut idwt = Idwt::new(&header, &topology).expect("Idwt builds");
    for p in 0..h / 2 {
      let coeff: Vec<u16> = plane[2 * p * w..(2 * p + 2) * w].iter().map(|&v| sm(v)).collect();
      idwt
        .push_precinct(&Precinct {
          bands: vec![BandData {
            gtli: 0,
            // Each line carries its own GCLI array of ceil(width / Ng) entries,
            // so two lines need twice that. Not ceil(2 * width / Ng), which
            // rounds differently whenever width is not a multiple of Ng.
            gcli: vec![0; 2 * w.div_ceil(4)],
            coeff,
          }],
        })
        .expect("precinct copies");
    }
    let planes = idwt.finish().unwrap();
    let expected: Vec<i32> = plane.iter().map(|&v| v << 4).collect();
    assert_eq!(planes[0], expected);
  }

  #[test]
  fn refuses_a_short_or_overlong_frame() {
    let (header, topology) = synthetic(12, 2);
    let (w, h) = (header.plane_width(), header.plane_height());
    let precincts = forward_plane(&noise(w * h, 3), w, h, &topology);

    let mut idwt = Idwt::new(&header, &topology).unwrap();
    idwt.push_precinct(&precincts[0]).unwrap();
    assert!(idwt.finish().is_err(), "finish before all precincts must fail");

    let mut idwt = Idwt::new(&header, &topology).unwrap();
    for p in &precincts {
      idwt.push_precinct(p).unwrap();
    }
    assert!(idwt.push_precinct(&precincts[0]).is_err(), "excess precinct must fail");
  }

  // The whole-stream comparison against the C reference decoder lives in
  // super::mct's tests: the C decoder's MCT-disabled output paths are broken
  // (they crash and write truncated planes), so the transform is verified
  // end-to-end (entropy, IDWT, inverse MCT, decompanding) against
  // the C decoder's final planes instead of post-IDWT intermediates.
}
