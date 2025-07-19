// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::{iter, ops::Range};

use crate::pixarray::{LineMut, SubPixel};

#[derive(Debug)]
pub struct ErrorNotTileable;

/// Image tile generator
pub struct ImageTiler<'a, T> {
  data: &'a [T],
  width: usize,
  #[allow(dead_code)]
  height: usize,
  cpp: usize,
  tiles: Range<usize>,
  tw: usize,
  th: usize,
  tcols: usize,
  trows: usize,
}

impl<'a, T> ImageTiler<'a, T> {
  pub fn new(data: &'a [T], width: usize, height: usize, cpp: usize, tw: usize, th: usize) -> Self {
    assert!(data.len() >= height * width * cpp);
    let tcols = width.div_ceil(tw);
    let trows = height.div_ceil(th);
    Self {
      data,
      width,
      height,
      cpp,
      tiles: Range { start: 0, end: trows * tcols },
      tw,
      th,
      tcols,
      trows,
    }
  }

  pub fn tile_cols(&self) -> usize {
    self.tcols
  }

  pub fn tile_rows(&self) -> usize {
    self.trows
  }

  pub fn tile_count(&self) -> usize {
    self.tile_rows() * self.tile_cols()
  }

  fn needs_padding(&self) -> bool {
    self.width % self.tw > 0
  }
}

impl<'a, T> Iterator for ImageTiler<'a, T>
where
  T: Copy + Default,
{
  type Item = Vec<T>;

  fn next(&mut self) -> Option<Self::Item> {
    if let Some(i) = self.tiles.next() {
      let mut buf = Vec::with_capacity(self.th * self.tw * self.cpp);

      let tile_row = i / self.tile_cols();
      let tile_col = i % self.tile_cols();

      //println!("Tile row: {}, col: {}", tile_row, tile_col);

      for row in 0..self.th {
        let off_row = (tile_row * self.th) + row;
        let offset = off_row * self.width * self.cpp + (tile_col * self.tw * self.cpp);

        if offset < self.data.len() {
          //println!("Fill row: {}", row);
          if tile_col < self.tile_cols() - 1 || !self.needs_padding() {
            let sub = &self.data[offset..offset + self.tw * self.cpp];
            buf.extend_from_slice(sub);
          } else {
            buf.extend_from_slice(&self.data[offset..offset + (self.width % self.tw) * self.cpp]);
            let last_pix = buf.last().copied().unwrap_or_default();
            buf.extend(iter::repeat(last_pix).take((self.tw - (self.width % self.tw)) * self.cpp));
          };
        } else {
          //println!("extend row: {}", row);
          buf.extend_from_within((row - 1) * self.tw * self.cpp..((row - 1) * self.tw * self.cpp) + self.tw * self.cpp);
        }
      }
      Some(buf)
    } else {
      None
    }
  }

  fn size_hint(&self) -> (usize, Option<usize>) {
    (self.tile_count(), Some(self.tile_count()))
  }
}

/// A trait for types that can be partitioned into mutable tiles.
///
/// This trait allows splitting a collection or buffer of subpixels into
/// mutable tiles of specified dimensions, returning an iterator over the tiles.
///
/// # Type Parameters
/// - `'a`: Lifetime of the data being tiled.
/// - `T`: The subpixel type, which must implement the `SubPixel` trait.
///
/// # Required Methods
/// - `into_tiles_iter_mut`: Consumes `self` and returns a mutable iterator over tiles,
///   or an error if the data cannot be tiled with the given dimensions.
///
/// # Errors
/// Returns `ErrorNotTileable` if the data cannot be partitioned into tiles
/// with the specified width, tile width, or tile height.
pub trait TilesMut<'a, T>
where
  T: SubPixel,
{
  fn into_tiles_iter_mut(self, width: usize, cpp: usize, tile_width: usize, tile_height: usize) -> std::result::Result<IntoTilesIter<'a, T>, ErrorNotTileable>;
}

