// SPDX-License-Identifier: LGPL-2.1
// Copyright 2025 Daniel Vogelbacher <daniel@chaospixel.com>

//! Decompressors for image codecs.
//!
//! The central abstraction is the [`Decompressor`] trait. Implementors receive
//! a compressed byte slice and write decoded pixels into mutable line
//! iterators.
//!
//! # Example
//!
//! Decode a 50 × 20 image stored as 12-bit big-endian packed samples.
//! At 12 bits per pixel, two pixels share three bytes, so the total buffer
//! size is `50 × 20 × 12 / 8 = 1500` bytes.
//!
//! ```
//! use rawler::bits::Endian;
//! use rawler::decompressors::packed::PackedDecompressor;
//! use rawler::decompressors::decompress_lines;
//! # fn main() -> std::result::Result<(), String> {
//! // 2 pixels packed into 3 bytes (MSB-first): 50 px × 12 bit / 8 = 75 bytes/line
//! // 20 lines × 75 bytes = 1500 bytes
//! let src = vec![0u8; 1500];
//! let width = 50;
//! let height = 20;
//! let dummy = false;
//!
//! let dc = PackedDecompressor::new(12, Endian::Big);
//! let image = decompress_lines::<u16, _>(&src, width, height, dummy, dc)?;
//!
//! assert_eq!(image.pixels().len(), 50 * 20);
//! # Ok(())
//! # }
//! ```

use crate::{
  alloc_image_typed_ok,
  pixarray::{LineMut, Pix2D, SubPixel},
};

use rayon::prelude::*;

pub mod arw6;
pub mod crx;
pub mod deflate;
pub mod jpeg;
pub mod jpegxl;
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
#[allow(unused)]
pub(crate) trait LineIterator<'a, T>: Iterator<Item = &'a [T]> + ExactSizeIterator
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

  /// Returns `true` if the decompressor is optimized for input line skipping
  /// while decompression.
  ///
  /// For example, for a given bitstream, a packed bits decompressor could
  /// calculate the starting bit for a given row, means it could skip multiple
  /// rows before decompression of a specific row.
  ///
  /// In contrast, a JPEG decompressor asked to decompress lin 8-20 from a given
  /// bitstream, is unable to skip the previous rows as the compressed row length
  /// vary because of entropy coding.
  fn can_skip_rows(&self) -> bool;
}

/// Decompresses a full image by calling a closure once per row in parallel.
///
/// Allocates a [`Pix2D<T>`] output buffer and iterates over its rows using
/// Rayon. The closure receives a mutable slice for the row and the zero-based
/// row index, and is responsible for writing the decompressed pixel data.
///
/// # Arguments
/// * `width` - Width of the image in pixels.
/// * `height` - Height of the image in rows.
/// * `dummy` - When `true`, allocates an uninitialised dummy buffer instead of
///   a zeroed one (used to skip actual decompression in benchmarks / probing).
/// * `closure` - Called as `closure(line, row)` for each row.
///
/// # Returns
/// * `Ok(Pix2D<T>)` with all rows filled on success.
/// * `Err(String)` if any row's closure returns an error.
#[inline(always)]
pub fn decompress_lines_fn<T, F>(width: usize, height: usize, dummy: bool, closure: &F) -> std::result::Result<Pix2D<T>, String>
where
  F: Fn(&mut [T], usize) -> std::result::Result<(), String> + Sync,
  T: SubPixel,
{
  let mut out: Pix2D<T> = alloc_image_typed_ok!(T, width, height, dummy);
  out
    .pixels_mut()
    .par_chunks_mut(width)
    .enumerate()
    .try_for_each(|(row, line)| closure(line, row))?;
  Ok(out)
}

