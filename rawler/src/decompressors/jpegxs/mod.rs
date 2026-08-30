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
pub mod entropy;
pub mod header;
pub mod pi;

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

