//! Deflate (zlib) decompressor for floating-point DNG image data.
//!
//! DNG tiles and strips that carry IEEE floating-point pixel data are stored
//! with two layers of encoding on top of the raw sample bytes:
//!
//! 1. **Byte-plane shuffling** — the bytes of each multi-byte sample are split
//!    across separate planes within the compressed block. For a 32-bit float
//!    tile of *W* pixels the block is laid out as *W* MSBs, then *W* second
//!    bytes, and so on. This improves compressibility by grouping bytes with
//!    similar entropy together.
//!
//! 2. **Horizontal delta prediction** (TIFF predictor 3, 34894, or 34895) —
//!    each byte is stored as the difference from the byte `pred_factor`
//!    positions to its left, where `pred_factor = cpp × bytes_per_component`.
//!    The decoder reverses this with a running sum before reassembling samples.
//!
//! 3. **zlib / Deflate compression** — the predicted byte stream is then
//!    compressed with zlib (identified by `CompressionMethod::Deflate` in the
//!    TIFF IFD).
//!
//! Supported bit depths: 16 (half-float), 24 (24-bit float), and 32
//! (single-precision float). All depths are widened to `f32` on output.

use libflate::zlib::Decoder;
use std::io::Read;

use crate::{
  bits::{Binary16, Binary24, Binary32, Endian, FloatingPointParameters, extend_binary_floating_point},
  decompressors::{Decompressor, LineIteratorMut},
};

/// Decompressor for Deflate-compressed floating-point DNG image data.
///
/// Constructed once per tile or strip.
#[derive(Debug)]
pub struct DeflateDecompressor {
  /// Horizontal delta stride in bytes: `cpp × bytes_per_component`.
  ///
  /// Determines how far back the delta predictor looks when decoding each
  /// byte. Equals the number of channels times the byte-width of the
  /// predictor unit selected by the TIFF `Predictor` tag.
  pred_factor: usize,
  /// Bits per sample (16, 24, or 32).
  bps: u32,
}

impl DeflateDecompressor {
  /// Creates a new `DeflateDecompressor` from IFD metadata.
  ///
  /// # Arguments
  /// * `cpp` — Channels (samples) per pixel, e.g. 1 for greyscale, 3 for RGB.
  /// * `predictor` — Value of the TIFF `Predictor` tag:
  ///   - `3` — Horizontal differencing with a 1-byte component stride.
  ///   - `34894` — DNG floating-point predictor with a 2-byte component stride.
  ///   - `34895` — DNG floating-point predictor with a 4-byte component stride.
  /// * `bps` — Bits per sample stored in the compressed stream (16, 24, or 32).
  ///
  /// # Panics
  /// Panics for any `predictor` value other than 3, 34894, and 34895
  pub fn new(cpp: usize, predictor: u16, bps: u32) -> Self {
    let pred_factor = cpp
      * match predictor {
        3 => 1,
        34894 => 2,
        34895 => 4,
        _ => panic!("DeflateDecompressor: Unsupported predictor {predictor}"),
      };
    Self { pred_factor, bps }
  }
}

impl<'a> Decompressor<'a, f32> for DeflateDecompressor {
  /// Decompresses a single zlib-compressed tile or strip into `f32` pixel lines.
  ///
  /// # Arguments
  /// * `src` — Zlib-compressed tile or strip bytes.
  /// * `skip_rows` — Number of leading rows to skip in the inflated output
  ///   (passed through from the strip/tile dispatch layer).
  /// * `lines` — Mutable iterator over destination `f32` rows.
  /// * `line_width` — Width of each line in pixels (not bytes).
  ///
  /// # Errors
  /// Returns `Err(String)` if zlib inflation fails or if the inflated data
  /// length does not match `bps/8 × line_width × lines.len()`.
  ///
  /// # Panics
  /// Panics if `bps` is not 16, 24, or 32.
  fn decompress(&self, src: &[u8], skip_rows: usize, lines: impl LineIteratorMut<'a, f32>, line_width: usize) -> std::result::Result<(), String> {
    let mut decoder = Decoder::new(src).map_err(|err| err.to_string())?;
    let mut decoded_data = Vec::new();
    decoder.read_to_end(&mut decoded_data).map_err(|err| err.to_string())?;

    let bytesps = self.bps as usize / 8;
    assert!(bytesps >= 2 && bytesps <= 4);
    if decoded_data.len() != bytesps * line_width * lines.len() {
      return Err(format!("DeflateDecompressor: buffer length mismatch"));
    }

    for (line, row) in lines.zip(decoded_data.chunks_exact_mut(bytesps as usize * line_width)).skip(skip_rows) {
      debug_assert_eq!(line.len(), line_width);
      decode_delta_bytes(row, self.pred_factor);

      match self.bps {
        16 => decode_fp_delta_row::<Binary16>(line, row, line_width),
        24 => decode_fp_delta_row::<Binary24>(line, row, line_width),
        32 => decode_fp_delta_row::<Binary32>(line, row, line_width),
        _ => panic!("DeflateDecompressor: bps {} not supported", self.bps),
      }
    }
    Ok(())
  }

  fn can_skip_rows(&self) -> bool {
    false
  }
}

