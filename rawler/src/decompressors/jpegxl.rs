// SPDX-License-Identifier: LGPL-2.1
// Copyright 2025 Daniel Vogelbacher <daniel@chaospixel.com>

//! JPEG-XL decompressor for DNG strips and tiles.

use jxl_oxide::{JxlImage, image::BitDepth};

use crate::decompressors::{Decompressor, LineIteratorMut};

/// Decompressor for JPEG-XL compressed DNG image data.
///
/// Supports integer codestreams with 8 to 16 bits per sample, decoded to
/// `u16`, and floating point codestreams, decoded to `f32`. Samples are
/// returned in the range of the codestream, they are not scaled.
pub struct JpegXLDecompressor {
  /// Bits per sample as declared in the TIFF IFD (not the JXL stream).
  bps: u32,
}

impl JpegXLDecompressor {
  /// Creates a new `JpegXLDecompressor` for the given TIFF bits-per-sample.
  pub fn new(bps: u32) -> Self {
    Self { bps }
  }
}

impl<'a> Decompressor<'a, u16> for JpegXLDecompressor {
  /// Decodes a JPEG-XL buffer into `u16` pixel lines.
  ///
  /// Renders frame 0 of the JXL image (alpha channel excluded) and writes
  /// samples row by row into `lines`, skipping the first `skip_rows` rows.
  ///
  /// * **8 bps** — samples are decoded to `u8` via a temporary buffer and
  ///   zero-extended to `u16` without scaling.
  /// * **9–16 bps** — samples are decoded to `u16`. jxl-oxide scales them to
  ///   the full `u16` range, so samples of codestreams with less than 16 bits
  ///   are scaled back to the codestream range, which restores the stored values.
  ///
  /// # Errors
  /// Returns `Err` if the JXL image cannot be read or rendered, if it holds
  /// floating point samples or if `bps` is not in the range 8 to 16.
  fn decompress(&self, src: &[u8], mut skip_rows: usize, lines: impl LineIteratorMut<'a, u16>, line_width: usize) -> std::result::Result<(), String> {
    let image = JxlImage::builder()
      .read(src)
      .map_err(|err| format!("Failed to read JPEG-XL image: {:?}", err))?;
    let codestream_bits = match image.image_header().metadata.bit_depth {
      BitDepth::IntegerSample { bits_per_sample } if (8..=16).contains(&bits_per_sample) => bits_per_sample,
      bit_depth => return Err(format!("JPEG-XL bit depth {:?} is not supported", bit_depth)),
    };
    let frame = image.render_frame(0).map_err(|err| format!("Failed to render JPEG-XL image: {:?}", err))?;

    let mut stream = frame.stream_no_alpha();

    match self.bps {
      8 if codestream_bits == 8 => {
        let mut tmp = vec![0_u8; line_width];

        for line in lines.skip(skip_rows) {
          while skip_rows > 0 {
            let written = stream.write_to_buffer(&mut tmp);
            assert_eq!(line.len(), written);
            skip_rows -= 1;
          }
          let written = stream.write_to_buffer(&mut tmp);
          assert_eq!(line.len(), written);
          for (p, x) in line.iter_mut().zip(tmp.iter()) {
            *p = *x as u16; // Only change storage format, you MUST NOT scale up!
          }
        }
      }
      8..=16 => {
        let max = (1_u32 << codestream_bits) - 1;
        for line in lines.skip(skip_rows) {
          while skip_rows > 0 {
            let written = stream.write_to_buffer(line);
            assert_eq!(line.len(), written);
            skip_rows -= 1;
          }
          let written = stream.write_to_buffer(line);
          assert_eq!(line.len(), written);
          if codestream_bits < 16 {
            for p in line.iter_mut() {
              *p = ((*p as u32 * max + 32767) / 65535) as u16;
            }
          }
        }
      }
      bps => return Err(format!("JPEG-XL data with {} bits per sample is not supported", bps)),
    }
    Ok(())
  }

  fn can_skip_rows(&self) -> bool {
    false
  }
}

