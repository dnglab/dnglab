// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026

//! JPEG XS (ISO/IEC 21122) decoding, as used by Nikon "High Efficiency" NEF.
//!
//! Nikon's HE and HE* modes store the CFA payload as a JPEG XS codestream
//! with a handful of vendor deviations. The encoder looks like intoPIX
//! TicoRAW: the `CAP` marker reads `CONTACT_INTOPIX_`. HE and HE* are the
//! same structure and differ only in rate.
//!
//! Four things are worth knowing before reading the submodules:
//!
//! * the four components are a star-tetrix decorrelation of the Bayer planes,
//!   `(Y, Cr, Δ, Cb)`, not the planes themselves;
//! * each component plane is half the CFA grid in both axes, even though the
//!   subsampling factors read as 1;
//! * exactly one component skips the wavelet, and it is index 2 rather than
//!   the last one;
//! * packets within a precinct are ordered subband-major, and the weights
//!   table is stored in that same order.

pub mod bitreader;
pub mod dwt;
pub mod entropy;
pub mod header;
pub mod mct;
pub mod pi;

use crate::RawlerError;
use crate::Result;

/// Finds the data the cross-check tests compare against, which is the output
/// of the patched SVT-JPEG-XS decoder and too large to check in. Point
/// `RAWLER_JPEGXS_REFERENCE` at a directory holding `he_star.jxs`,
/// `precinct0.txt` and `ref_final.planes` to run them; [`entropy`] and
/// [`mct`] say how to regenerate those files.
///
/// With the variable unset the tests skip. With it pointed at an incomplete
/// directory they fail. They cannot pass by accident.
#[cfg(test)]
pub(crate) mod reference {
  use std::path::PathBuf;

  pub fn dir(test: &str) -> Option<PathBuf> {
    let dir = std::env::var_os("RAWLER_JPEGXS_REFERENCE");
    if dir.is_none() {
      eprintln!("skipping {}: set RAWLER_JPEGXS_REFERENCE to the reference data directory", test);
    }
    dir.map(PathBuf::from)
  }

  pub fn read(dir: &PathBuf, name: &str) -> Vec<u8> {
    std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("RAWLER_JPEGXS_REFERENCE is set but {} is unreadable: {}", name, e))
  }
}

/// Decode a Nikon HE/HE* codestream into its four Bayer quad planes `(R, G1,
/// G2, B)`, each half the CFA grid in both axes, as 14-bit sensor values.
pub fn decode_planes(stream: &[u8]) -> Result<(header::Header, Vec<Vec<u16>>)> {
  let header = header::parse(stream)?;
  let topology = pi::build(&header)?;
  let mut idwt = dwt::Idwt::new(&header, &topology)?;

  let mut offset = header.first_slice;
  let mut remaining = header.precinct_count();
  for slice in 0..header.slice_count() {
    if stream.get(offset..offset + 2) != Some(&[0xff, 0x20]) {
      return Err(RawlerError::DecoderFailed(format!("JPEG XS: no SLH marker at slice {}", slice)));
    }
    offset += 6;
    // Vertical prediction chains through the slice and resets at its start.
    let mut top: Option<entropy::Precinct> = None;
    for _ in 0..remaining.min(header.slice_height as usize) {
      let (precinct, next) = entropy::decode_precinct(stream, offset, &header, &topology, top.as_ref())?;
      offset = next;
      idwt.push_precinct(&precinct)?;
      top = Some(precinct);
      remaining -= 1;
    }
  }
  if stream.get(offset..offset + 2) != Some(&[0xff, 0x11]) {
    return Err(RawlerError::DecoderFailed("JPEG XS: no EOC marker after the last slice".into()));
  }

  let planes = mct::transform(idwt.finish()?, &header)?;
  Ok((header, planes))
}

/// Decode a Nikon HE/HE* codestream into the full-resolution RGGB CFA
/// mosaic. Returns the samples with their width and height, which match the
/// `Wf` x `Hf` grid the picture header declares.
pub fn decode(stream: &[u8]) -> Result<(Vec<u16>, usize, usize)> {
  let (header, planes) = decode_planes(stream)?;
  let (w, h) = (header.plane_width(), header.plane_height());
  let mosaic = mct::interleave(&planes, w, h)?;
  Ok((mosaic, 2 * w, 2 * h))
}
