// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Nikolay Amiantov <ab@fmap.me>

//! Tile geometry: which rows are stored in which chunks.

use crate::Result;

use super::container::REGIONS;

/// A span of the canvas's lattice with the data. *Lattice* rows
/// [top, bottom) (not the canvas rows!) hold data; chunk 0's window begins at
/// *lattice* row `row_base`. This may be an even or an odd lattice if it's a
/// detail level. The decoder doesn't care because it just needs to know which
/// rows are stored per chunk, which is fully described by top, bottom,
/// row_base and rows_per_chunk.
#[derive(Clone, Copy)]
pub(super) struct LatticeSpan {
  pub top: usize,
  pub bottom: usize,
  pub row_base: usize,
}

/// One coding region's geometry, shared by every component within it.
pub(super) struct RegionGeometry {
  pub width: usize,
  /// Lattice rows of this region stored per chunk.
  pub rows_per_chunk: usize,
  /// One span per orientation, in stream order (1 for LL / green_hi, 3 for
  /// detail = HL/LH/HH).
  pub orientations: Vec<LatticeSpan>,
  /// Components this region carries. green_lo, chroma_r, chroma_b for LL and
  /// detail regions, green_hi in a dedicated region.
  pub n_components: usize,
  /// Vertical polyphase flip for recombining a detail region (see [`vflip`]);
  /// false for the single-orientation regions.
  pub vflip: bool,
}

/// The tile's coordinate frame. Shared by all of the tile's regions and
/// components.
///
/// The codec places every region's rows on one shared 1-D canvas to determine
/// which rows, for each orientation, are stored where. We define lattices on
/// this canvas for different regions and components, and split it by chunks.
///
/// * `chunk_origin` -- the point on the canvas where chunk 0 begins.
/// * `comp_top` -- the point where the components' data begins: it occupies
///   canvas rows [comp_top, comp_top + comp_height).
///
/// comp_top >= chunk_origin >= 0. Only points >= comp_top are actually
/// stored. The coordinates affect how the data is split between chunks.
///
/// The codec chooses both such that:
/// * The coarsest level's detail starts at chunk_origin;
/// * The first and the final chunks are of the same size.
#[derive(Clone, Copy)]
pub(super) struct TileCoords {
  pub tile_width: usize,
  pub tile_height: usize,
  /// Number of vertical levels (= number of coding regions - 1).
  pub n_levels: usize,
}

impl TileCoords {
  pub fn new(tile_width: usize, tile_height: usize, n_levels: usize) -> Result<Self> {
    // The derivations below need at least two levels; the fixed region count
    // guarantees that.
    if n_levels + 1 != REGIONS {
      return Err(format!("ARW6: {} coding regions, expected {}", n_levels + 1, REGIONS).into());
    }
    Ok(Self {
      tile_width,
      tile_height,
      n_levels,
    })
  }

  /// Every component is half the tile tall.
  pub fn comp_height(&self) -> usize {
    self.tile_height / 2
  }

  /// The step of the coarsest level.
  pub fn chunk_height(&self) -> usize {
    1 << (self.n_levels - 1)
  }

  /// The first point of the coarsest level's detail (half of step).
  pub fn chunk_origin(&self) -> usize {
    1 << (self.n_levels - 2)
  }

  /// The point where the data begins: chunk_origin plus the offset that makes
  /// the first and last chunks symmetric.
  pub fn comp_top(&self) -> usize {
    let phase = (-((self.comp_height() / 2) as i64)).rem_euclid(self.chunk_height() as i64) as usize;
    self.chunk_origin() + phase
  }

  pub fn comp_bottom(&self) -> usize {
    self.comp_top() + self.comp_height()
  }
}

/// One wavelet level's geometry for a given tile.
struct LevelGeom {
  width: usize,
  step: usize,
  /// A level's orientation can use one of two lattices: an even or an odd
  /// one. They differ in how the data is arranged. Several orientations can
  /// reuse the same lattice; it only affects how the data is distributed
  /// between chunks.
  even: LatticeSpan,
  odd: LatticeSpan,
}

/// Each vertical level's geometry, finest first, level = 1..n_levels.
fn iter_levels(coords: &TileCoords) -> Vec<LevelGeom> {
  // phase = the lattice's canvas offset: 0 for `even`, half a step for `odd`.
  let span = |step: usize, phase: usize| LatticeSpan {
    top: (coords.comp_top() - phase).div_ceil(step),
    bottom: (coords.comp_bottom() - phase).div_ceil(step),
    row_base: (coords.chunk_origin() - phase).div_ceil(step),
  };
  (1..=coords.n_levels)
    .map(|level| {
      let step = 1 << (level - 1); // absolute-row lattice step
      LevelGeom {
        width: coords.tile_width >> level,
        step,
        even: span(step, 0),
        odd: span(step, step / 2),
      }
    })
    .collect()
}

/// Derive each coding region's geometry from the coordinates (one
/// [`RegionGeometry`] per region, indexed by region number).
/// Every component within a region shares the region's geometry, and this is
/// where each region's component count is fixed. The regions are:
///
/// 0: LL, the coarsest region; a single orientation;
/// 1..n_levels-1: detail regions, coarsest first, three orientations each (HL
///   on the `even` lattice, LH/HH sharing the `odd`);
/// n_levels: green_hi, a single component.
pub(super) fn derive_geom(coords: &TileCoords) -> Vec<RegionGeometry> {
  let levels = iter_levels(coords); // levels are finest to coarsest here.
  let chunk_height = coords.chunk_height();

  // region 0: LL, reuses the coarsest lattice, 3 components.
  let coarsest = levels.last().expect("at least two levels");
  let mut region_geoms = vec![RegionGeometry {
    width: coarsest.width,
    rows_per_chunk: chunk_height / coarsest.step,
    orientations: vec![coarsest.even],
    n_components: 3,
    vflip: false,
  }];
  // regions 1..n_levels-1: the 2-D detail splits, coarsest first, 3 components.
  for region in 1..coords.n_levels {
    let lvl = &levels[coords.n_levels - region];
    region_geoms.push(RegionGeometry {
      width: lvl.width,
      rows_per_chunk: chunk_height / lvl.step,
      orientations: vec![lvl.even, lvl.odd, lvl.odd], // HL, LH, HH; HH reuses LH's span
      n_components: 3,
      vflip: vflip(coords.comp_top(), lvl.step),
    });
  }
  // region n_levels: green_hi, reuses the finest lattice, a single component.
  let finest = &levels[0];
  region_geoms.push(RegionGeometry {
    width: finest.width,
    rows_per_chunk: chunk_height / finest.step,
    orientations: vec![finest.even],
    n_components: 1,
    vflip: false,
  });
  region_geoms
}

/// Row-interleave order for recombining a detail region of lattice spacing
/// `step` (>= 2), passed to [`super::idwt::idwt2d`]: true when the recombined
/// plane's first row comes from the `odd` lattice, offset half a step --
/// i.e. when its top coordinate ceil(comp_top/(step/2)) is an odd number.
fn vflip(comp_top: usize, step: usize) -> bool {
  comp_top.div_ceil(step >> 1) & 1 == 1
}
