//! Lossy JPEG decompressor for 8-bit RGB or grayscale image strips and tiles.

use crate::decompressors::ljpeg::{LJpegDecompressor, is_lossless_jpeg};
use crate::decompressors::{Decompressor, LineIteratorMut};

/// Decompressor for lossy JPEG-compressed image data.
pub struct JpegDecompressor {}

impl JpegDecompressor {
  pub fn new() -> Self {
    Self {}
  }
}

impl<'a> Decompressor<'a, u16> for JpegDecompressor {
  /// Decodes a lossy JPEG buffer into `u16` pixel lines.
  ///
  /// The JPEG is decoded to an 8-bit RGB or grayscale image; each `u8`
  /// sample is zero-extended to `u16` without scaling. Only `ImageRgb8`
  /// and `ImageLuma8` output is accepted; any other format returns `Err`.
  ///
  /// # Errors
  /// Returns `Err` if `line_width` is 0, if JPEG decoding fails or the decoded image is not RGB-8 or Luma-8.
  fn decompress(&self, src: &[u8], skip_rows: usize, lines: impl LineIteratorMut<'a, u16>, line_width: usize) -> std::result::Result<(), String> {
    if line_width == 0 {
      return Err("JpegDecompressor: lines of 0 samples can't be decoded".to_string());
    }
    let img = image::load_from_memory_with_format(src, image::ImageFormat::Jpeg).map_err(|err| format!("Lossy JPEG decompression failed: {:?}", err))?;
    let samples: &[u8] = match &img {
      image::DynamicImage::ImageRgb8(image_buffer) => image_buffer,
      image::DynamicImage::ImageLuma8(image_buffer) => image_buffer,
      _ => return Err("JpegDecompressor: expected RGB-8 or Luma-8 image".to_string()),
    };
    for (dst, src) in lines.zip(samples.chunks_exact(line_width).skip(skip_rows)) {
      for (dst, src) in dst.iter_mut().zip(src.iter()) {
        *dst = *src as u16; // Only change storage format, you MUST NOT scale up!
      }
    }
    Ok(())
  }

  fn can_skip_rows(&self) -> bool {
    false
  }
}

/// Decompressor for JPEG data of TIFF compression 7, which is lossless JPEG
/// in raw image data and lossy JPEG in previews. The kind is detected for
/// each strip or tile from its frame header.
pub struct ModernJpegDecompressor {
  lossless: LJpegDecompressor,
}

impl ModernJpegDecompressor {
  /// `dng_bug` enables the 16-bit lossless JPEG deviation of DNG files with
  /// DNGVersion < 1.1.0.0
  pub fn new(dng_bug: bool) -> Self {
    Self {
      lossless: LJpegDecompressor::new_with_dng_bug(dng_bug),
    }
  }
}

impl<'a> Decompressor<'a, u16> for ModernJpegDecompressor {
  fn decompress(&self, src: &[u8], skip_rows: usize, lines: impl LineIteratorMut<'a, u16>, line_width: usize) -> std::result::Result<(), String> {
    if is_lossless_jpeg(src) {
      self.lossless.decompress(src, skip_rows, lines, line_width)
    } else {
      JpegDecompressor::new().decompress(src, skip_rows, lines, line_width)
    }
  }

  fn can_skip_rows(&self) -> bool {
    false
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn line_width_of_0_is_an_error() {
    let mut out = [0_u16; 8];
    let err = JpegDecompressor::new().decompress(&[], 0, std::iter::once(&mut out[..]), 0).err();
    assert!(err.as_ref().is_some_and(|err| err.contains("0 samples")), "{:?}", err);
  }
}
