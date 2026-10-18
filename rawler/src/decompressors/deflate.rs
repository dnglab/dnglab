//! Deflate (zlib) decompressors for DNG image data.
//!
//! [`DeflateDecompressor`] handles IEEE floating-point data, stored in tiles
//! or strips. With the floating point predictor (TIFF predictor 3, 34894,
//! or 34895), there are two layers of encoding on top of the raw sample bytes:
//!
//! 1. **Byte-plane shuffling** — the bytes of each multi-byte sample are split
//!    across separate planes within the compressed block. For a 32-bit float
//!    tile of *W* pixels the block is laid out as *W* MSBs, then *W* second
//!    bytes, and so on. This improves compressibility by grouping bytes with
//!    similar entropy together.
//!
//! 2. **Horizontal delta prediction** — each byte is stored as the difference
//!    from the byte `pred_factor` positions to its left, where
//!    `pred_factor = cpp × bytes_per_component`. The decoder reverses this
//!    with a running sum before reassembling samples.
//!
//! Without predictor (TIFF predictor 1), the samples are stored unchanged in
//! the byte order of the TIFF file.
//!
//! Finally, the byte stream is compressed with zlib (identified by
//! `CompressionMethod::Deflate` in the TIFF IFD).
//!
//! Supported bit depths: 16 (half-float), 24 (24-bit float), and 32
//! (single-precision float). All depths are widened to `f32` on output.
//!
//! [`DeflateIntDecompressor`] handles 32-bit unsigned integer data.

use libflate::zlib::Decoder;
use std::io::Read;

use crate::{
  bits::{Binary16, Binary24, Binary32, Endian, FloatingPointParameters, extend_binary_floating_point},
  decompressors::{Decompressor, LineIteratorMut, packed::PackedDecompressor},
};

/// Decompressor for Deflate-compressed floating-point DNG image data.
///
/// Constructed once per tile or strip.
#[derive(Debug)]
pub struct DeflateDecompressor {
  /// Horizontal delta stride in bytes for the floating point predictor:
  /// `cpp × bytes_per_component`, or `None` if no predictor is used.
  ///
  /// Determines how far back the delta predictor looks when decoding each
  /// byte. Equals the number of channels times the byte-width of the
  /// predictor unit selected by the TIFF `Predictor` tag.
  pred_factor: Option<usize>,
  /// Bits per sample (16, 24, or 32).
  bps: u32,
  /// Byte order of the samples if no predictor is used
  endian: Endian,
}