/// Implementation for mutable slices of T: SubPixel
impl<'a, T> TilesMut<'a, T> for &'a mut [T]
where
  T: SubPixel,
{
  fn into_tiles_iter_mut(self, width: usize, cpp: usize, tile_width: usize, tile_height: usize) -> std::result::Result<IntoTilesIter<'a, T>, ErrorNotTileable> {
    assert!(width > 0, "Width and height must be greater than zero");
    assert!(tile_width * tile_height > 0, "Tile width and height must be greater than zero");
    assert!(cpp > 0, "cpp must be greater than zero");
    if !self.len().is_multiple_of(tile_width * cpp * tile_height) {
      return Err(ErrorNotTileable);
    }
    Ok(IntoTilesIter {
      count: 0,
      width,
      cpp,
      tile_width,
      tile_height,
      original: self,
    })
  }
}

/// An iterator that splits a mutable slice into tiles of specified width and height.
///
/// # Type Parameters
/// - `T`: The type of elements in the slice.
///
/// # Fields
/// - `count`: The current tile index or count of tiles processed.
/// - `width`: The width of the original image or data slice.
/// - `tile_width`: The width of each tile.
/// - `tile_height`: The height of each tile.
/// - `original`: A mutable reference to the original data slice to be tiled.
///
/// This iterator yields mutable references to tiles within the original slice,
/// allowing for in-place modification of each tile.
pub struct IntoTilesIter<'a, T> {
  count: usize,
  width: usize,
  cpp: usize,
  tile_width: usize,
  tile_height: usize,
  original: &'a mut [T],
}

impl<'a, T> IntoTilesIter<'a, T> {
  /// Returns the total tile count
  fn tile_count(&self) -> usize {
    self.original.len() / self.cpp / (self.tile_height * self.tile_width)
  }
}

/// We know the exact amount of tiles that can be
/// produced, so we mark the iterator as ExactSizeIterator
impl<'a, T> ExactSizeIterator for IntoTilesIter<'a, T> where T: Send {}

// unsafe impl<'a, T> TrustedLen for IntoTilesIter<'a, T> where T: Send {}

/// A iterator that gives owned Tiles
impl<'a, T> Iterator for IntoTilesIter<'a, T> {
  type Item = Tile<'a, T>;

  fn next(&mut self) -> Option<Self::Item> {
    let tile_cols = self.width / self.tile_width;
    let _tile_rows = self.original.len() / self.cpp / (self.tile_height * self.width);

    let tile_x = self.count % tile_cols;
    let tile_y = self.count / tile_cols;

    assert!(self.count <= self.tile_count());

    let start_index = tile_x * (self.tile_width * self.cpp) + tile_y * (self.width * self.cpp) * self.tile_height;
    self.count += 1;

    if start_index >= self.original.len() {
      return None;
    } else {
      // The next tile line has always a distance equal to full image width.
      let next_line_distance = self.width * self.cpp;
      let first_line_begin = &mut self.original[start_index..];
      if first_line_begin.len() < self.tile_height * next_line_distance - (tile_x * self.tile_width * self.cpp) {
        // The tile input buffer is too small. Maybe an issue with component-per-pixels?
        panic!("Tile buffer too small.")
      }
      let first_line = &mut first_line_begin[..self.tile_width * self.cpp];

      Some(Tile {
        first_line,
        tile_height: self.tile_height,
        width: self.width,
        cpp: self.cpp,
        _phantom: std::marker::PhantomData,
      })
    }
  }

  fn size_hint(&self) -> (usize, Option<usize>) {
    (self.tile_count() - self.count, Some(self.tile_count() - self.count))
  }
}

/// Represents a rectangular tile within an image buffer.
///
/// # Type Parameters
/// - `'a`: Lifetime of the data the tile references.
/// - `T`: Pixel type contained in the tile.
///
/// # Fields
/// - `first_line`: Pointer to the first line (row) of the tile's pixel data.
/// - `tile_height`: Number of rows in the tile.
/// - `width`: Width of the entire pixel buffer (not just the tile).
/// - `_phantom`: Marker to associate the lifetime `'a` and type `[T]` with the struct.
pub struct Tile<'a, T> {
  // contains tile_width as well
  first_line: *mut [T],
  tile_height: usize,
  width: usize, // of the pixbuf
  cpp: usize,
  _phantom: std::marker::PhantomData<&'a [T]>,
}

impl<'a, T> Tile<'a, T> {
  pub fn into_iter_mut(self) -> TileIterMut<'a, T> {
    TileIterMut { tile: self, current_line: 0 }
  }
}

