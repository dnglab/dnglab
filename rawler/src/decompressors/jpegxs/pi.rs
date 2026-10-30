// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026

//! Picture information: the band and packet topology derived from a parsed
//! [`Header`]. Pure arithmetic, no bitstream reading.
//!
//! Everything downstream keys off this layout:
//!
//! * bands are numbered component-major (all of component 0's bands, then
//!   component 1's, ...), which is the order the per-precinct 2-bit coding
//!   modes are stored in;
//! * packets within a precinct are ordered subband-major (vertical half
//!   first, component second), which is both the emission order of the
//!   packets *and* the order of the WGT (gain, priority) entries;
//! * those two orders differ, so the WGT entries are matched to band ids by
//!   walking the packets. Getting that wrong is silent: the GCLI code is
//!   unary, so a wrong GTLI changes decoded values but never code lengths.

use core::ops::Range;

use crate::RawlerError;
use crate::Result;

use super::header::Header;

/// One wavelet band of one component (or the single band of a component with
/// suppressed decomposition).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Band {
  pub component: usize,
  /// Band index within its component; 0 is the most decomposed (lowest
  /// frequency) band.
  pub local: usize,
  /// Width of one band line, in coefficients.
  pub width: usize,
  /// Width of one band line in code groups: `ceil(width / Ng)`.
  pub gcli_width: usize,
  /// Width of one band line in significance groups: `ceil(gcli_width / Ss)`.
  pub significance_width: usize,
  /// Band lines contributed per precinct. 1 for every band of a decomposed
  /// component here (NLy = 1 halves the vertical resolution of both the
  /// vertical low- and high-pass), and the full precinct height for the
  /// single band of a suppressed component.
  pub lines_per_precinct: usize,
  /// Quantisation gain and refinement priority, taken from the WGT table
  /// after it is matched to band ids in [`build`].
  pub gain: u8,
  pub priority: u8,
}

impl Band {
  /// Greatest trailing line index: how many magnitude bit planes the encoder
  /// dropped from this band, given the quantisation `qp` and refinement `rp`
  /// in the precinct header. `max` caps the result at the largest bit-plane
  /// count the stream can represent.
  pub fn gtli(&self, qp: u8, rp: u8, max: u8) -> u8 {
    let refine = if self.priority < rp { 1u8 } else { 0u8 };
    qp.saturating_sub(self.gain).saturating_sub(refine).min(max)
  }
}

/// One packet of a precinct: a contiguous run of one component's bands.
///
/// Every packet carries exactly one line of each band it names. For a
/// decomposed component that is the band's whole per-precinct contribution;
/// for a suppressed component, whose single band spans the full precinct
/// height, `line` selects which of its lines this packet carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
  pub component: usize,
  /// Global ids of the bands this packet carries.
  pub bands: Range<usize>,
  /// Vertical half of the precinct this packet belongs to; for a suppressed
  /// component this is also the precinct line the packet carries.
  pub line: usize,
}

/// Band and packet layout of one precinct, the same for every precinct in the
/// frame. Nikon plane heights divide exactly by the precinct height; a ragged
/// last precinct would need per-precinct heights.
#[derive(Debug, Clone)]
pub struct Topology {
  /// All bands, component-major: component 0's bands first, in frequency
  /// order (band 0 most decomposed), then component 1's, and so on.
  pub bands: Vec<Band>,
  /// Packets of one precinct, in emission order.
  pub packets: Vec<Packet>,
  /// Range of global band ids belonging to each component.
  pub component_bands: Vec<Range<usize>>,
}