impl DeflateDecompressor {
  /// Creates a new `DeflateDecompressor` from IFD metadata.
  ///
  /// # Arguments
  /// * `cpp` — Channels (samples) per pixel, e.g. 1 for greyscale, 3 for RGB.
  /// * `predictor` — Value of the TIFF `Predictor` tag:
  ///   - `1` — No predictor.
  ///   - `3` — Floating point predictor with a 1-byte component stride.
  ///   - `34894` — DNG floating-point predictor with a 2-byte component stride.
  ///   - `34895` — DNG floating-point predictor with a 4-byte component stride.
  /// * `bps` — Bits per sample stored in the compressed stream (16, 24, or 32).
  /// * `endian` — Byte order of the samples (IFD byte order), only used without predictor.
  ///
  /// # Errors
  /// Returns `Err` for any other `predictor` or `bps` value.
  pub fn new(cpp: usize, predictor: u16, bps: u32, endian: Endian) -> std::result::Result<Self, String> {
    let pred_factor = match predictor {
      1 => None,
      3 => Some(cpp),
      34894 => Some(2 * cpp),
      34895 => Some(4 * cpp),
      _ => return Err(format!("DeflateDecompressor: unsupported predictor {predictor}")),
    };
    if ![16, 24, 32].contains(&bps) {
      return Err(format!("DeflateDecompressor: unsupported bit depth: {bps} bps"));
    }
    Ok(Self { pred_factor, bps, endian })
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
  /// length does not match `bps/8 × line_width × (skip_rows + lines.len())`.
  fn decompress(&self, src: &[u8], skip_rows: usize, lines: impl LineIteratorMut<'a, f32>, line_width: usize) -> std::result::Result<(), String> {
    let mut decoder = Decoder::new(src).map_err(|err| err.to_string())?;
    let mut decoded_data = Vec::new();
    decoder.read_to_end(&mut decoded_data).map_err(|err| err.to_string())?;

    let row_bytes = self.bps as usize / 8 * line_width;
    if decoded_data.len() != row_bytes * (skip_rows + lines.len()) {
      return Err("DeflateDecompressor: buffer length mismatch".to_string());
    }

    let Some(pred_factor) = self.pred_factor else {
      // No predictor, samples are stored like uncompressed data
      return PackedDecompressor::new(self.bps, self.endian).decompress(&decoded_data, skip_rows, lines, line_width);
    };

    for (line, row) in lines.zip(decoded_data.chunks_exact_mut(row_bytes).skip(skip_rows)) {
      debug_assert_eq!(line.len(), line_width);
      decode_delta_bytes(row, pred_factor);

      match self.bps {
        16 => decode_fp_delta_row::<Binary16>(line, row, line_width),
        24 => decode_fp_delta_row::<Binary24>(line, row, line_width),
        _ => decode_fp_delta_row::<Binary32>(line, row, line_width),
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
    // 32-bit values are already f32, extending would fail for subnormal values
    *pix = if NARROW::STORAGE_WIDTH == Binary32::STORAGE_WIDTH {
      f32::from_bits(value)
    } else {
      f32::from_bits(extend_binary_floating_point::<NARROW, Binary32>(value))
    };
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

  fn zlib(data: &[u8]) -> Vec<u8> {
    let mut encoder = Encoder::new(Vec::new()).unwrap();
    encoder.write_all(data).unwrap();
    encoder.finish().into_result().unwrap()
  }

  fn decode_float(src: &[u8], cpp: usize, predictor: u16, bps: u32, endian: Endian, width: usize, height: usize) -> std::result::Result<Vec<f32>, String> {
    let mut out = vec![0.0_f32; width * height];
    DeflateDecompressor::new(cpp, predictor, bps, endian)?.decompress(src, 0, out.chunks_exact_mut(width), width)?;
    Ok(out)
  }

  /// Apply the floating point predictor (byte-plane shuffle and byte delta) to f32 rows
  fn encode_fp_predictor(samples: &[f32], width: usize, pred_factor: usize) -> Vec<u8> {
    let mut data = Vec::new();
    for row in samples.chunks_exact(width) {
      let mut bytes = vec![0_u8; width * 4];
      for (col, v) in row.iter().enumerate() {
        for (b, byte) in v.to_be_bytes().into_iter().enumerate() {
          bytes[b * width + col] = byte;
        }
      }
      for i in (pred_factor..bytes.len()).rev() {
        bytes[i] = bytes[i].wrapping_sub(bytes[i - pred_factor]);
      }
      data.extend_from_slice(&bytes);
    }
    zlib(&data)
  }

  #[test]
  fn deflate_float_no_predictor() -> std::result::Result<(), String> {
    let samples = [0.0_f32, 0.25, 1.0, 1234.5, -2.0, 0.001];
    let le: Vec<u8> = samples.iter().flat_map(|v| v.to_le_bytes()).collect();
    let be: Vec<u8> = samples.iter().flat_map(|v| v.to_be_bytes()).collect();
    assert_eq!(decode_float(&zlib(&le), 1, 1, 32, Endian::Little, 3, 2)?, samples);
    assert_eq!(decode_float(&zlib(&be), 1, 1, 32, Endian::Big, 3, 2)?, samples);

    // Half float: 0.0, 0.5, 1.0, 2.0
    let half: Vec<u8> = [0x0000_u16, 0x3800, 0x3C00, 0x4000].iter().flat_map(|v| v.to_le_bytes()).collect();
    assert_eq!(decode_float(&zlib(&half), 1, 1, 16, Endian::Little, 2, 2)?, [0.0, 0.5, 1.0, 2.0]);
    Ok(())
  }

  #[test]
  fn deflate_float_fp_predictor() -> std::result::Result<(), String> {
    let width = 8;
    let mut samples: Vec<f32> = (0..width * 3).map(|i| i as f32 * 0.37 - 2.0).collect();
    samples[5] = f32::from_bits(1); // Subnormal value
    // Predictor 3/34894/34895 compares bytes 1/2/4 samples to the left
    for (cpp, predictor, mult) in [(1, 3, 1), (2, 3, 1), (1, 34894, 2), (2, 34894, 2), (1, 34895, 4)] {
      let src = encode_fp_predictor(&samples, width, cpp * mult);
      assert_eq!(
        decode_float(&src, cpp, predictor, 32, Endian::Little, width, 3)?,
        samples,
        "cpp {cpp}, predictor {predictor}"
      );
    }
    Ok(())
  }

  #[test]
  fn deflate_float_rejects_invalid_parameters() {
    assert!(DeflateDecompressor::new(1, 2, 32, Endian::Little).is_err());
    assert!(DeflateDecompressor::new(1, 7, 32, Endian::Little).is_err());
    assert!(DeflateDecompressor::new(1, 3, 8, Endian::Little).is_err());
  }

  #[test]
  fn deflate_int_rejects_unknown_predictor() {
    assert!(DeflateIntDecompressor::new(1, 3, Endian::Little).is_err());
  }
}
