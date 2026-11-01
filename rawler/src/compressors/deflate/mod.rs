// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Jess Olmstead <jess@jessolmstead.com>

//! Deflate compression of floating point strips for the DNG writer.
//!
//! The samples are byte-split and differenced with the TIFF floating point
//! predictor, then compressed into a zlib stream.

use std::io::{self, Write};

use libflate::zlib::Encoder;

/// Encode a strip of floating point samples with the floating point predictor
/// and compress it with zlib
///
/// The bytes of each row are split into planes, most significant bytes first,
/// and each byte is stored as the difference to the byte `cpp` positions
/// to its left (TIFF predictor 3). This is the inverse of
/// [`DeflateDecompressor`](crate::decompressors::deflate::DeflateDecompressor).
pub fn encode_float_strip(strip: &[f32], row_len: usize, cpp: usize, bits: u16) -> io::Result<Vec<u8>> {
  let bytes = bits as usize / 8;
  let mut buf = vec![0_u8; strip.len() * bytes];
  for (row, out) in strip.chunks_exact(row_len).zip(buf.chunks_exact_mut(row_len * bytes)) {
    for (col, v) in row.iter().enumerate() {
      let value = match bits {
        16 => (half::f16::from_f32(*v).to_bits() as u32) << 16,
        _ => v.to_bits(),
      };
      for (b, byte) in value.to_be_bytes()[..bytes].iter().enumerate() {
        out[b * row_len + col] = *byte;
      }
    }
    for i in (cpp..out.len()).rev() {
      out[i] = out[i].wrapping_sub(out[i - cpp]);
    }
  }
  let mut encoder = Encoder::new(Vec::new())?;
  encoder.write_all(&buf)?;
  encoder.finish().into_result()
}