impl<'a> Decompressor<'a, f32> for JpegXLDecompressor {
  /// Decodes a floating point JPEG-XL buffer into `f32` pixel lines.
  ///
  /// Renders frame 0 of the JXL image (alpha channel excluded) and writes
  /// samples row by row into `lines`, skipping the first `skip_rows` rows.
  /// Samples keep their stored values, e.g. on the scale of the DNG WhiteLevel.
  ///
  /// DNG requires 16-bit floats, but the codestream header may declare another
  /// float format (DNG SDK samples declare 32-bit), so any float format is accepted.
  ///
  /// jxl-oxide decodes half float zeros and subnormals of lossless (modular)
  /// codestreams wrong, for example 0.0 becomes 2^-15.
  ///
  /// # Errors
  /// Returns `Err` if the JXL image cannot be read or rendered, if it holds
  /// integer samples, if `bps` is not 16 or if the image is smaller than `lines`.
  fn decompress(&self, src: &[u8], skip_rows: usize, lines: impl LineIteratorMut<'a, f32>, line_width: usize) -> std::result::Result<(), String> {
    if self.bps != 16 {
      return Err(format!("JPEG-XL floating point data with {} bits per sample is not supported", self.bps));
    }
    let image = JxlImage::builder()
      .read(src)
      .map_err(|err| format!("Failed to read JPEG-XL image: {:?}", err))?;
    let bit_depth = image.image_header().metadata.bit_depth;
    if !matches!(bit_depth, BitDepth::FloatSample { .. }) {
      return Err(format!("JPEG-XL bit depth {:?} is not a floating point format", bit_depth));
    }
    let frame = image.render_frame(0).map_err(|err| format!("Failed to render JPEG-XL image: {:?}", err))?;

    let mut stream = frame.stream_no_alpha();
    if skip_rows > 0 {
      let mut tmp = vec![0.0_f32; line_width];
      for _ in 0..skip_rows {
        stream.write_to_buffer(&mut tmp);
      }
    }
    for line in lines {
      let written = stream.write_to_buffer(line);
      if written != line.len() {
        return Err(format!("JPEG-XL image returned {} samples for a line of {} samples", written, line.len()));
      }
    }
    Ok(())
  }

