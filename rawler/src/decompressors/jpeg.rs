//! Lossy JPEG decompressor for 8-bit RGB image strips and tiles.

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
  /// The JPEG is decoded to an 8-bit RGB image; each `u8` sample is
  /// zero-extended to `u16` without scaling. Only `ImageRgb8` output is
  /// accepted; any other format returns `Err`.
  ///
  /// # Errors
  /// Returns `Err` if JPEG decoding fails or the decoded image is not RGB-8.
  fn decompress(&self, src: &[u8], skip_rows: usize, lines: impl LineIteratorMut<'a, u16>, line_width: usize) -> std::result::Result<(), String> {
    let img = image::load_from_memory_with_format(src, image::ImageFormat::Jpeg).map_err(|err| format!("Lossy JPEG decompression failed: {:?}", err))?;
    match img {
      image::DynamicImage::ImageRgb8(image_buffer) => {
        for (dst, src) in lines.zip(image_buffer.chunks_exact(line_width).skip(skip_rows)) {
          for (dst, src) in dst.iter_mut().zip(src.iter()) {
            *dst = *src as u16; // Only change storage format, you MUST NOT scale up!
          }
        }
        Ok(())
      }
      _ => Err(format!("JpegDecompressor: expected RGB-8 image")),
    }
  }

  fn can_skip_rows(&self) -> bool {
    false
  }
}
