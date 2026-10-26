// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Nikolay Amiantov <ab@fmap.me>

//! Sony ARW6 (lossy compressed RAW) decompressor.

mod bitstream;
mod container;
mod geometry;
mod idwt;
mod pixelops;

use std::io::Cursor;

use bitstream_io::BitReader;
use rayon::prelude::*;

use crate::Result;
use crate::alloc_image_ok;
use crate::bits::clampbits;
use crate::pixarray::{PixU16, SharedPix2D};

use bitstream::decode_component;
use container::{Region, TileRecord};
use geometry::{RegionGeometry, TileCoords, derive_geom};
use idwt::{diamond_idwt, idwt2d};
use pixelops::{delinearize, predict_horizontal};

const CODEC_BIT_DEPTH: u32 = 12;

/// BitPump for the codec's MSB-first (Big Endian) bitstreams
type BitPump<'a> = BitReader<Cursor<&'a [u8]>, bitstream_io::BigEndian>;

/// A 2-D array of wavelet coefficients.
struct Plane {
  pub w: usize,
  pub h: usize,
  pub data: Vec<i32>,
}

impl Plane {
  pub fn new(w: usize, h: usize) -> Self {
    Self { w, h, data: vec![0; w * h] }
  }

  #[inline(always)]
  pub fn row(&self, r: usize) -> &[i32] {
    &self.data[r * self.w..(r + 1) * self.w]
  }

  #[inline(always)]
  pub fn row_mut(&mut self, r: usize) -> &mut [i32] {
    let w = self.w;
    &mut self.data[r * w..(r + 1) * w]
  }
}

/// Decompress an ARW6 strip (`buf`) into the full `width` × `height` Bayer mosaic.
pub(crate) fn decompress_arw6(buf: &[u8], width: usize, height: usize, dummy: bool) -> Result<PixU16> {
  let image = alloc_image_ok!(width, height, dummy);
  let records = container::parse_records(buf)?;
  // The tiles are decoded straight into the shared image, so their rectangles
  // must lie inside it and must not overlap.
  for (i, rec) in records.iter().enumerate() {
    if rec.x + rec.w > width || rec.y + rec.h > height {
      return Err(
        format!(
          "ARW6: tile {}x{} at ({}, {}) exceeds the {}x{} image",
          rec.w, rec.h, rec.x, rec.y, width, height
        )
        .into(),
      );
    }
    if records[..i]
      .iter()
      .any(|prev| rec.x < prev.x + prev.w && prev.x < rec.x + rec.w && rec.y < prev.y + prev.h && prev.y < rec.y + rec.h)
    {
      return Err(format!("ARW6: tile at ({}, {}) overlaps another tile", rec.x, rec.y).into());
    }
  }
  let shared = SharedPix2D::new(image);
  records.par_iter().try_for_each(|rec| {
    let regions = container::parse_tile(rec)?;
    // Safety: the tile rectangles are validated non-overlapping above, and
    // decode_tile_into writes only within its tile's rectangle.
    decode_tile_into(rec, &regions, unsafe { shared.inner_mut() })
  })?;
  Ok(shared.into_inner())
}

/// Reconstruct one tile into its rectangle of `image`: entropy-decode every
/// region, undo the wavelets and assemble the four components into the tile's
/// Bayer mosaic.
///
/// Region 0 holds the components' LL data, regions 1..n_levels-1 their detail
/// levels coarsest-first, region n_levels `green_hi` alone; within the shared
/// regions, components 0, 1, 2 = `green_lo`, `chroma_r`, `chroma_b`.
fn decode_tile_into(rec: &TileRecord<'_>, regions: &[Region<'_>], image: &mut PixU16) -> Result<()> {
  let n_levels = regions.len() - 1; // vertical level count
  let coords = TileCoords::new(rec.w, rec.h, n_levels)?;
  let geoms = derive_geom(&coords);
  let decode_comp = |comp: usize| -> Result<Plane> {
    let mut ll = decode_single_orient(regions, &geoms, 0, comp, "LL")?;
    predict_horizontal(&mut ll);
    let mut recon = ll;
    // detail regions 1..n_levels-1 (coarsest -> finest), each with its vertical polyphase flip
    for region in 1..n_levels {
      let detail = decode_region_comp(regions, &geoms, region, comp)?;
      let [hl, lh, hh] = &detail[..] else {
        return Err(format!("ARW6: detail region {} did not decode to 3 orientations", region).into());
      };
      recon = idwt2d(&recon, lh, hl, hh, geoms[region].vflip)?;
    }
    Ok(recon)
  };
  // The three wavelet pipelines and green_hi are independent; decode them in
  // parallel (on top of the tiles decoding in parallel).
  let planes = (0..4)
    .into_par_iter()
    .map(|comp| {
      if comp < 3 {
        decode_comp(comp)
      } else {
        decode_single_orient(regions, &geoms, n_levels, 0, "green_hi")
      }
    })
    .collect::<Result<Vec<_>>>()?;
  let [green_lo, chroma_r, chroma_b, green_hi]: [Plane; 4] = planes.try_into().map_err(|_| "ARW6: expected 4 planes")?;
  assemble_tile_into(&green_lo, &chroma_r, &chroma_b, &green_hi, rec, image)
}

