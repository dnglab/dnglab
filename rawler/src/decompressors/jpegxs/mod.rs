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

