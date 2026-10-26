use crate::{
  CFA,
  imgop::{Dim2, Rect},
  pixarray::{Pix2DView, PixF32, RgbF32},
};
use std::marker::PhantomData;

pub struct DemosaicTiler<'a> {
  raw: Pix2DView<'a, f32>,
  cfa: &'a CFA,
  rgb: &'a mut RgbF32,

  size: Dim2,
  halo: Dim2,

  trow: usize,
  tcol: usize,
}

impl<'a> DemosaicTiler<'a> {
  pub fn new(raw: &'a PixF32, cfa: &'a CFA, rgb: &'a mut RgbF32, size: Dim2, halo: Dim2) -> Self {
    Self::from_view(raw.view(raw.rect()), cfa, rgb, size, halo)
  }

  pub fn from_view(raw: Pix2DView<'a, f32>, cfa: &'a CFA, rgb: &'a mut RgbF32, size: Dim2, halo: Dim2) -> Self {
    Self {
      raw,
      cfa,
      rgb,
      size,
      halo,
      trow: 0,
      tcol: 0,
    }
  }
}

impl<'a> Iterator for DemosaicTiler<'a> {
  type Item = Tile<'a>;

  fn next(&mut self) -> Option<Self::Item> {
    debug_assert!(self.size.w > self.halo.w * 2);
    debug_assert!(self.size.h > self.halo.h * 2);

    let stride_x = self.size.w - self.halo.w * 2;
    let stride_y = self.size.h - self.halo.h * 2;
    let tile_x = self.tcol * stride_x;
    let tile_y = self.trow * stride_y;
    if tile_y >= self.raw.height() {
      return None;
    }

    let tile_width = (self.raw.width() - tile_x).min(self.size.w);
    let tile_height = (self.raw.height() - tile_y).min(self.size.h);
    let tile = Rect::new(crate::imgop::Point::new(tile_x, tile_y), Dim2::new(tile_width, tile_height));

    let last_col = tile_x + tile_width == self.raw.width();
    let last_row = tile_y + tile_height == self.raw.height();
    let core_left = if tile_x == 0 { 0 } else { self.halo.w };
    let core_top = if tile_y == 0 { 0 } else { self.halo.h };
    let core_right = if last_col { 0 } else { self.halo.w };
    let core_bottom = if last_row { 0 } else { self.halo.h };
    let core = Rect::new(
      crate::imgop::Point::new(tile_x + core_left, tile_y + core_top),
      Dim2::new(tile_width - core_left - core_right, tile_height - core_top - core_bottom),
    );

    if last_col {
      self.tcol = 0;
      if last_row {
        // Use the image height as a terminal row sentinel for the next call.
        self.trow = self.raw.height();
      } else {
        self.trow += 1;
      }
    } else {
      self.tcol += 1;
    }

    Some(Tile {
      raw: Pix2DView {
        rect: self.raw.rect,
        inner: self.raw.inner,
      },
      cfa: self.cfa,
      rgb: self.rgb as *mut RgbF32,
      tile,
      core,
      _garbage: [0.0; 3],
      _phantom: PhantomData,
    })
  }
}

pub struct Tile<'a> {
  raw: Pix2DView<'a, f32>,
  cfa: &'a CFA,
  rgb: *mut RgbF32,

  tile: Rect,
  core: Rect,

  _garbage: [f32; 3],
  _phantom: PhantomData<&'a mut RgbF32>,
}

// `DemosaicTiler::next` creates only disjoint core rectangles, and `rgb_mut`
// writes through `rgb` exclusively for pixels in a tile's core.
unsafe impl Send for Tile<'_> {}