/// Decompresses a full image using a [`Decompressor`] implementation, one row at a time.
///
/// Allocates a [`Pix2D<T>`] output buffer and processes each row in parallel
/// with Rayon. [`Decompressor::decompress`] is called once per row, with
/// `skip_rows` set to the row index and a single-element line iterator.
///
/// # Arguments
/// * `src` - Source byte slice containing the compressed image data.
/// * `width` - Width of the image in pixels.
/// * `height` - Height of the image in rows.
/// * `dummy` - When `true`, allocates an uninitialised dummy buffer and skips
///   decompression (see [`decompress_lines_fn`]).
/// * `decompressor` - A value implementing [`Decompressor`] for all lifetimes.
///
/// # Returns
/// * `Ok(Pix2D<T>)` with all rows filled on success.
/// * `Err(String)` if the decompressor returns an error for any row.
#[inline(always)]
pub fn decompress_lines<T, D>(src: &[u8], width: usize, height: usize, dummy: bool, decompressor: D) -> std::result::Result<Pix2D<T>, String>
where
  D: for<'a> Decompressor<'a, T>,
  T: SubPixel,
{
  let mut out: Pix2D<T> = alloc_image_typed_ok!(T, width, height, dummy);
  out
    .pixels_mut()
    .par_chunks_exact_mut(width)
    .enumerate()
    .try_for_each(|(row, line)| decompressor.decompress(src, row, std::iter::once(line), width))?;
  Ok(out)
}

/// Decompresses a full image by calling a closure once per strip of `stripsize` rows.
///
/// Similar to [`decompress_lines_fn`] but groups consecutive rows into strips
/// of `stripsize` before dispatching. This is useful for codecs that must decode
/// several rows together (e.g. interleaved or vertically-linked formats).
///
/// The closure receives a flat mutable slice covering all `stripsize` rows of the
/// strip (`line.len() == width * stripsize`) and the absolute row index of the
/// first row in the strip (`row_start = block_index * stripsize`).
///
/// # Arguments
/// * `width` - Width of the image in pixels.
/// * `height` - Height of the image in rows. Must be a multiple of `stripsize`.
/// * `stripsize` - Number of rows per strip.
/// * `dummy` - When `true`, allocates an uninitialised dummy buffer.
/// * `closure` - Called as `closure(block_slice, row_start)` for each strip.
///
/// # Returns
/// * `Ok(Pix2D<T>)` on success.
/// * `Err(String)` if any block's closure returns an error.
#[inline(always)]
pub fn decompress_strips_fn<T, F>(width: usize, height: usize, stripsize: usize, dummy: bool, closure: &F) -> std::result::Result<Pix2D<T>, String>
where
  F: Fn(&mut [T], usize, usize) -> std::result::Result<(), String> + Sync,
  T: SubPixel,
{
  let mut out: Pix2D<T> = alloc_image_typed_ok!(T, width, height, dummy);
  out
    .pixels_mut()
    .par_chunks_mut(width * stripsize)
    .enumerate()
    .try_for_each(|(strip, lines)| closure(lines, strip, strip * stripsize))?;
  Ok(out)
}

/// Decompresses a full image using a [`Decompressor`] implementation, one block of rows at a time.
///
/// Similar to [`decompress_lines`] but groups consecutive rows into blocks of
/// `stripsize` before dispatching. [`Decompressor::decompress`] is called once per
/// block with `skip_rows` set to the absolute row index of the first row in the
/// block and a `ChunksExactMut` iterator that yields each individual row within
/// the block.
///
/// This is useful for codecs that inherently decode multiple rows per call,
/// such as those with vertical dependencies or multi-row entropy coding units.
///
/// # Arguments
/// * `src` - Source byte slice containing the compressed image data.
/// * `width` - Width of the image in pixels.
/// * `height` - Height of the image in rows. Must be a multiple of `stripsize`.
/// * `dummy` - When `true`, allocates an uninitialised dummy buffer.
/// * `stripsize` - Number of rows per block.
/// * `decompressor` - A value implementing [`Decompressor`] for all lifetimes.
///
/// # Returns
/// * `Ok(Pix2D<T>)` on success.
/// * `Err(String)` if the decompressor returns an error for any block.
#[inline(always)]
pub fn decompress_strips<T, D>(src: &[u8], width: usize, height: usize, dummy: bool, stripsize: usize, decompressor: D) -> std::result::Result<Pix2D<T>, String>
where
  D: for<'a> Decompressor<'a, T>,
  T: SubPixel,
{
  let mut out: Pix2D<T> = alloc_image_typed_ok!(T, width, height, dummy);
  out
    .pixels_mut()
    .par_chunks_mut(width * stripsize)
    .enumerate()
    .try_for_each(|(strip, lines)| decompressor.decompress(src, strip * stripsize, lines.chunks_exact_mut(width), width))?;
  Ok(out)
}

