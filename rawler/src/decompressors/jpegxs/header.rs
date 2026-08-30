// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026

//! JPEG XS codestream headers, as used by Nikon "High Efficiency" NEF.
//!
//! The codestream is ISO/IEC 21122 with three vendor deviations:
//!
//! * `CAP` holds a 32-byte vendor payload instead of a capabilities bitmap.
//! * `PIH` is 39 bytes instead of 26. The first 24 payload bytes are the
//!   standard fields; the rest is vendor data of unknown meaning.
//! * `CDT` uses three bytes per component instead of two, the third holding
//!   that component's own wavelet decomposition.
//!
//! Only the third one matters. It lets a component other than the last skip
//! the wavelet, which plain JPEG XS cannot express.

use crate::RawlerError;
use crate::Result;

/// Marker codes. Only the ones that appear in Nikon streams are listed.
mod marker {
  pub const SOC: u16 = 0xff10;
  pub const EOC: u16 = 0xff11;
  pub const PIH: u16 = 0xff12;
  pub const CDT: u16 = 0xff13;
  pub const WGT: u16 = 0xff14;
  pub const SLH: u16 = 0xff20;
  pub const CAP: u16 = 0xff50;
}

/// Size of the standard part of a picture header, including the length field.
const PIH_STANDARD_LEN: usize = 26;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Component {
  pub bit_depth: u8,
  /// Horizontal and vertical subsampling factors.
  pub sx: u8,
  pub sy: u8,
  /// This component's own wavelet decomposition. Both zero means the component
  /// is coded without a wavelet ("suppressed decomposition").
  pub nlx: u8,
  pub nly: u8,
}

impl Component {
  pub fn suppressed(&self) -> bool {
    self.nlx == 0 && self.nly == 0
  }

  /// How many bands this component contributes.
  pub fn bands(&self) -> usize {
    if self.suppressed() {
      1
    } else {
      2 * self.nly as usize + self.nlx as usize + 1
    }
  }
}

/// The picture header, plus the component and weights tables.
#[derive(Debug, Clone)]
pub struct Header {
  /// Total codestream length in bytes, from SOC to EOC.
  pub lcod: u32,
  /// Size of the CFA sampling grid. Each component plane is half of this in
  /// both axes, even though the subsampling factors read as 1.
  pub grid_width: u16,
  pub grid_height: u16,
  /// Precinct width in multiples of 8; zero means one precinct per line.
  pub precinct_width: u16,
  /// Slice height in precincts.
  pub slice_height: u16,
  /// Coefficients per code group, and code groups per significance group.
  pub group_size: u8,
  pub sig_group_size: u8,
  /// Nominal wavelet coefficient precision, fractional bits, and the width of a
  /// raw bit-plane count.
  pub bw: u8,
  pub fq: u8,
  pub br: u8,
  /// Progression order of bands within a precinct. Nikon uses 1.
  pub ppoc: u8,
  /// Colour transform. Nikon declares 0, but the components really are
  /// star-tetrix decorrelated and the inverse still has to run.
  pub cpih: u8,
  /// Frame-wide decomposition. Individual components may differ; see
  /// [`Component`].
  pub nlx: u8,
  pub nly: u8,
  /// Inverse quantiser type, sign handling, run mode, long-header flag and
  /// per-packet raw-mode flag.
  pub qpih: u8,
  pub fs: u8,
  pub rm: u8,
  pub lh: u8,
  pub rl: u8,
  pub components: Vec<Component>,
  /// Band (gain, priority) pairs, in packet-emission order.
  pub weights: Vec<(u8, u8)>,
  /// Offset of the first slice header within the codestream.
  pub first_slice: usize,
}

impl Header {
  pub fn plane_width(&self) -> usize {
    self.grid_width as usize / 2
  }

  pub fn plane_height(&self) -> usize {
    self.grid_height as usize / 2
  }

  pub fn total_bands(&self) -> usize {
    self.components.iter().map(|c| c.bands()).sum()
  }

  /// Height of a precinct, in component-plane lines.
  pub fn precinct_height(&self) -> usize {
    1 << self.nly
  }

