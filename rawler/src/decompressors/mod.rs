// SPDX-License-Identifier: LGPL-2.1
// Copyright 2025 Daniel Vogelbacher <daniel@chaospixel.com>

use crate::pixarray::SubPixel;

pub mod crx;
pub mod deflate;
pub mod jpeg;
pub mod ljpeg;
pub mod packed;
pub mod radc;

/// Trait for mutable line iterators over image data.
///
/// This trait is implemented for iterators that yield mutable slices of subpixel data.
/// It is used to provide mutable access to each line of an image during decompression.
///
/// # Type Parameters
/// - `'a`: Lifetime of the data.
/// - `T`: The subpixel type, which must implement [`SubPixel`].
pub trait LineIteratorMut<'a, T>: Iterator<Item = &'a mut [T]> + ExactSizeIterator
where
  T: SubPixel + 'a,
{
}

impl<'a, T, I> LineIteratorMut<'a, T> for I
where
  I: ExactSizeIterator<Item = &'a mut [T]>,
  T: SubPixel + 'a,
{
}

/// Trait for immutable line iterators over image data.
///
/// This trait is implemented for iterators that yield immutable slices of subpixel data.
/// It is used to provide read-only access to each line of an image.
///
/// # Type Parameters
/// - `'a`: Lifetime of the data.
/// - `T`: The subpixel type, which must implement [`SubPixel`].
pub trait LineIterator<'a, T>: Iterator<Item = &'a [T]> + ExactSizeIterator
where
  T: SubPixel + 'a,
{
}

/// Trait for decompressors handling raw image data.
///
/// Implementors of this trait provide functionality to decompress raw image data into pixel lines (slices).
/// The decompressor operates on a source byte slice and writes decompressed data into provided line buffers.
///
/// # Type Parameters
/// - `'a`: Lifetime of the data.
/// - `T`: The subpixel type, which must implement [`SubPixel`].
pub trait Decompressor<'a, T>: Send + Sync
where
  T: SubPixel + 'a,
{
  /// Decompresses the source data into the provided line buffers.
  ///
  /// # Arguments
  /// * `src` - Source byte slice containing compressed image data.
  /// * `skip_rows` - Number of rows to skip before starting decompression.
  /// * `lines` - Mutable iterator over destination lines to write decompressed data.
  /// * `line_width` - The width of each line in pixels.
  ///
  /// # Returns
  /// * `Ok(())` on success.
  /// * `Err(String)` with an error message on failure.
  fn decompress(&self, src: &[u8], skip_rows: usize, lines: impl LineIteratorMut<'a, T>, line_width: usize) -> std::result::Result<(), String>;

  /// Returns `true` if the decompressor is optimized for strip-based processing.
  fn strips_optimized(&self) -> bool {
    false
  }

  /// Returns `true` if the decompressor is optimized for tile-based processing.
  fn tile_optimized(&self) -> bool {
    false
  }
}