/// Decode a single-orientation region's component (the LL or green_hi,
/// named by `what` for errors) into its one plane.
fn decode_single_orient(regions: &[Region<'_>], geoms: &[RegionGeometry], region_i: usize, comp_i: usize, what: &str) -> Result<Plane> {
  let [plane]: [Plane; 1] = decode_region_comp(regions, geoms, region_i, comp_i)?
    .try_into()
    .map_err(|orientations: Vec<Plane>| format!("ARW6: {} decoded to {} orientations instead of one", what, orientations.len()))?;
  Ok(plane)
}

/// Entropy-decode one component of one coding region into its dequantized
/// per-orientation planes, using the shared tile geometry.
fn decode_region_comp(regions: &[Region<'_>], geoms: &[RegionGeometry], region_i: usize, comp_i: usize) -> Result<Vec<Plane>> {
  let region = &regions[region_i];
  let geom = &geoms[region_i];
  if region.components.len() != geom.n_components {
    return Err(
      format!(
        "ARW6: region {} carries {} components, geometry expects {}",
        region_i,
        region.components.len(),
        geom.n_components
      )
      .into(),
    );
  }
  let comp = &region.components[comp_i];
  if comp.orient_count != geom.orientations.len() {
    return Err(
      format!(
        "ARW6: region {} component declares {} orientations, geometry expects {}",
        region_i,
        comp.orient_count,
        geom.orientations.len()
      )
      .into(),
    );
  }
  decode_component(comp.data, geom.width, geom.rows_per_chunk, &geom.orientations, &comp.chunks)
}

/// Combine one tile's decoded components into its Bayer mosaic, written
/// straight into the tile's rectangle of `image`.
///
/// The green plane is `green_lo` and `green_hi` combined by one inverse
/// quincunx 5/3 level, offset by the 12-bit mid value. R and B pixels come
/// from the chroma residuals:
///
/// ```text
/// restored = (g1 + g2)/2 + 2*value
/// ```
///
/// where g1, g2 are the two greens of the same 2x2 Bayer cell (left and right
/// neighbours in the merged green plane). Everything is clamped to 12 bits;
/// the stored pixels are mapped through the delinearization curve back to
/// linear sensor values.
fn assemble_tile_into(green_lo: &Plane, chroma_r: &Plane, chroma_b: &Plane, green_hi: &Plane, rec: &TileRecord<'_>, image: &mut PixU16) -> Result<()> {
  let mid_value = 1i32 << (CODEC_BIT_DEPTH - 1);
  let h = green_lo.h.min(green_hi.h);
  let w = green_lo.w;
  if green_hi.w < w || chroma_r.h < h || chroma_r.w < w || chroma_b.h < h || chroma_b.w < w {
    return Err(
      format!(
        "ARW6: component size mismatch: green_lo {}x{}, green_hi {}x{}, chroma_r {}x{}, chroma_b {}x{}",
        green_lo.w, green_lo.h, green_hi.w, green_hi.h, chroma_r.w, chroma_r.h, chroma_b.w, chroma_b.h
      )
      .into(),
    );
  }
  let full_w = 2 * w;
  if full_w > rec.w || 2 * h > rec.h {
    return Err(format!("ARW6: {}x{} mosaic exceeds its {}x{} tile", full_w, 2 * h, rec.w, rec.h).into());
  }
  let greens = diamond_idwt(green_lo, green_hi, h, w);
  // Bayer RGGB layout per 2x2 cell: R at (even row, even col), B at (odd row,
  // odd col), greens on the other diagonal.
  let width = image.width;
  for r in 0..h {
    let g_row = greens.row(r);
    let cr_row = chroma_r.row(r);
    let cb_row = chroma_b.row(r);
    let (top, bottom) = image.data.split_at_mut((rec.y + 2 * r + 1) * width);
    let row_even = &mut top[(rec.y + 2 * r) * width + rec.x..][..full_w];
    let row_odd = &mut bottom[rec.x..][..full_w];
    for i in 0..w {
      let g0 = clampbits(g_row[2 * i] + mid_value, CODEC_BIT_DEPTH); // lower-left diagonal
      let g1 = clampbits(g_row[2 * i + 1] + mid_value, CODEC_BIT_DEPTH); // upper-right diagonal
      row_odd[2 * i] = delinearize(g0);
      row_even[2 * i + 1] = delinearize(g1);
      // mean of the cell's two greens (shared by its R and B)
      let green_mean = (g0 as i32 + g1 as i32) >> 1;
      row_even[2 * i] = delinearize(clampbits(green_mean + 2 * cr_row[i], CODEC_BIT_DEPTH));
      row_odd[2 * i + 1] = delinearize(clampbits(green_mean + 2 * cb_row[i], CODEC_BIT_DEPTH));
    }
  }
  Ok(())
}
