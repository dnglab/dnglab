// SPDX-License-Identifier: LGPL-2.1
// Copyright 2025 Daniel Vogelbacher <daniel@chaospixel.com>

//! JPEG-XL decompressor for DNG strips and tiles.

use jxl_oxide::{
  EnumColourEncoding, JxlImage, RenderingIntent,
  color::{ColourEncoding, ColourSpace, Primaries, TransferFunction, WhitePoint},
  image::BitDepth,
};

use crate::decompressors::{Decompressor, LineIteratorMut};
use crate::imgop::matrix::{multiply, multiply_row1};
use crate::imgop::xyz::{SRGB_TO_XYZ_D65, XYZ_TO_REC2020_D65};

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

/// Conversion of samples rendered as linear sRGB back to the colour encoding
/// declared by a lossy (XYB) codestream, see [`render_as_linear_srgb`]
#[derive(Clone, Copy)]
enum LinearSrgbTo {
  /// Linear RGB with other primaries, by a matrix from linear sRGB
  Rgb([[f32; 3]; 3]),
  /// Linear gray, by the luminance of linear sRGB
  Gray,
}

/// Requests linear sRGB output for lossy (XYB) codestreams with a linear colour
/// encoding that jxl-oxide would damage, and returns the conversion that
/// restores the declared encoding.
///
/// Workaround for jxl-oxide 0.12: it renders XYB images in linear sRGB first and
/// maps them to other primaries or to gray with gamut mapping (or clipping) to
/// 0.0 .. 1.0. That destroys values above 1.0, like floats on the scale of the
/// DNG WhiteLevel, and colours outside of the sRGB gamut, like camera native data
/// declared as Rec. 2100. libjxl converts by a plain matrix, which is done here.
///
/// Returns `None` if the default rendering is kept.
fn render_as_linear_srgb(image: &mut JxlImage) -> Option<LinearSrgbTo> {
  let metadata = &image.image_header().metadata;
  if !metadata.xyb_encoded {
    return None;
  }
  let ColourEncoding::Enum(encoding) = &metadata.colour_encoding else {
    return None;
  };
  if encoding.white_point != WhitePoint::D65 || encoding.tf != TransferFunction::Linear {
    return None;
  }
  let conversion = match (encoding.colour_space, encoding.primaries) {
    (ColourSpace::Grey, _) => LinearSrgbTo::Gray,
    (ColourSpace::Rgb, Primaries::Bt2100) => LinearSrgbTo::Rgb(multiply(&XYZ_TO_REC2020_D65, &SRGB_TO_XYZ_D65)),
    // Linear sRGB is rendered without gamut mapping
    _ => return None,
  };
  image.request_color_encoding(EnumColourEncoding::srgb_linear(RenderingIntent::Relative));
  Some(conversion)
}

impl<'a> Decompressor<'a, f32> for JpegXLDecompressor {
  /// Decodes a floating point JPEG-XL buffer into `f32` pixel lines.
  ///
  /// Renders frame 0 of the JXL image (alpha channel excluded) and writes
  /// samples row by row into `lines`, skipping the first `skip_rows` rows.
  /// Samples keep their stored values, e.g. on the scale of the DNG WhiteLevel.
  /// Lossy codestreams are converted from XYB as by libjxl, see [`render_as_linear_srgb`].
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
    let mut image = JxlImage::builder()
      .read(src)
      .map_err(|err| format!("Failed to read JPEG-XL image: {:?}", err))?;
    let bit_depth = image.image_header().metadata.bit_depth;
    if !matches!(bit_depth, BitDepth::FloatSample { .. }) {
      return Err(format!("JPEG-XL bit depth {:?} is not a floating point format", bit_depth));
    }
    let conversion = render_as_linear_srgb(&mut image);
    let frame = image.render_frame(0).map_err(|err| format!("Failed to render JPEG-XL image: {:?}", err))?;

