// SPDX-License-Identifier: LGPL-2.1
// Copyright 2025 Daniel Vogelbacher <daniel@chaospixel.com>

//! Safe Rust wrapper around the C helper that calls libjxl for encoding.

use crate::dng::writer::DngError;

unsafe extern "C" {
  fn rawler_jxl_encode(
    pixels: *const u16,
    width: u32,
    height: u32,
    num_channels: u32,
    bits_per_sample: u32,
    distance: f32,
    effort: u32,
    output: *mut *mut u8,
    output_size: *mut usize,
  ) -> i32;

  fn rawler_jxl_free(ptr: *mut u8);
}

/// Encode a u16 raster tile into a bare JXL bitstream.
///
/// * `pixels`         – interleaved u16 samples (native endian)
/// * `width`/`height` – tile dimensions in pixels
/// * `cpp`            – channels per pixel (1 or 3)
/// * `bps`            – stored bits per sample (8–16)
/// * `distance`       – JXL butterfly distance (0.0 = lossless, 1.0 ≈ visually lossless)
/// * `effort`         – encoder effort 1–10 (default 7)
pub fn encode_jxl_tile(pixels: &[u16], width: u32, height: u32, cpp: u32, bps: u32, distance: f32, effort: u32) -> Result<Vec<u8>, DngError> {
  let mut out_ptr: *mut u8 = std::ptr::null_mut();
  let mut out_len: usize = 0;

  let rc = unsafe {
    rawler_jxl_encode(
      pixels.as_ptr(),
      width,
      height,
      cpp,
      bps,
      distance,
      effort,
      &mut out_ptr,
      &mut out_len,
    )
  };

  if rc != 0 {
    return Err(DngError::General(format!("JXL encoding failed (error code {})", rc)));
  }

  let bytes = unsafe { std::slice::from_raw_parts(out_ptr, out_len).to_vec() };
  unsafe { rawler_jxl_free(out_ptr) };

  Ok(bytes)
}