/// Horizontal band widths of a plane of width `w` split `nlx` times, most
/// decomposed first. Each split takes `w / 2` for the high-pass and leaves
/// `w - w / 2` for the low-pass, which is split again; band 0 is the final
/// low-pass. For `w = 3032, nlx = 5` this yields `95, 95, 189, 379, 758,
/// 1516` (the odd widths come from 379 splitting as 190 + 189).
fn horizontal_widths(mut w: usize, nlx: u8) -> Vec<usize> {
  let mut highs = Vec::with_capacity(nlx as usize);
  for _ in 0..nlx {
    let high = w / 2;
    highs.push(high);
    w -= high;
  }
  let mut widths = Vec::with_capacity(nlx as usize + 1);
  widths.push(w);
  widths.extend(highs.into_iter().rev());
  widths
}

/// Index at which a decomposed component's bands split into its two packets:
/// the first index where the accumulated per-precinct coefficient count
/// reaches half the component's total. With NLy = 1 the vertical low-pass
/// bands (0..=NLx) sum to exactly one plane line, so the split lands after
/// them and the vertical high-pass pair forms the second packet.
fn split_index(coeffs: &[usize]) -> usize {
  let total: usize = coeffs.iter().sum();
  let mut acc = 0;
  for (i, c) in coeffs.iter().enumerate() {
    acc += c;
    if 2 * acc >= total {
      return i + 1;
    }
  }
  coeffs.len()
}

/// Work out the band and packet topology for a parsed header.
///
/// Only the Nikon layout is supported: frame NLy = 1, with every component
/// either skipping the wavelet or decomposed with that same NLy. Anything
/// else fails rather than producing a layout that looks plausible and is
/// wrong.
pub fn build(header: &Header) -> Result<Topology> {
  if header.components.is_empty() {
    return Err(RawlerError::DecoderFailed("JPEG XS: no components".into()));
  }
  if header.group_size == 0 || header.sig_group_size == 0 {
    return Err(RawlerError::DecoderFailed("JPEG XS: zero group size".into()));
  }
  if header.nly != 1 {
    return Err(RawlerError::DecoderFailed(format!(
      "JPEG XS: only NLy = 1 layouts are supported, got {}",
      header.nly
    )));
  }

  let plane_width = header.plane_width();
  let ng = header.group_size as usize;
  let ss = header.sig_group_size as usize;
  let precinct_lines = header.precinct_height();

  let mut bands: Vec<Band> = Vec::with_capacity(header.total_bands());
  let mut component_bands: Vec<Range<usize>> = Vec::with_capacity(header.components.len());
  for (ci, comp) in header.components.iter().enumerate() {
    let start = bands.len();
    let (widths, lines_per_precinct) = if comp.suppressed() {
      // No wavelet: one band the full plane width, spanning every precinct line.
      (vec![plane_width], precinct_lines)
    } else {
      if comp.nly != 1 {
        return Err(RawlerError::DecoderFailed(format!(
          "JPEG XS: component {} has unsupported decomposition NLx={} NLy={}",
          ci, comp.nlx, comp.nly
        )));
      }
      // Bands 0..=NLx are the horizontal decomposition of the vertical
      // low-pass; the vertical high-pass is split horizontally once, giving
      // the final two bands (its low half first, `w - w/2` wide).
      let mut widths = horizontal_widths(plane_width, comp.nlx);
      widths.push(plane_width - plane_width / 2);
      widths.push(plane_width / 2);
      (widths, 1)
    };
    for (local, width) in widths.into_iter().enumerate() {
      let gcli_width = width.div_ceil(ng);
      bands.push(Band {
        component: ci,
        local,
        width,
        gcli_width,
        significance_width: gcli_width.div_ceil(ss),
        lines_per_precinct,
        gain: 0,
        priority: 0,
      });
    }
    component_bands.push(start..bands.len());
    debug_assert_eq!(bands.len() - start, comp.bands());
  }

  // Packet layout, subband-major: for each vertical half, walk the
  // components in index order.
  let mut packets: Vec<Packet> = Vec::with_capacity(2 * header.components.len());
  for half in 0..2 {
    for (ci, comp) in header.components.iter().enumerate() {
      let range = component_bands[ci].clone();
      let packet_bands = if comp.suppressed() {
        // One packet per precinct line, both naming the same single band.
        range
      } else {
        let coeffs: Vec<usize> = bands[range.clone()].iter().map(|b| b.width * b.lines_per_precinct).collect();
        let split = range.start + split_index(&coeffs);
        if half == 0 { range.start..split } else { split..range.end }
      };
      packets.push(Packet {
        component: ci,
        bands: packet_bands,
        line: half,
      });
    }
  }

  // Map the WGT entries onto bands: entries are in packet-emission order,
  // each band taking the next entry the first time a packet names it. The
  // suppressed component's second packet re-names an already-seen band and
  // consumes nothing.
  let mut assigned = 0usize;
  let mut seen = vec![false; bands.len()];
  for packet in &packets {
    for id in packet.bands.clone() {
      if !seen[id] {
        seen[id] = true;
        let (gain, priority) = *header
          .weights
          .get(assigned)
          .ok_or_else(|| RawlerError::DecoderFailed(format!("JPEG XS: WGT has {} entries but the packet layout needs more", header.weights.len())))?;
        bands[id].gain = gain;
        bands[id].priority = priority;
        assigned += 1;
      }
    }
  }
  if assigned != header.weights.len() || assigned != bands.len() {
    return Err(RawlerError::DecoderFailed(format!(
      "JPEG XS: packet layout covers {} of {} bands but WGT has {} entries",
      assigned,
      bands.len(),
      header.weights.len()
    )));
  }

  Ok(Topology {
    bands,
    packets,
    component_bands,
  })
}