  fn can_skip_rows(&self) -> bool {
    false
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Lossless 8x4 gray codestream with 12 bits per sample, encoded by cjxl from [`samples_12bit`]
  const JXL_12BIT: &[u8] = &[
    0xff, 0x0a, 0x18, 0x70, 0x18, 0x14, 0x37, 0x02, 0x08, 0x04, 0x01, 0x00, 0xdc, 0x00, 0x4b, 0x38, 0x6a, 0x18, 0xd2, 0xd3, 0xbf, 0x2a, 0x28, 0x48, 0x8a, 0x2d,
    0x82, 0x1b, 0xb0, 0xfe, 0x76, 0x62, 0x86, 0x22, 0x8a, 0x28, 0x56, 0x17, 0x45, 0x14, 0xab, 0x2b, 0x15, 0x14, 0xab, 0x3b, 0xa9, 0x54, 0x77, 0x52, 0xf9, 0xea,
    0xa4, 0x52, 0xdd, 0x49, 0x05, 0xc5, 0xea, 0x4a, 0x05, 0xc5, 0xea, 0x4a, 0x05, 0xc5, 0xea, 0x4e, 0xc7,
  ];

  /// Samples of [`JXL_12BIT`], including the highest 12 bit value
  fn samples_12bit() -> Vec<u16> {
    let mut samples: Vec<u16> = (0..32).map(|i| (i * 1301 % 4096) as u16).collect();
    samples[31] = 4095;
    samples
  }

  /// Lossless 8x4 RGB codestream with linear 32-bit float samples (Rec. 2100 primaries),
  /// encoded by cjxl from [`samples_float`]. cjxl puts it into a container, as 32-bit
  /// floats require codestream level 10.
  const JXL_FLOAT_LOSSLESS: &[u8] = &[
    0x00, 0x00, 0x00, 0x0c, 0x4a, 0x58, 0x4c, 0x20, 0x0d, 0x0a, 0x87, 0x0a, 0x00, 0x00, 0x00, 0x14, 0x66, 0x74, 0x79, 0x70, 0x6a, 0x78, 0x6c, 0x20, 0x00, 0x00,
    0x00, 0x00, 0x6a, 0x78, 0x6c, 0x20, 0x00, 0x00, 0x00, 0x09, 0x6a, 0x78, 0x6c, 0x6c, 0x0a, 0x00, 0x00, 0x00, 0xa3, 0x6a, 0x78, 0x6c, 0x63, 0xff, 0x0a, 0x18,
    0x70, 0x72, 0x00, 0x79, 0xb4, 0x08, 0x08, 0x04, 0x01, 0x00, 0x30, 0x02, 0x4b, 0x18, 0x93, 0x8e, 0x83, 0x83, 0x99, 0x04, 0xc5, 0xc6, 0x3f, 0x74, 0x02, 0x9e,
    0x7c, 0x94, 0xc1, 0x81, 0x6b, 0x38, 0x3c, 0x04, 0x39, 0x36, 0x0f, 0x00, 0x40, 0x16, 0x07, 0x00, 0xc0, 0x05, 0x00, 0x30, 0x00, 0x00, 0x03, 0x00, 0x30, 0x00,
    0x00, 0x03, 0x00, 0x30, 0x00, 0x00, 0x03, 0x00, 0x30, 0x00, 0x80, 0x00, 0x00, 0x04, 0x00, 0x10, 0x00, 0x40, 0x00, 0x00, 0x01, 0x00, 0x04, 0x00, 0x30, 0x00,
    0x00, 0x01, 0x00, 0x04, 0x00, 0x10, 0x00, 0x40, 0x00, 0x00, 0x01, 0x00, 0x04, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0x02, 0x00, 0x10, 0x00, 0x40, 0x00, 0x00,
    0x01, 0x00, 0x04, 0x00, 0x10, 0x00, 0xc0, 0x01, 0x00, 0x1c, 0x00, 0x00, 0xf2, 0x04, 0x00, 0x30, 0x00, 0x80, 0x01, 0x00, 0x0c, 0x00, 0x60, 0x00, 0x00, 0x03,
    0x00, 0x18, 0x00, 0x40, 0x00, 0x00, 0x80, 0x01, 0x00, 0x84, 0x11, 0xa0, 0xff, 0x7f, 0x80, 0xfe, 0xff, 0x01, 0xfa, 0xff, 0x07, 0x00,
  ];

  /// Samples of [`JXL_FLOAT_LOSSLESS`], far above the nominal range of 0.0 .. 1.0
  fn samples_float() -> Vec<f32> {
    (0..4)
      .flat_map(|y| (0..8).flat_map(move |x| [100 + 20 * x + 10 * y, 400 + 15 * x, 900 - 30 * y]))
      .map(|v| v as f32)
      .collect()
  }

  #[test]
  fn codestream_with_less_than_16_bits_keeps_stored_values() -> std::result::Result<(), String> {
    let mut out = vec![0_u16; 8 * 4];
    JpegXLDecompressor::new(12).decompress(JXL_12BIT, 0, out.chunks_exact_mut(8), 8)?;
    assert_eq!(out, samples_12bit());
    Ok(())
  }

  #[test]
  fn float_codestream_keeps_stored_values() -> std::result::Result<(), String> {
    let mut out = vec![0_f32; 8 * 4 * 3];
    JpegXLDecompressor::new(16).decompress(JXL_FLOAT_LOSSLESS, 0, out.chunks_exact_mut(8 * 3), 8 * 3)?;
    assert_eq!(out, samples_float());
    Ok(())
  }

  #[test]
  fn float_decoding_rejects_integer_codestream() {
    let mut out = vec![0_f32; 8 * 4];
    assert!(JpegXLDecompressor::new(16).decompress(JXL_12BIT, 0, out.chunks_exact_mut(8), 8).is_err());
  }
}
