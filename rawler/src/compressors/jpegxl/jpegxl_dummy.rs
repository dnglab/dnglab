// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! JPEG XL encoder for DNG tiles.
//!
//! The encoder uses the libjxl of the system, which is loaded at runtime. Nothing is
//! required at build time; if libjxl is missing or too old, encoding fails with an
//! error. Set the environment variable `RAWLER_LIBJXL` to load libjxl from a
//! specific path.

use super::types::*;

const JPEGXL_NOT_SUPPORTED: &str = "jpegxl encoding not supported on this platform";

pub fn libjxl_version() -> Result<(u32, u32, u32), String> {
    Err(JPEGXL_NOT_SUPPORTED.into())
}

pub fn encode_tile(_samples: TileSamples<'_>, _width: usize, _height: usize, _channels: usize, _settings: &TileSettings) -> Result<Vec<u8>, String> {
    Err(JPEGXL_NOT_SUPPORTED.into())
}
