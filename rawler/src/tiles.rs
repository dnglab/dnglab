// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use log::debug;

/// Image tiles
pub struct TiledData<T> {
  pub tile_width: usize,
  pub tile_length: usize,
  pub tiles: Vec<Vec<T>>,
}

impl<T> TiledData<T>
where
  T: Clone,
{
  /// Construct new TiledData for given image
  ///
  /// `width` and `height` are for input. Tile size will be
  /// calculated based in input.
  pub fn new(img: &[T], width: usize, height: usize) -> Self {
    debug!("Tile input: w: {}, h: {}", width, height);
    let tile_width = Self::find_optimal_tile_size(width);
    let tile_length = Self::find_optimal_tile_size(height);
    let tile_dim = tile_length * tile_width;

    assert_eq!(width % tile_width, 0);
    assert_eq!(height % tile_length, 0);

    let mut tiles = Vec::with_capacity((width * height) / (tile_width * tile_length));

    for tile_row in 0..height / tile_length {
      for tile_col in 0..width / tile_width {
        let mut tile = Vec::with_capacity(tile_dim);
        for line in 0..tile_length {
          let offset = ((tile_row * tile_length) + line) * width + (tile_col * tile_width);
          tile.extend_from_slice(&img[offset..offset + tile_width]); // add line
        }
        tiles.push(tile);
      }
    }
    Self {
      tile_width,
      tile_length,
      tiles,
    }
  }

  /// Find optimal tile size
  fn find_optimal_tile_size(len: usize) -> usize {
    // TODO: write tests
    let start = len / 8;
    for i in (1..start).rev() {
      // We need mod_2 because tiles are compressed
      // with 2-component LJPEG
      if (len % i == 0) && (i % 2 == 0) {
        return i;
      }
    }
    return len;
  }
}