impl<'a> Tile<'a> {
  /// Returns the tile-local raw view. `tile` is expressed relative to `raw`.
  #[inline(always)]
  pub fn raw(&self) -> Pix2DView<'a, f32> {
    self.raw.view(self.tile)
  }

  /// Returns a CFA whose `(0, 0)` phase matches the tile-local raw view.
  #[inline(always)]
  pub fn cfa(&self) -> CFA {
    self.cfa.shift(self.tile.p.x, self.tile.p.y)
  }

  /// Returns the output pixel at tile-local `(row, col)` when it belongs to
  /// this tile's core; writes outside the core go to `_garbage`.
  #[inline(always)]
  pub fn rgb_mut(&mut self, row: usize, col: usize) -> &mut [f32; 3] {
    debug_assert!(row < self.tile.d.h, "row is outside the tile");
    debug_assert!(col < self.tile.d.w, "column is outside the tile");

    let raw_x = self.tile.p.x + col;
    let raw_y = self.tile.p.y + row;
    let is_core = raw_x >= self.core.p.x && raw_x < self.core.p.x + self.core.d.w && raw_y >= self.core.p.y && raw_y < self.core.p.y + self.core.d.h;

    if is_core {
      // SAFETY: DemosaicTiler yields pairwise-disjoint cores. Tile's fields
      // are private, so this is the only route to the RGB destination.
      unsafe { (&mut *self.rgb).at_mut(raw_y, raw_x) }
    } else {
      &mut self._garbage
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn raw(width: usize, height: usize) -> PixF32 {
    PixF32::new(width, height)
  }

  fn assert_tiling(width: usize, height: usize, size: Dim2, halo: Dim2) {
    let raw = raw(width, height);
    let cfa = CFA::new("RGGB");
    let mut rgb = RgbF32::new(width, height);

    {
      for mut tile in DemosaicTiler::new(&raw, &cfa, &mut rgb, size, halo) {
        assert!(!tile.core.is_empty());
        for row in 0..tile.tile.d.h {
          for col in 0..tile.tile.d.w {
            for value in tile.rgb_mut(row, col) {
              *value += 1.0;
            }
          }
        }
      }
    }
    assert!(
      rgb.pixels().iter().all(|pixel| *pixel == [1.0; 3]),
      "core coverage is not exactly one for image {width}x{height}, tile {:?}, halo {:?}",
      size,
      halo
    );

    let mut rgb = RgbF32::new(width, height);
    {
      for mut tile in DemosaicTiler::new(&raw, &cfa, &mut rgb, size, halo) {
        for row in 0..tile.tile.d.h {
          for col in 0..tile.tile.d.w {
            let is_halo = row < halo.h
              || row >= tile.tile.d.h.saturating_sub(halo.h)
              || col < halo.w
              || col >= tile.tile.d.w.saturating_sub(halo.w);
            *tile.rgb_mut(row, col) = [if is_halo { -1.0 } else { 1.0 }; 3];
          }
        }
      }
    }
    for row in halo.h..height.saturating_sub(halo.h) {
      for col in halo.w..width.saturating_sub(halo.w) {
        assert_eq!(
          *rgb.at(row, col),
          [1.0; 3],
          "halo write reached the core at ({col}, {row}) for image {width}x{height}, tile {:?}, halo {:?}",
          size,
          halo
        );
      }
    }
  }

  #[test]
  fn tiles_cover_configuration_matrix() {
    for (width, height, size, halo) in [
      (1, 1, Dim2::new(1, 1), Dim2::new(0, 0)),
      (2, 3, Dim2::new(3, 5), Dim2::new(1, 2)),
      (3, 3, Dim2::new(3, 3), Dim2::new(1, 1)),
      (7, 7, Dim2::new(5, 5), Dim2::new(1, 1)),
      (8, 6, Dim2::new(4, 4), Dim2::new(1, 1)),
      (8, 7, Dim2::new(5, 5), Dim2::new(1, 1)),
      (17, 13, Dim2::new(6, 5), Dim2::new(1, 1)),
      (500, 500, Dim2::new(256, 256), Dim2::new(8, 8)),
      (501, 500, Dim2::new(255, 256), Dim2::new(7, 8)),
      (4096, 17, Dim2::new(256, 9), Dim2::new(8, 2)),
      (4097, 18, Dim2::new(257, 10), Dim2::new(9, 2)),
      (10000, 19, Dim2::new(1024, 11), Dim2::new(32, 3)),
    ] {
      assert_tiling(width, height, size, halo);
    }
  }

  fn assert_cfa_is_tile_local(size: Dim2) {
    let raw = raw(8, 7);
    let cfa = CFA::new("RGGB");
    let mut rgb = RgbF32::new(8, 7);

    let tiler = DemosaicTiler::new(&raw, &cfa, &mut rgb, size, Dim2::new(1, 1));
    for tile in tiler {
      let tile_cfa = tile.cfa();
      for row in 0..tile.tile.d.h {
        for col in 0..tile.tile.d.w {
          assert_eq!(tile_cfa.color_at(row, col), cfa.color_at(tile.tile.p.y + row, tile.tile.p.x + col));
        }
      }
    }
  }

  #[test]
  fn cfa_phase_is_correct_for_even_tile_stride() {
    assert_cfa_is_tile_local(Dim2::new(4, 4));
  }

  #[test]
  fn cfa_phase_is_correct_for_odd_tile_stride() {
    assert_cfa_is_tile_local(Dim2::new(5, 5));
  }

}
