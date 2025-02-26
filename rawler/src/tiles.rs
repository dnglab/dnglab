// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::{iter, ops::Range};

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
}