fn decode_delta_bytes(src: &mut [u8], factor: usize) {
  for col in factor..src.len() {
    src[col] = src[col].wrapping_add(src[col - factor]);
  }
}

fn decode_fp_delta_row<NARROW: FloatingPointParameters>(line: &mut [f32], row: &[u8], line_width: usize) {
  for (col, pix) in line.iter_mut().enumerate() {
    let mut tmp = [0; 4];
    debug_assert!(NARROW::STORAGE_BYTES <= tmp.len());

    for c in 0..NARROW::STORAGE_BYTES {
      tmp[c] = row[col + c * line_width];
    }
    let value = u32::from_be_bytes(tmp) >> (u32::BITS as usize - NARROW::STORAGE_WIDTH);
    *pix = f32::from_bits(extend_binary_floating_point::<NARROW, Binary32>(value));
  }
}

/// Decompressor for Deflate-compressed 32-bit unsigned integer DNG image data.
///
/// Samples are returned as `f32` holding the raw integer values (not
/// normalized), as `u16` storage is not sufficient.
#[derive(Debug)]
pub struct DeflateIntDecompressor {
  /// Horizontal difference stride in samples, 0 if no predictor is used
  stride: usize,
  endian: Endian,
}

impl DeflateIntDecompressor {
  /// Creates a new `DeflateIntDecompressor` from IFD metadata.
  ///
  /// # Arguments
  /// * `cpp` — Channels (samples) per pixel.
  /// * `predictor` — Value of the TIFF `Predictor` tag: `1` (none), `2` (horizontal
  ///   difference), `34892` (horizontal difference X2) or `34893` (horizontal difference X4).
  /// * `endian` — Byte order of the samples (IFD byte order).
  pub fn new(cpp: usize, predictor: u16, endian: Endian) -> std::result::Result<Self, String> {
    let stride = match predictor {
      1 => 0,
      2 => cpp,
      34892 => 2 * cpp,
      34893 => 4 * cpp,
      _ => return Err(format!("DeflateIntDecompressor: unsupported predictor {predictor}")),
    };
    Ok(Self { stride, endian })
  }
}

impl<'a> Decompressor<'a, f32> for DeflateIntDecompressor {
  fn decompress(&self, src: &[u8], skip_rows: usize, lines: impl LineIteratorMut<'a, f32>, line_width: usize) -> std::result::Result<(), String> {
    let mut decoder = Decoder::new(src).map_err(|err| err.to_string())?;
    let mut decoded_data = Vec::new();
    decoder.read_to_end(&mut decoded_data).map_err(|err| err.to_string())?;

    let row_bytes = size_of::<u32>() * line_width;
    if decoded_data.len() != row_bytes * (skip_rows + lines.len()) {
      return Err("DeflateIntDecompressor: buffer length mismatch".to_string());
    }

    let mut samples = vec![0_u32; line_width];
    for (line, row) in lines.zip(decoded_data.chunks_exact(row_bytes).skip(skip_rows)) {
      for (sample, bytes) in samples.iter_mut().zip(row.as_chunks::<4>().0) {
        *sample = match self.endian {
          Endian::Big => u32::from_be_bytes(*bytes),
          Endian::Little => u32::from_le_bytes(*bytes),
        };
      }
      if self.stride > 0 {
        for col in self.stride..samples.len() {
          samples[col] = samples[col].wrapping_add(samples[col - self.stride]);
        }
      }
      line.iter_mut().zip(samples.iter()).for_each(|(p, s)| *p = *s as f32);
    }
    Ok(())
  }

  fn can_skip_rows(&self) -> bool {
    false
  }
}

#[cfg(test)]
mod tests {
  use std::io::Write;

  use libflate::zlib::Encoder;

  use super::*;

  /// Apply horizontal differencing with `stride` and compress
  fn encode(samples: &[u32], width: usize, stride: usize) -> Vec<u8> {
    let mut data = Vec::new();
    for row in samples.chunks_exact(width) {
      for col in 0..width {
        let v = if stride > 0 && col >= stride {
          row[col].wrapping_sub(row[col - stride])
        } else {
          row[col]
        };
        data.extend_from_slice(&v.to_le_bytes());
      }
    }
    let mut encoder = Encoder::new(Vec::new()).unwrap();
    encoder.write_all(&data).unwrap();
    encoder.finish().into_result().unwrap()
  }

  #[test]
  fn deflate_int_predictors() -> std::result::Result<(), String> {
    let width = 8;
    let samples: Vec<u32> = (0..width as u32 * 2).map(|i| i.wrapping_mul(0x1234_5679)).collect();
    let expected: Vec<f32> = samples.iter().map(|v| *v as f32).collect();
    for (predictor, stride) in [(1, 0), (2, 1), (34892, 2), (34893, 4)] {
      let src = encode(&samples, width, stride);
      let mut out = vec![0.0_f32; samples.len()];
      DeflateIntDecompressor::new(1, predictor, Endian::Little)?.decompress(&src, 0, out.chunks_exact_mut(width), width)?;
      assert_eq!(out, expected, "predictor {predictor}");
    }
    Ok(())
  }

  #[test]
  fn deflate_int_rejects_unknown_predictor() {
    assert!(DeflateIntDecompressor::new(1, 3, Endian::Little).is_err());
  }
}
