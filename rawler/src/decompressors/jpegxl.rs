// SPDX-License-Identifier: LGPL-2.1
// Copyright 2025 Daniel Vogelbacher <daniel@chaospixel.com>

//! JPEG-XL decompressor for DNG strips and tiles.

use jxl_oxide::{JxlImage, image::BitDepth};

use crate::decompressors::{Decompressor, LineIteratorMut};

/// Decompressor for JPEG-XL compressed DNG image data.
///
/// Supports integer codestreams with 8 to 16 bits per sample. Samples are
/// returned in the range of the codestream bit depth.
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

  #[test]
  fn codestream_with_less_than_16_bits_keeps_stored_values() -> std::result::Result<(), String> {
    let mut out = vec![0_u16; 8 * 4];
    JpegXLDecompressor::new(12).decompress(JXL_12BIT, 0, out.chunks_exact_mut(8), 8)?;
    assert_eq!(out, samples_12bit());
    Ok(())
  }
}
