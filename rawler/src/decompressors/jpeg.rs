//! Lossy JPEG decompressor for 8-bit RGB or grayscale image strips and tiles.

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
  /// Returns `Err` if JPEG decoding fails or the decoded image is not RGB-8 or Luma-8.
  fn decompress(&self, src: &[u8], skip_rows: usize, lines: impl LineIteratorMut<'a, u16>, line_width: usize) -> std::result::Result<(), String> {
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