  pub fn precinct_count(&self) -> usize {
    self.plane_height().div_ceil(self.precinct_height())
  }

  pub fn slice_count(&self) -> usize {
    self.precinct_count().div_ceil(self.slice_height as usize)
  }
}

/// Reads big-endian fields with bounds checking.
struct Reader<'a> {
  buf: &'a [u8],
  pos: usize,
}

impl<'a> Reader<'a> {
  fn new(buf: &'a [u8]) -> Self {
    Self { buf, pos: 0 }
  }

  fn need(&self, n: usize) -> Result<()> {
    if self.pos + n > self.buf.len() {
      Err(RawlerError::DecoderFailed(format!(
        "JPEG XS: truncated header, wanted {} bytes at offset {} of {}",
        n,
        self.pos,
        self.buf.len()
      )))
    } else {
      Ok(())
    }
  }

  fn u8(&mut self) -> Result<u8> {
    self.need(1)?;
    let v = self.buf[self.pos];
    self.pos += 1;
    Ok(v)
  }

  fn u16(&mut self) -> Result<u16> {
    self.need(2)?;
    let v = u16::from_be_bytes([self.buf[self.pos], self.buf[self.pos + 1]]);
    self.pos += 2;
    Ok(v)
  }

  fn u32(&mut self) -> Result<u32> {
    self.need(4)?;
    let v = u32::from_be_bytes(self.buf[self.pos..self.pos + 4].try_into().expect("4 bytes"));
    self.pos += 4;
    Ok(v)
  }

  fn skip(&mut self, n: usize) -> Result<()> {
    self.need(n)?;
    self.pos += n;
    Ok(())
  }
}

