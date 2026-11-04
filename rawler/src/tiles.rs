// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::ops::Range;

use crate::{
  imgop::{Dim2, Rect},
  pixarray::ImageView,
};

/// Image tile generator
pub struct ImageTiler<'a, T> {
  image: ImageView<'a, T>,
  /// Parts of the tiles inside of the image
  areas: Vec<Rect>,
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
      image: ImageView::new(&data[..height * width * cpp], width, height, cpp),
      areas: Dim2::new(width, height).tile_areas(Dim2::new(tw, th)),
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
}

impl<'a, T> ImageTiler<'a, T>
where
  T: Copy,
{
  /// Materialize the tile at linear index `idx` (row-major).
  ///
  /// Stateless: borrows `&self` only, so callers can build tiles from many
  /// threads in parallel (rayon `par_iter`). Edge-padding and last-row
  /// replication follow the same rules as the iterator.
  pub fn build_tile(&self, idx: usize) -> Vec<T> {
    let (tw, th, cpp) = (self.tw, self.th, self.image.channels());
    let mut buf = Vec::with_capacity(th * tw * cpp);
    let area = self.areas[idx];
    for row in self.image.region(area).rows() {
      buf.extend_from_slice(row);
      // Columns right of the image repeat the last pixel
      let last = &row[row.len() - cpp..];
      for _ in area.width()..tw {
        buf.extend_from_slice(last);
      }
    }
    // Rows below the image repeat the last row
    for _ in area.height()..th {
      buf.extend_from_within(buf.len() - tw * cpp..);
    }
    buf
  }
}

impl<'a, T> Iterator for ImageTiler<'a, T>
where
  T: Copy,
{
  type Item = Vec<T>;

  fn next(&mut self) -> Option<Self::Item> {
    let i = self.tiles.next()?;
    Some(self.build_tile(i))
  }

  fn size_hint(&self) -> (usize, Option<usize>) {
    (self.tile_count(), Some(self.tile_count()))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

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
  fn build_tile_pads_partial_tiles() {
    // 5x3 pixels in tiles of 4x2: the right tiles have one column of the
    // image, the bottom tiles one row
    let mono: Vec<u16> = (0..15).collect();
    let tiles: Vec<Vec<u16>> = ImageTiler::new(&mono, 5, 3, 1, 4, 2).collect();
    #[rustfmt::skip]
    let expected = [
      vec![0, 1, 2, 3,
           5, 6, 7, 8],
      vec![4, 4, 4, 4,
           9, 9, 9, 9],
      vec![10, 11, 12, 13,
           10, 11, 12, 13],
      vec![14, 14, 14, 14,
           14, 14, 14, 14],
    ];
    assert_eq!(tiles, expected);

    // Three samples per pixel, the padding repeats the last pixel
    let rgb: Vec<u16> = (0..15).flat_map(|p| [p, p + 100, p + 200]).collect();
    let tiler = ImageTiler::new(&rgb, 5, 3, 3, 4, 2);
    #[rustfmt::skip]
    assert_eq!(tiler.build_tile(1), [
      4, 104, 204,  4, 104, 204,  4, 104, 204,  4, 104, 204,
      9, 109, 209,  9, 109, 209,  9, 109, 209,  9, 109, 209,
    ]);
    assert_eq!(tiler.build_tile(2), [&rgb[30..42], &rgb[30..42]].concat());
    #[rustfmt::skip]
    assert_eq!(tiler.build_tile(3), [
      14, 114, 214,  14, 114, 214,  14, 114, 214,  14, 114, 214,
      14, 114, 214,  14, 114, 214,  14, 114, 214,  14, 114, 214,
    ]);
  }
}