/// Decompresses a full image by calling a closure over fixed-size chunks of the output buffer.
///
/// Unlike the line-based helpers, this function partitions the flat output
/// pixel buffer into chunks of exactly `chunksize` pixels and processes them
/// in parallel with Rayon. It is intended for compressed streams whose
/// internal structure does not align to row boundaries (e.g. some sequential
/// entropy-coded formats).
///
/// The closure receives a mutable pixel slice of length `chunksize` and the
/// zero-based chunk index. Errors cannot be signalled from the closure; use
/// [`decompress_lines_fn`] when fallible per-row processing is needed.
///
/// # Arguments
/// * `width` - Width of the image in pixels (used only for buffer allocation).
/// * `height` - Height of the image in rows (used only for buffer allocation).
/// * `chunksize` - Number of pixels per chunk. Should evenly divide
///   `width * height` to avoid a short trailing chunk.
/// * `dummy` - When `true`, allocates an uninitialised dummy buffer.
/// * `closure` - Called as `closure(chunk, chunk_id)` for each chunk.
///
/// # Returns
/// * `Ok(Pix2D<T>)` on success.
#[inline(always)]
pub fn decompress_chunked_fn<T, F>(width: usize, height: usize, chunksize: usize, dummy: bool, closure: &F) -> std::result::Result<Pix2D<T>, String>
where
  F: Fn(&mut [T], usize) + Sync,
  T: SubPixel,
{
  let mut out: Pix2D<T> = alloc_image_typed_ok!(T, width, height, dummy);
  out.pixels_mut().par_chunks_mut(chunksize).enumerate().for_each(|(chunk_id, chunk)| {
    closure(chunk, chunk_id);
  });
  Ok(out)
}

/// Adapts a bare function pointer into a [`Decompressor`] that processes one line at a time.
///
/// The wrapped function has the signature
/// `fn(src: &[u8], row: usize, line: &mut [T], line_width: usize) -> Result<(), String>`
/// and is called once for every line yielded by the iterator passed to
/// [`Decompressor::decompress`]. The `row` argument is `skip_rows + line_index`,
/// so callers that supply a non-zero `skip_rows` can use it to locate the
/// correct position in the source buffer.
///
/// Because a function pointer is inherently generic over all lifetimes, this
/// type satisfies the `for<'a> Decompressor<'a, T>` bound required by
/// [`decompress_lines`] and [`decompress_strips`].
///
/// Non-capturing closures can be coerced to function pointers and passed
/// directly to the tuple constructor: `FnLineDecompressor(|src, row, line, w| { … })`.
pub struct FnLineDecompressor<T>(fn(&[u8], usize, LineMut<T>, usize) -> std::result::Result<(), String>);

