// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Overlapping tiles for demosaic algorithms
//!
//! Algorithms that interpolate from a neighbourhood of pixels process the
//! image in tiles: a tile reads its core and a halo of pixels around it, and
//! writes only its core. The cores don't overlap, so the tiles are processed in
//! parallel. If the halo covers the reach of the algorithm, the result doesn't
//! depend on the tiling.
//!
//! An algorithm that leaves a band at the edges of the image to another
//! interpolation tiles the [`inner_area`] of the image with a halo of the width
//! of the band, so the windows are never clipped. An algorithm that runs up to
//! the edges tiles the whole image, the windows of the tiles at the edges are
//! clipped then.

use rayon::prelude::*;

use crate::{
  imgop::{Dim2, Point, Rect},
  pixarray::{DisjointRegionMut, ImageRegionMut},
};

/// Tile of an overlapping tiling, see [`halo_tiles`]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HaloTile {
  /// Area the tile reads: its core and up to `halo` pixels around it, clipped
  /// to the image
  pub window: Rect,
  /// Area the tile writes, in the coordinates of the image
  pub core: Rect,
}

impl HaloTile {
  /// The core relative to the window
  pub fn core_in_window(&self) -> Rect {
    Rect::new(Point::new(self.core.p.x - self.window.p.x, self.core.p.y - self.window.p.y), self.core.d)
  }
}

/// Area of an image of `dim` that is at least `border` pixels away from its
/// edges, empty if there is none
pub fn inner_area(dim: Dim2, border: usize) -> Rect {
  let (w, h) = (dim.w.saturating_sub(2 * border), dim.h.saturating_sub(2 * border));
  if w == 0 || h == 0 {
    Rect::new(Point::new(border.min(dim.w), border.min(dim.h)), Dim2::default())
  } else {
    Rect::new(Point::new(border, border), Dim2::new(w, h))
  }
}

/// Tiles whose cores of size `core` cover `area` of an image of `dim`
///
/// The cores don't overlap, they are row by row from the top-left of `area`
/// and clipped at its right and bottom edge (see [`Dim2::tile_areas`]). The
/// window of a tile extends its core by `halo` pixels on each side, clipped to
/// the image. An empty area has no tiles.
///
/// # Panics
/// Panics if `area` is not inside of the image or if the width or height of
/// `core` is zero.
pub fn halo_tiles(dim: Dim2, area: Rect, core: Dim2, halo: usize) -> Vec<HaloTile> {
  assert!(
    area.p.x + area.d.w <= dim.w && area.p.y + area.d.h <= dim.h,
    "{:?} is outside of {}x{} pixels",
    area,
    dim.w,
    dim.h
  );
  area
    .d
    .tile_areas(core)
    .into_iter()
    .map(|core| {
      let core = core.translate(area.p);
      let window = Rect::new_with_points(
        Point::new(core.p.x.saturating_sub(halo), core.p.y.saturating_sub(halo)),
        Point::new((core.p.x + core.d.w + halo).min(dim.w), (core.p.y + core.d.h + halo).min(dim.h)),
      );
      HaloTile { window, core }
    })
    .collect()
}