#[cfg(test)]
mod tests {
  use super::super::header::parse;
  use super::super::header::testdata::SAMPLE;
  use super::*;

  fn nikon_topology() -> Topology {
    build(&parse(&SAMPLE).expect("header parses")).expect("topology builds")
  }

  /// The eight band widths of a decomposed component, band 0 first.
  const WIDTHS: [usize; 8] = [95, 95, 189, 379, 758, 1516, 1516, 1516];

  #[test]
  fn derives_the_band_widths_from_the_wavelet_split() {
    let t = nikon_topology();
    for ci in [0usize, 1, 3] {
      let widths: Vec<usize> = t.bands[t.component_bands[ci].clone()].iter().map(|b| b.width).collect();
      assert_eq!(widths, WIDTHS, "component {}", ci);
    }
    // Sum is one whole precinct of the component: 2 lines x 3032.
    assert_eq!(WIDTHS.iter().sum::<usize>(), 6064);
    // The suppressed component has a single full-width band.
    assert_eq!(t.bands[t.component_bands[2].clone()].iter().map(|b| b.width).collect::<Vec<_>>(), [3032]);
  }

  #[test]
  fn computes_gcli_and_significance_widths() {
    let t = nikon_topology();
    // ceil(width / 4) and ceil(gcli_width / 8) for each band width above.
    let expected = [(24, 3), (24, 3), (48, 6), (95, 12), (190, 24), (379, 48), (379, 48), (379, 48)];
    for ci in [0usize, 1, 3] {
      let got: Vec<(usize, usize)> = t.bands[t.component_bands[ci].clone()]
        .iter()
        .map(|b| (b.gcli_width, b.significance_width))
        .collect();
      assert_eq!(got, expected, "component {}", ci);
    }
    let d = &t.bands[t.component_bands[2].start];
    assert_eq!((d.gcli_width, d.significance_width), (758, 95));
  }

  #[test]
  fn numbers_bands_component_major() {
    let t = nikon_topology();
    assert_eq!(t.bands.len(), 25);
    assert_eq!(t.component_bands, [0..8, 8..16, 16..17, 17..25]);
    for (ci, range) in t.component_bands.iter().enumerate() {
      for (local, id) in range.clone().enumerate() {
        assert_eq!(t.bands[id].component, ci);
        assert_eq!(t.bands[id].local, local);
      }
    }
    // One line per precinct for decomposed bands, the full precinct height
    // for the suppressed component's band.
    for b in &t.bands {
      assert_eq!(b.lines_per_precinct, if b.component == 2 { 2 } else { 1 });
    }
  }

