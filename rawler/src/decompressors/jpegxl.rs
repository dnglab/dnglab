// SPDX-License-Identifier: LGPL-2.1
// Copyright 2025 Daniel Vogelbacher <daniel@chaospixel.com>

//! JPEG-XL decompressor for DNG strips and tiles.

use jxl_oxide::JxlImage;

use crate::decompressors::{Decompressor, LineIteratorMut};

/// Decompressor for JPEG-XL compressed DNG image data.
///
/// Only 8-bit and 16-bit JPEG-XL streams are currently supported. Other
/// bit depths are rejected because `jxl_oxide` scales samples to the full
/// storage-type range on decode, which would break black-level calculations
/// without a rescaling step that is not yet implemented.
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
  /// * **9–16 bps** — samples are written directly into the `u16` output.
  ///
  /// # Errors
  /// Returns `Err` if the JXL image cannot be read or rendered.
  ///
  /// # Panics
  /// Panics if the JXL stream's internal bit depth is not 8 or 16, or if
  /// `bps` is outside the ranges 8 and 9–16.
  fn decompress(&self, src: &[u8], mut skip_rows: usize, lines: impl LineIteratorMut<'a, u16>, line_width: usize) -> std::result::Result<(), String> {
    let image = JxlImage::builder()
      .read(src)
      .map_err(|err| format!("Failed to read JPEG-XL image: {:?}", err))?;
    if let Some(header) = image.frame_header(0) {
      if header.bit_depth.bits_per_sample() != 8 && header.bit_depth.bits_per_sample() != 16 {
        // jxl_oxide scales the pixels into full range of storage type.
        // If we get e.g. 12 bit compressed data, output is scaled to 16 bit.
        // This breaks blacklevel scaling. We need to scale back to given bps (from TIFF).
        unimplemented!("JPEG-XL bit-depth {} not supported yet", header.bit_depth.bits_per_sample());
      }
      //eprintln!("JPEG-XL Bit-Depth: {:?}", header.bit_depth);
    }
    let frame = image.render_frame(0).map_err(|err| format!("Failed to render JPEG-XL image: {:?}", err))?;

    let mut stream = frame.stream_no_alpha();

    match self.bps {
      8 => {
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
      9..=16 => {
        for line in lines.skip(skip_rows) {
          while skip_rows > 0 {
            let written = stream.write_to_buffer(line);
            assert_eq!(line.len(), written);
            skip_rows -= 1;
          }
          let written = stream.write_to_buffer(line);
          assert_eq!(line.len(), written);
        }
      }
      _ => unimplemented!(),
    }

    /*
    let all_ch = frame.image_all_channels();

    let pixbuf = all_ch.buf();
    for (line, buf) in lines.zip(pixbuf.chunks_exact(line_width).skip(skip_rows)) {
      for (p, f) in line.iter_mut().zip(buf.iter()) {
        //debug_assert!(*f <= (1.0 + f32::EPSILON));
        // *p = (f * u16::MAX as f32) as u16;
        // *p = *f as u16;
      }
    }
    */
    Ok(())
  }

  fn can_skip_rows(&self) -> bool {
    false
  }
}