/// Parse everything up to the first slice header, which is left unread.
pub fn parse(buf: &[u8]) -> Result<Header> {
  let mut r = Reader::new(buf);
  if r.u16()? != marker::SOC {
    return Err(RawlerError::DecoderFailed("JPEG XS: no SOC marker".into()));
  }

  let mut pih: Option<Header> = None;
  let mut ncomp = 0usize;
  let mut components: Vec<Component> = Vec::new();
  let mut weights: Vec<(u8, u8)> = Vec::new();

  loop {
    let here = r.pos;
    let code = r.u16()?;
    if code == marker::SLH {
      let mut h = pih.ok_or_else(|| RawlerError::DecoderFailed("JPEG XS: no PIH marker".into()))?;
      if components.is_empty() {
        return Err(RawlerError::DecoderFailed("JPEG XS: no CDT marker".into()));
      }
      if weights.is_empty() {
        return Err(RawlerError::DecoderFailed("JPEG XS: no WGT marker".into()));
      }
      h.components = components;
      h.weights = weights;
      h.first_slice = here;
      if h.weights.len() != h.total_bands() {
        return Err(RawlerError::DecoderFailed(format!(
          "JPEG XS: WGT has {} entries but the component table implies {} bands",
          h.weights.len(),
          h.total_bands()
        )));
      }
      return Ok(h);
    }
    if code == marker::EOC {
      return Err(RawlerError::DecoderFailed("JPEG XS: EOC before any slice".into()));
    }

    let len = r.u16()? as usize;
    if len < 2 {
      return Err(RawlerError::DecoderFailed(format!("JPEG XS: bad marker length {}", len)));
    }
    let body = len - 2;
    let body_start = r.pos;

    match code {
      // Vendor payload, no fields we need.
      marker::CAP => r.skip(body)?,

      marker::PIH => {
        if len < PIH_STANDARD_LEN {
          return Err(RawlerError::DecoderFailed(format!("JPEG XS: PIH length {} below the standard 26", len)));
        }
        let lcod = r.u32()?;
        let _ppih = r.u16()?;
        let _plev = r.u16()?;
        let grid_width = r.u16()?;
        let grid_height = r.u16()?;
        let precinct_width = r.u16()?;
        let slice_height = r.u16()?;
        ncomp = r.u8()? as usize;
        let group_size = r.u8()?;
        let sig_group_size = r.u8()?;
        let bw = r.u8()?;
        let fq_br = r.u8()?;
        let flags = r.u8()?;
        let decomp = r.u8()?;
        let tail = r.u8()?;
        // Anything past the standard 26 bytes is vendor data.
        r.skip(body_start + body - r.pos)?;

        if slice_height == 0 {
          return Err(RawlerError::DecoderFailed("JPEG XS: slice height is zero".into()));
        }
        if ncomp == 0 {
          return Err(RawlerError::DecoderFailed("JPEG XS: component count is zero".into()));
        }
        // The packet layout in `super::pi` assumes this progression order.
        if (flags >> 4) & 0x07 != 1 {
          return Err(RawlerError::DecoderFailed(format!(
            "JPEG XS: unsupported progression order Ppoc = {}",
            (flags >> 4) & 0x07
          )));
        }
        pih = Some(Header {
          lcod,
          grid_width,
          grid_height,
          precinct_width,
          slice_height,
          group_size,
          sig_group_size,
          bw,
          fq: fq_br >> 4,
          br: fq_br & 0x0f,
          ppoc: (flags >> 4) & 0x07,
          cpih: flags & 0x0f,
          nlx: decomp >> 4,
          nly: decomp & 0x0f,
          lh: tail >> 7,
          rl: (tail >> 6) & 1,
          qpih: (tail >> 4) & 3,
          fs: (tail >> 2) & 3,
          rm: tail & 3,
          components: Vec::new(),
          weights: Vec::new(),
          first_slice: 0,
        });
      }

      marker::CDT => {
        // Three bytes per component in Nikon streams, two in stock JPEG XS,
        // where every component takes the frame-wide decomposition instead.
        let frame = pih.as_ref().ok_or_else(|| RawlerError::DecoderFailed("JPEG XS: CDT before PIH".into()))?;
        // The PIH branch above rejected a zero component count.
        let stride = body / ncomp;
        if stride * ncomp != body || !(2..=3).contains(&stride) {
          return Err(RawlerError::DecoderFailed(format!(
            "JPEG XS: CDT body {} does not hold 2 or 3 bytes for each of {} components",
            body, ncomp
          )));
        }
        for _ in 0..ncomp {
          let bit_depth = r.u8()?;
          let sub = r.u8()?;
          let (nlx, nly) = if stride == 3 {
            let d = r.u8()?;
            (d >> 4, d & 0x0f)
          } else {
            (frame.nlx, frame.nly)
          };
          components.push(Component {
            bit_depth,
            sx: sub >> 4,
            sy: sub & 0x0f,
            nlx,
            nly,
          });
        }
      }

      marker::WGT => {
        if !body.is_multiple_of(2) {
          return Err(RawlerError::DecoderFailed(format!("JPEG XS: WGT body {} is odd", body)));
        }
        for _ in 0..body / 2 {
          let gain = r.u8()?;
          let priority = r.u8()?;
          weights.push((gain, priority));
        }
      }

      // Markers Nikon does not emit. Skipping keeps the parser useful elsewhere.
      _ => r.skip(body)?,
    }

    if r.pos != body_start + body {
      return Err(RawlerError::DecoderFailed(format!(
        "JPEG XS: marker {:04x} consumed {} bytes, expected {}",
        code,
        r.pos - body_start,
        body
      )));
    }
  }
}