    let mut stream = frame.stream_no_alpha();
    // Gray images are rendered with three samples per pixel
    let rendered_cpp = stream.channels() as usize;
    let output_cpp = if matches!(conversion, Some(LinearSrgbTo::Gray)) { 1 } else { rendered_cpp };
    let mut row = vec![0.0_f32; line_width / output_cpp * rendered_cpp];
    for _ in 0..skip_rows {
      stream.write_to_buffer(&mut row);
    }
    for line in lines {
      let written = stream.write_to_buffer(&mut row);
      if written != row.len() || line.len() != line_width {
        return Err(format!(
          "JPEG-XL image returned {} of {} samples for a line of {} samples",
          written,
          row.len(),
          line.len()
        ));
      }
      match conversion {
        None => line.copy_from_slice(&row),
        Some(LinearSrgbTo::Rgb(matrix)) => {
          for (out, rgb) in line.as_chunks_mut::<3>().0.iter_mut().zip(row.as_chunks::<3>().0) {
            *out = multiply_row1(&matrix, rgb);
          }
        }
        Some(LinearSrgbTo::Gray) => {
          let [lr, lg, lb] = SRGB_TO_XYZ_D65[1];
          for (out, [r, g, b]) in line.iter_mut().zip(row.as_chunks::<3>().0) {
            *out = lr * r + lg * g + lb * b;
          }
        }
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

  /// Samples of [`JXL_FLOAT_LOSSLESS`] and [`JXL_FLOAT_LOSSY`], far above the nominal range of 0.0 .. 1.0
  fn samples_float() -> Vec<f32> {
    (0..4)
      .flat_map(|y| (0..8).flat_map(move |x| [100 + 20 * x + 10 * y, 400 + 15 * x, 900 - 30 * y]))
      .map(|v| v as f32)
      .collect()
  }

  /// Lossy 8x4 RGB codestream with linear 32-bit float samples (Rec. 2100 primaries),
  /// encoded by cjxl with distance 0.5 from [`samples_float`]
  const JXL_FLOAT_LOSSY: &[u8] = &[
    0xff, 0x0a, 0x18, 0x70, 0x72, 0x09, 0x79, 0xb4, 0x08, 0x00, 0x13, 0x08, 0x00, 0x40, 0x01, 0x77, 0x3f, 0xc8, 0x01, 0x00, 0x50, 0xa1, 0x32, 0xca, 0xb8, 0xc1,
    0xcb, 0xb9, 0x9e, 0x2f, 0x3f, 0x74, 0x58, 0x4c, 0xdb, 0xb8, 0xce, 0xd0, 0x56, 0xdb, 0x16, 0x96, 0x30, 0x36, 0xdb, 0x0b, 0x83, 0x84, 0x24, 0xb1, 0x38, 0x38,
    0xe4, 0xc2, 0x43, 0x40, 0x81, 0xb7, 0x35, 0xaf, 0x39, 0x3a, 0xd9, 0x13, 0x65, 0x32, 0x4e, 0x0a, 0x2a, 0x49, 0x24, 0x47, 0xf0, 0xb6, 0xff, 0xee, 0x44, 0x94,
    0x00, 0x68, 0xe6, 0x9f, 0xd6, 0xc1, 0x09, 0x80, 0x9a, 0xdb, 0x23, 0x62, 0x17, 0x59, 0x4f, 0xac, 0x04,
  ];

  /// Lossy 8x4 gray codestream with linear 32-bit float samples, encoded by cjxl
  /// with distance 0.5 from [`samples_float_gray`]
  const JXL_FLOAT_LOSSY_GRAY: &[u8] = &[
    0xff, 0x0a, 0x18, 0x70, 0x72, 0x49, 0xd1, 0x22, 0x00, 0x13, 0x08, 0x00, 0x34, 0x01, 0x77, 0x3f, 0xc8, 0x01, 0x00, 0x50, 0xa1, 0x32, 0xca, 0xb8, 0xc1, 0xcb,
    0xb9, 0x9e, 0x2f, 0x3f, 0x74, 0x58, 0x4c, 0xdb, 0xb8, 0xce, 0xd0, 0x56, 0xdb, 0x16, 0x96, 0x30, 0x36, 0xdb, 0x0b, 0x83, 0x84, 0x84, 0x04, 0x09, 0x08, 0x6c,
    0xa6, 0x51, 0x30, 0x08, 0xa1, 0x92, 0xc4, 0x42, 0x50, 0x3b, 0xef, 0x25, 0x49, 0x86, 0x68, 0x03, 0x33, 0x00, 0xeb, 0x4f, 0x7f, 0xec, 0x1b, 0xdd, 0x24, 0x57,
    0xde, 0xbc, 0xe7, 0x6e, 0x80, 0x4c, 0xd3, 0x92, 0x00, 0x09, 0x00, 0x02, 0x00,
  ];

  /// Samples of [`JXL_FLOAT_LOSSY_GRAY`], far above the nominal range of 0.0 .. 1.0
  fn samples_float_gray() -> Vec<f32> {
    (0..4).flat_map(|y| (0..8).map(move |x| (100 + 100 * x + 10 * y) as f32)).collect()
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

  /// Lossy codestreams in linear Rec. 2100 or gray keep values above 1.0, as decoded by libjxl
  #[test]
  fn lossy_float_codestream_keeps_values_above_1() -> std::result::Result<(), String> {
    let mut out = vec![0_f32; 8 * 4 * 3];
    JpegXLDecompressor::new(16).decompress(JXL_FLOAT_LOSSY, 0, out.chunks_exact_mut(8 * 3), 8 * 3)?;
    // Lossy compression, libjxl decodes these codestreams with less than 3 % deviation
    for (decoded, expected) in out.iter().zip(samples_float()) {
      assert!((decoded - expected).abs() < expected * 0.05, "decoded {} for {}", decoded, expected);
    }
    let mut out = vec![0_f32; 8 * 4];
    JpegXLDecompressor::new(16).decompress(JXL_FLOAT_LOSSY_GRAY, 0, out.chunks_exact_mut(8), 8)?;
    for (decoded, expected) in out.iter().zip(samples_float_gray()) {
      assert!((decoded - expected).abs() < expected * 0.05, "decoded {} for {}", decoded, expected);
    }
    Ok(())
  }

  #[test]
  fn float_decoding_rejects_integer_codestream() {
    let mut out = vec![0_f32; 8 * 4];
    assert!(JpegXLDecompressor::new(16).decompress(JXL_12BIT, 0, out.chunks_exact_mut(8), 8).is_err());
  }
}