  #[test]
  fn lays_out_packets_subband_major() {
    let t = nikon_topology();
    let got: Vec<(usize, Range<usize>, usize)> = t.packets.iter().map(|p| (p.component, p.bands.clone(), p.line)).collect();
    assert_eq!(
      got,
      [
        (0, 0..6, 0),   // comp0 bands 0..5
        (1, 8..14, 0),  // comp1 bands 0..5
        (2, 16..17, 0), // comp2 line 0
        (3, 17..23, 0), // comp3 bands 0..5
        (0, 6..8, 1),   // comp0 bands 6,7
        (1, 14..16, 1), // comp1 bands 6,7
        (2, 16..17, 1), // comp2 line 1
        (3, 23..25, 1), // comp3 bands 6,7
      ]
    );
  }

  #[test]
  fn every_packet_holds_one_plane_line_of_coefficients() {
    let t = nikon_topology();
    assert_eq!(t.packets.len(), 8);
    for (i, p) in t.packets.iter().enumerate() {
      let coefficients: usize = t.bands[p.bands.clone()].iter().map(|b| b.width).sum();
      assert_eq!(coefficients, 3032, "packet {}", i);
    }
  }

  /// The WGT table is stored in packet-emission order, not band-id order, and
  /// the suppressed component's two packets share a single entry. Getting
  /// this mapping wrong is silent: the GCLI code is unary, so a wrong GTLI
  /// changes decoded values but never code lengths.
  #[test]
  fn maps_wgt_entries_in_packet_emission_order() {
    let t = nikon_topology();
    #[rustfmt::skip]
    let band_of_entry: [usize; 25] = [
      0, 1, 2, 3, 4, 5,        // entries  0..=5   comp0 bands 0..5
      8, 9, 10, 11, 12, 13,    // entries  6..=11  comp1 bands 0..5
      16,                      // entry   12       comp2's single band
      17, 18, 19, 20, 21, 22,  // entries 13..=18  comp3 bands 0..5
      6, 7,                    // entries 19..=20  comp0 bands 6,7
      14, 15,                  // entries 21..=22  comp1 bands 6,7
      23, 24,                  // entries 23..=24  comp3 bands 6,7
    ];
    let h = parse(&SAMPLE).unwrap();
    for (entry, &id) in band_of_entry.iter().enumerate() {
      assert_eq!((t.bands[id].gain, t.bands[id].priority), h.weights[entry], "entry {}", entry);
    }
  }

  #[test]
  fn applies_the_truncation_rule() {
    let band = |gain, priority| Band {
      component: 0,
      local: 0,
      width: 0,
      gcli_width: 0,
      significance_width: 0,
      lines_per_precinct: 1,
      gain,
      priority,
    };
    // qp - gain, minus one refinement bit when priority < rp.
    assert_eq!(band(1, 10).gtli(4, 3, 15), 3);
    assert_eq!(band(1, 2).gtli(4, 3, 15), 2);
    // Saturates at zero rather than wrapping.
    assert_eq!(band(5, 0).gtli(4, 3, 15), 0);
    assert_eq!(band(4, 0).gtli(4, 3, 15), 0);
    // Clamped to the caller's maximum.
    assert_eq!(band(0, 10).gtli(30, 0, 15), 15);
  }

  #[test]
  fn rejects_unsupported_decompositions() {
    let mut h = parse(&SAMPLE).unwrap();
    h.nly = 2;
    assert!(build(&h).is_err());

    let mut h = parse(&SAMPLE).unwrap();
    h.components[0].nly = 0;
    h.components[0].nlx = 5; // decomposed, but not the supported NLy = 1
    assert!(build(&h).is_err());
  }
}