/// Run `op` for `tiles` in parallel, each with the core of `output` it writes
///
/// The positions of the tiles are in the coordinates of `output`. `op` also
/// gets the scratch state of its worker, which `init` creates and the
/// following tiles of the worker reuse, e.g. for tile buffers.
///
/// # Panics
/// Panics if the cores of the tiles overlap or are not inside of `output`.
pub fn par_tiles<'a, T, S, I, F>(output: ImageRegionMut<'a, T>, tiles: &[HaloTile], init: I, op: F)
where
  T: Send,
  I: Fn() -> S + Sync + Send,
  F: Fn(&mut S, &HaloTile, &mut DisjointRegionMut<'a, T>) + Sync + Send,
{
  let cores: Vec<Rect> = tiles.iter().map(|tile| tile.core).collect();
  let regions = output.split_regions_mut(&cores).expect("Tiles write disjoint areas");
  tiles
    .par_iter()
    .zip(regions)
    .for_each_init(init, |state, (tile, mut region)| op(state, tile, &mut region));
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::pixarray::Pix2D;

  fn area(x: usize, y: usize, w: usize, h: usize) -> Rect {
    Rect::new(Point::new(x, y), Dim2::new(w, h))
  }

  #[test]
  fn inner_area_keeps_the_border() {
    assert_eq!(inner_area(Dim2::new(20, 12), 5), area(5, 5, 10, 2));
    // No pixels at least 5 pixels away from the edges
    assert!(inner_area(Dim2::new(20, 10), 5).is_empty());
    assert!(inner_area(Dim2::new(3, 20), 5).is_empty());
    assert_eq!(inner_area(Dim2::new(7, 4), 0), area(0, 0, 7, 4));
  }

  #[test]
  fn cores_cover_the_area_and_windows_the_halo() {
    let dim = Dim2::new(40, 30);
    let inner = inner_area(dim, 5);
    let tiles = halo_tiles(dim, inner, Dim2::new(8, 7), 5);
    // Each pixel of the area is in exactly one core, no pixel outside of it
    let mut count = Pix2D::<u16>::new(dim.w, dim.h);
    for tile in &tiles {
      for y in tile.core.p.y..tile.core.p.y + tile.core.d.h {
        for x in tile.core.p.x..tile.core.p.x + tile.core.d.w {
          *count.at_mut(y, x) += 1;
        }
      }
    }
    for (y, x) in (0..dim.h).flat_map(|y| (0..dim.w).map(move |x| (y, x))) {
      let inside = inner.intersect(&area(x, y, 1, 1)).is_some();
      assert_eq!(*count.at(y, x), inside as u16, "{}:{}", x, y);
    }
    // The halo of the inner area is inside of the image, so no window is clipped
    for tile in &tiles {
      let core = tile.core_in_window();
      assert_eq!(core.p, Point::new(5, 5), "{:?}", tile);
      assert_eq!(tile.window.d, Dim2::new(core.d.w + 10, core.d.h + 10), "{:?}", tile);
      assert_eq!(core.translate(tile.window.p), tile.core);
    }
    // The cores of the last column and row are clipped to the area
    assert_eq!(tiles.last().map(|tile| tile.core), Some(area(29, 19, 6, 6)));
  }

  #[test]
  fn windows_are_clipped_to_the_image() {
    let dim = Dim2::new(10, 6);
    let tiles = halo_tiles(dim, area(0, 0, 10, 6), Dim2::new(4, 4), 2);
    assert_eq!(
      tiles,
      [
        HaloTile {
          window: area(0, 0, 6, 6),
          core: area(0, 0, 4, 4)
        },
        HaloTile {
          window: area(2, 0, 8, 6),
          core: area(4, 0, 4, 4)
        },
        HaloTile {
          window: area(6, 0, 4, 6),
          core: area(8, 0, 2, 4)
        },
        HaloTile {
          window: area(0, 2, 6, 4),
          core: area(0, 4, 4, 2)
        },
        HaloTile {
          window: area(2, 2, 8, 4),
          core: area(4, 4, 4, 2)
        },
        HaloTile {
          window: area(6, 2, 4, 4),
          core: area(8, 4, 2, 2)
        },
      ]
    );
    // At the left and top edge, the halo is missing
    assert_eq!(tiles[4].core_in_window(), area(2, 2, 4, 2));
    assert_eq!(tiles[0].core_in_window(), area(0, 0, 4, 4));
  }

  #[test]
  fn empty_area_has_no_tiles() {
    let dim = Dim2::new(8, 8);
    assert!(halo_tiles(dim, inner_area(dim, 5), Dim2::new(4, 4), 5).is_empty());
  }

  #[test]
  #[should_panic(expected = "is outside of")]
  fn area_must_be_inside_of_the_image() {
    let _ = halo_tiles(Dim2::new(8, 8), area(4, 4, 5, 4), Dim2::new(4, 4), 1);
  }

  #[test]
  fn par_tiles_write_their_cores() {
    let dim = Dim2::new(23, 17);
    let mut out = Pix2D::<u16>::new(dim.w, dim.h);
    let tiles = halo_tiles(dim, inner_area(dim, 2), Dim2::new(5, 4), 2);
    // Each tile writes the index of its first pixel in the image, workers count their tiles
    par_tiles(
      out.view_mut().into(),
      &tiles,
      || 0_usize,
      |count, tile, region| {
        *count += 1;
        let index = (tile.core.p.y * dim.w + tile.core.p.x) as u16 + 1;
        region.rows_mut().for_each(|row| row.fill(index));
      },
    );
    for tile in &tiles {
      let index = (tile.core.p.y * dim.w + tile.core.p.x) as u16 + 1;
      assert_eq!(*out.at(tile.core.p.y + tile.core.d.h - 1, tile.core.p.x + tile.core.d.w - 1), index);
    }
    // The border is not written
    assert_eq!(*out.at(0, 0), 0);
    assert_eq!(*out.at(dim.h - 1, dim.w - 1), 0);
    assert_eq!(*out.at(dim.h - 2, 2), 0);
  }
}