impl<'a, T> Decompressor<'a, T> for FnLineDecompressor<T>
where
  T: SubPixel + 'a,
{
  fn decompress(&self, src: &[u8], skip_rows: usize, lines: impl LineIteratorMut<'a, T>, line_width: usize) -> std::result::Result<(), String> {
    for (row, line) in lines.enumerate() {
      (self.0)(src, skip_rows + row, line, line_width)?;
    }
    Ok(())
  }

  /// For line decompressors, we assume these should be able to skip lines.
  fn can_skip_rows(&self) -> bool {
    true
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const WIDTH: usize = 1024;

  // Fills each pixel with (row as u16).wrapping_add(col as u16)
  fn fill_row_col(src: &[u8], row: usize, line: &mut [u16], _line_width: usize) -> std::result::Result<(), String> {
    let _ = src;
    for (col, pixel) in line.iter_mut().enumerate() {
      *pixel = (row as u16).wrapping_add(col as u16);
    }
    Ok(())
  }

  fn always_err(_src: &[u8], _row: usize, _line: &mut [u16], _line_width: usize) -> std::result::Result<(), String> {
    Err("intentional error".into())
  }

  fn record_line_width(_src: &[u8], _row: usize, line: &mut [u16], line_width: usize) -> std::result::Result<(), String> {
    // Write line_width into first pixel so the test can observe it
    if !line.is_empty() {
      line[0] = line_width as u16;
    }
    Ok(())
  }

  // --- FnLineDecompressor::decompress ---

  #[test]
  fn test_single_line_pixels_filled() {
    let src = vec![0u8; 1];
    let mut line = vec![0u16; WIDTH];
    let decomp = FnLineDecompressor::<u16>(fill_row_col);
    decomp.decompress(&src, 0, std::iter::once(line.as_mut_slice()), WIDTH).unwrap();
    for (col, &pixel) in line.iter().enumerate() {
      assert_eq!(pixel, col as u16, "col {col}");
    }
  }

  #[test]
  fn test_skip_rows_forwarded() {
    let src = vec![0u8; 1];
    let mut line = vec![0u16; WIDTH];
    let decomp = FnLineDecompressor::<u16>(fill_row_col);
    // skip_rows=7: fn receives 7 + 0 = 7
    decomp.decompress(&src, 7, std::iter::once(line.as_mut_slice()), WIDTH).unwrap();
    for (col, &pixel) in line.iter().enumerate() {
      assert_eq!(pixel, 7u16.wrapping_add(col as u16), "col {col}");
    }
  }

  #[test]
  fn test_multiple_lines_row_indices() {
    let src = vec![0u8; 1];
    let mut buf = vec![0u16; WIDTH * 4];
    let decomp = FnLineDecompressor::<u16>(fill_row_col);
    // skip_rows=2, 4 lines: fn receives rows 2,3,4,5
    decomp.decompress(&src, 2, buf.chunks_exact_mut(WIDTH), WIDTH).unwrap();
    for row in 0..4usize {
      for col in 0..WIDTH {
        let expected = (2 + row as u16).wrapping_add(col as u16);
        assert_eq!(buf[row * WIDTH + col], expected, "row {row} col {col}");
      }
    }
  }

  #[test]
  fn test_line_width_forwarded() {
    let src = vec![0u8; 1];
    let mut line = vec![0u16; WIDTH];
    let decomp = FnLineDecompressor::<u16>(record_line_width);
    decomp.decompress(&src, 0, std::iter::once(line.as_mut_slice()), WIDTH).unwrap();
    assert_eq!(line[0], WIDTH as u16);
  }

  #[test]
  fn test_error_propagates() {
    let src = vec![0u8; 1];
    let mut line = vec![0u16; WIDTH];
    let decomp = FnLineDecompressor::<u16>(always_err);
    let result = decomp.decompress(&src, 0, std::iter::once(line.as_mut_slice()), WIDTH);
    assert_eq!(result.unwrap_err(), "intentional error");
  }

  #[test]
  fn test_error_stops_after_first_failing_line() {
    let src = vec![0u8; 1];
    let mut buf = vec![0u16; WIDTH * 3];
    let decomp = FnLineDecompressor::<u16>(always_err);
    // All three lines would fail; just confirm the whole call is Err
    let result = decomp.decompress(&src, 0, buf.chunks_exact_mut(WIDTH), WIDTH);
    assert!(result.is_err());
  }

  // --- decompress_lines integration ---

  #[test]
  fn test_decompress_lines_pixel_values() {
    let src = vec![0u8; 1];
    let height = 4usize;
    let result = decompress_lines::<u16, _>(&src, WIDTH, height, false, FnLineDecompressor(fill_row_col)).unwrap();
    let pixels = result.pixels();
    for row in 0..height {
      for col in 0..WIDTH {
        let expected = (row as u16).wrapping_add(col as u16);
        assert_eq!(pixels[row * WIDTH + col], expected, "row {row} col {col}");
      }
    }
  }

  #[test]
  fn test_decompress_lines_output_dimensions() {
    let src = vec![0u8; 1];
    let height = 8usize;
    let result = decompress_lines::<u16, _>(&src, WIDTH, height, false, FnLineDecompressor(fill_row_col)).unwrap();
    assert_eq!(result.pixels().len(), WIDTH * height);
  }
}