// TODO: Add safety note
unsafe impl<T: Send> Send for Tile<'_, T> {}

/// An iterator that allows mutable access to the lines of a `Tile`.
///
/// # Type Parameters
/// * `T` - The type of the elements contained in the tile.
///
/// # Fields
/// * `tile` - The tile being iterated over.
/// * `current_line` - The index of the current line in the tile.
pub struct TileIterMut<'a, T> {
  tile: Tile<'a, T>,
  current_line: usize,
}

impl<'a, T> Iterator for TileIterMut<'a, T>
where
  T: SubPixel + 'a,
{
  type Item = LineMut<'a, T>;

  fn next(&mut self) -> Option<Self::Item> {
    if self.current_line >= self.tile.tile_height {
      return None;
    }

    // Calculating the next line offset is easy - each tile has same width/height,
    // so the distance is simply the full width of the image.
    let next_line_distance = self.tile.width * self.tile.cpp;
    let line_ptr = unsafe { (self.tile.first_line as *mut T).offset((self.current_line * next_line_distance) as isize) };
    self.current_line += 1;

    // This is safe because we check in the constructor if the line_ptr
    // can be advanced until tile end line.
    Some(unsafe { std::slice::from_raw_parts_mut(line_ptr, self.tile.first_line.len()) })
  }

  fn size_hint(&self) -> (usize, Option<usize>) {
    (self.tile.tile_height - self.current_line, Some(self.tile.tile_height - self.current_line))
  }
}

impl<'a, T> ExactSizeIterator for TileIterMut<'a, T> where T: SubPixel + 'a {}

#[cfg(test)]
mod tests {
  use super::*;

  use rayon::prelude::*;

  #[test]
  fn tile_1x1() -> std::result::Result<(), Box<dyn std::error::Error>> {
    crate::init_test_logger();
    let w = 1;
    let h = 1;
    let c = 3;
    let buf = vec![0_u16; w * h * c];

    let tiles: Vec<Vec<u16>> = ImageTiler::new(&buf, w, h, c, 20, 20).collect();

    assert_eq!(tiles.len(), 1);
    assert_eq!(tiles[0].len(), c * 20 * 20);

    Ok(())
  }

  #[test]
  fn test_par_bridge() {
    #[rustfmt::skip]
    let mut vec = vec![
      1,  2,    3,  4,   5,  6,
      7,  8,    9, 10,  11, 12,

      13, 14,  15, 16,  17, 18,
      19, 20,  21, 22,  23, 24,
    ];
    let cpp = 1;
    let tiles = vec.into_tiles_iter_mut(6, cpp, 2, 2);
    assert!(tiles.is_ok());
    let tiles = tiles.unwrap();

    tiles.par_bridge().for_each(|tile| {
      tile.into_iter_mut().for_each(|line| {
        for p in line {
          *p *= 2;
        }
      });
    });
  }

  #[test]
  fn test_par_bridge_without_collect() {
    #[rustfmt::skip]
    let mut vec = vec![
      1,  2,    3,  4,   5,  6,
      7,  8,    9, 10,  11, 12,

      13, 14,  15, 16,  17, 18,
      19, 20,  21, 22,  23, 24,
    ];
    let expected_vec: Vec<u16> = vec.iter().map(|p| p * 2).collect();
    let cpp = 1;
    let tiles = vec.into_tiles_iter_mut(6, cpp, 2, 2);
    assert!(tiles.is_ok());

    let tiles = tiles.unwrap();

    tiles.par_bridge().for_each(|tile| {
      tile.into_iter_mut().for_each(|line| {
        for p in line {
          *p *= 2;
        }
      });
    });

    assert_eq!(vec, expected_vec);
  }

  #[test]
  fn test_tiles_mut() {
    #[rustfmt::skip]
    let mut vec = vec![
      1,  2,    3,  4,   5,  6,
      7,  8,    9, 10,  11, 12,

      13, 14,  15, 16,  17, 18,
      19, 20,  21, 22,  23, 24,
    ];
    let cpp = 1;
    let tiles = vec.into_tiles_iter_mut(6, cpp, 2, 2);
    assert!(tiles.is_ok());
    let tiles = tiles.unwrap();

    let tiles: Vec<Tile<u16>> = tiles.collect();

    assert_eq!(tiles.len(), 6);
  }
}