/// Real codestream bytes shared by this module's tests and the topology tests
/// in [`super::pi`].
#[cfg(test)]
pub(crate) mod testdata {
  /// The first 155 bytes of the codestream from a Nikon Z6III HE* file.
  #[rustfmt::skip]
  pub(crate) const SAMPLE: [u8; 155] = [
    0xff, 0x10, 0xff, 0x50, 0x00, 0x22, 0x43, 0x4f, 0x4e, 0x54, 0x41, 0x43,
    0x54, 0x5f, 0x49, 0x4e, 0x54, 0x4f, 0x50, 0x49, 0x58, 0x5f, 0xef, 0xc0,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0x12, 0x00, 0x27, 0x00, 0xe9, 0xa2, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x17, 0xb0, 0x0f, 0xc8, 0x00, 0x00, 0x00, 0x10, 0x04, 0x04,
    0x08, 0x12, 0x44, 0x10, 0x51, 0x14, 0x50, 0x88, 0x70, 0x83, 0xf0, 0x15,
    0x23, 0xd1, 0x49, 0xcd, 0x3f, 0x7f, 0x07, 0xff, 0x13, 0x00, 0x0e, 0x0e,
    0x11, 0x51, 0x0e, 0x11, 0x51, 0x0e, 0x11, 0x00, 0x0e, 0x11, 0x51, 0xff,
    0x14, 0x00, 0x34, 0x03, 0x02, 0x03, 0x13, 0x02, 0x09, 0x02, 0x16, 0x01,
    0x0a, 0x01, 0x15, 0x02, 0x05, 0x01, 0x03, 0x01, 0x11, 0x00, 0x01, 0x00,
    0x07, 0x00, 0x0d, 0x00, 0x0b, 0x02, 0x06, 0x01, 0x04, 0x01, 0x10, 0x00,
    0x00, 0x00, 0x08, 0x00, 0x0e, 0x01, 0x14, 0x00, 0x12, 0x00, 0x0f, 0x00,
    0x17, 0x00, 0x0c, 0x00, 0x18, 0xff, 0x20, 0x00, 0x04, 0x00, 0x00,
  ];
}

#[cfg(test)]
mod tests {
  use super::testdata::SAMPLE;
  use super::*;

  #[test]
  fn parses_a_nikon_he_header() {
    let h = parse(&SAMPLE).expect("header parses");
    assert_eq!(h.lcod, 15_311_360);
    assert_eq!((h.grid_width, h.grid_height), (6064, 4040));
    assert_eq!((h.plane_width(), h.plane_height()), (3032, 2020));
    assert_eq!((h.group_size, h.sig_group_size), (4, 8));
    assert_eq!((h.bw, h.fq, h.br), (18, 4, 4));
    assert_eq!((h.nlx, h.nly), (5, 1));
    assert_eq!(h.ppoc, 1);
    assert_eq!(h.cpih, 0);
    assert_eq!((h.qpih, h.fs, h.rm, h.lh, h.rl), (1, 1, 0, 0, 0));
    assert_eq!((h.precinct_width, h.slice_height), (0, 16));
    assert_eq!(h.first_slice, 149);
  }

  #[test]
  fn reads_the_extended_component_table() {
    let h = parse(&SAMPLE).unwrap();
    assert_eq!(h.components.len(), 4);
    for c in &h.components {
      assert_eq!((c.bit_depth, c.sx, c.sy), (14, 1, 1));
    }
    // Component 2 is the one without a wavelet, not the last one. Stock JPEG XS
    // cannot express this, which is why the extended CDT exists.
    let suppressed: Vec<bool> = h.components.iter().map(|c| c.suppressed()).collect();
    assert_eq!(suppressed, [false, false, true, false]);
    assert_eq!(h.components.iter().filter(|c| c.suppressed()).count(), 1);
    assert_eq!(h.components.iter().map(|c| c.bands()).collect::<Vec<_>>(), [8, 8, 1, 8]);
  }

  #[test]
  fn band_count_matches_the_weights_table() {
    let h = parse(&SAMPLE).unwrap();
    // 3 decomposed components at 8 bands each, plus the suppressed one.
    assert_eq!(h.total_bands(), 25);
    assert_eq!(h.weights.len(), 25);
    // Priorities are a permutation of 0..25.
    let mut prio: Vec<u8> = h.weights.iter().map(|(_, p)| *p).collect();
    prio.sort_unstable();
    assert_eq!(prio, (0..25).collect::<Vec<u8>>());
  }

  #[test]
  fn derives_the_slice_geometry() {
    let h = parse(&SAMPLE).unwrap();
    assert_eq!(h.precinct_height(), 2);
    assert_eq!(h.precinct_count(), 1010);
    assert_eq!(h.slice_count(), 64);
  }

  #[test]
  fn rejects_a_truncated_codestream() {
    assert!(parse(&SAMPLE[..40]).is_err());
    assert!(parse(&[0xff, 0x11]).is_err());
    assert!(parse(&[]).is_err());
  }
}
