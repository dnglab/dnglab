// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Staggered CFA layouts (DNG CFALayout 2 to 5)
//!
//! In a staggered layout, every second row or column of photosites is offset
//! by half a photosite, as on the Super CCD sensors of Fujifilm. The
//! photosites form a checkerboard in the image axes, which is a full square
//! grid rotated by 45°. [`Stagger`] maps the samples onto that grid without
//! interpolation, so the demosaic algorithms for rectangular CFAs work on it.
//!
//! The demosaiced image is mapped back onto the stage 3 grid of DNG readers:
//! the area with doubled columns if rows are offset, or with doubled rows if
//! columns are offset. Each photosite is a pixel of that grid, the other
//! pixels are interpolated from their four neighbours.
//!
//! Layouts 6 to 9 offset rows and columns, they are not supported.

use rayon::prelude::*;

use crate::cfa::{CFA, CFALayout};
use crate::imgop::{Dim2, Point, Rect};
use crate::rawimage::DefaultCrop;

/// Geometry of the photosites of a staggered CFA layout and of the grid
/// rotated by 45° that holds them
///
/// A photosite at row `r` and column `c` of the area has the position (x, y)
/// on the stage 3 grid, `(2c + o, r)` if rows are offset or `(c, 2r + o)` if
/// columns are offset, where `o` is 1 for the offset rows or columns. On the
/// rotated grid, it has the column `(x + y - parity) / 2` and the row
/// `(y - x + parity) / 2 + row_offset`.
#[derive(Clone, Debug, PartialEq)]
pub struct Stagger {
  /// Area of the image data with the photosites, the ActiveArea
  area: Rect,
  /// Rows are offset (layouts 4 and 5), otherwise columns (layouts 2 and 3)
  rows_offset: bool,
  /// The offset rows or columns are those whose index plus `parity` is odd
  parity: usize,
  /// Added to the rows of the rotated grid, so they start at 0
  row_offset: usize,
  /// Size of the rotated grid
  rotated: Dim2,
}

impl Stagger {
  /// Geometry of the photosites of `area` in the image data, `layout` is
  /// relative to the top-left sample of the image data
  ///
  /// `None` for a rectangular layout. Fails for layouts 6 to 9 and an area
  /// smaller than 2x2 pixels.
  pub fn new(layout: CFALayout, area: Rect) -> crate::Result<Option<Self>> {
    // The layout relative to the area
    let (rows_offset, parity) = match layout.shift(area.p.x, area.p.y) {
      CFALayout::Rectangular => return Ok(None),
      CFALayout::StaggeredA => (false, 0),
      CFALayout::StaggeredB => (false, 1),
      CFALayout::StaggeredC => (true, 0),
      CFALayout::StaggeredD => (true, 1),
      _ => return Err(input_error!("Demosaic of the staggered CFALayout {:?} is not supported", layout)),
    };
    if area.d.w < 2 || area.d.h < 2 {
      return Err(input_error!("Staggered CFA data of {}x{} pixels is too small", area.d.w, area.d.h));
    }
    let mut stagger = Self {
      area,
      rows_offset,
      parity,
      row_offset: 0,
      rotated: Dim2::new(0, 0),
    };
    // The positions are linear in the row and the column, so the extremes are
    // at the first and last two rows and columns
    let (w, h) = (area.d.w, area.d.h);
    let corners: Vec<(usize, usize)> = [0, 1, h - 2, h - 1]
      .into_iter()
      .flat_map(|r| [0, 1, w - 2, w - 1].into_iter().map(move |c| (r, c)))
      .collect();
    let half_diff = |(r, c): (usize, usize)| {
      let (x, y) = stagger.position(r, c);
      (x as i64 - y as i64 - parity as i64) / 2
    };
    stagger.row_offset = corners.iter().map(|corner| half_diff(*corner)).max().unwrap_or(0).max(0) as usize;
    let (rows, cols) = corners
      .iter()
      .map(|(r, c)| stagger.rotated_position(stagger.position(*r, *c)))
      .fold((0, 0), |(rows, cols), (row, col)| (rows.max(row + 1), cols.max(col + 1)));
    stagger.rotated = Dim2::new(cols, rows);
    Ok(Some(stagger))
  }

  /// Size of the grid rotated by 45°
  pub fn rotated(&self) -> Dim2 {
    self.rotated
  }

  /// Size of the stage 3 grid, the area with doubled columns or rows
  pub fn stage3(&self) -> Dim2 {
    let (sx, sy) = self.full_scale();
    Dim2::new(self.area.d.w * sx, self.area.d.h * sy)
  }

  /// Scale from the image data to the stage 3 grid (horizontal, vertical)
  pub fn full_scale(&self) -> (usize, usize) {
    if self.rows_offset { (2, 1) } else { (1, 2) }
  }

  /// Position on the stage 3 grid of the photosite at `row` and `col` of the area
  fn position(&self, row: usize, col: usize) -> (usize, usize) {
    if self.rows_offset {
      (2 * col + ((row + self.parity) & 1), row)
    } else {
      (col, 2 * row + ((col + self.parity) & 1))
    }
  }

  /// Row and column on the rotated grid of the photosite at `position` of the stage 3 grid
  fn rotated_position(&self, (x, y): (usize, usize)) -> (usize, usize) {
    let col = (x + y - self.parity) / 2;
    let row = (y as i64 - x as i64 + self.parity as i64) / 2 + self.row_offset as i64;
    (row as usize, col)
  }

  /// Row and column of the area of the photosite at `row` and `col` of the
  /// rotated grid, `None` if there is none
  fn photosite(&self, row: usize, col: usize) -> Option<(usize, usize)> {
    let diff = row as i64 - self.row_offset as i64;
    let x = col as i64 - diff + self.parity as i64;
    let y = col as i64 + diff;
    if x < 0 || y < 0 {
      return None;
    }
    let (x, y) = (x as usize, y as usize);
    let (r, c) = if self.rows_offset {
      // x has the parity of the offset of row y
      (y, (x.checked_sub((y + self.parity) & 1)?) / 2)
    } else {
      ((y.checked_sub((x + self.parity) & 1)?) / 2, x)
    };
    (r < self.area.d.h && c < self.area.d.w && self.position(r, c) == (x, y)).then_some((r, c))
  }

  /// 2x2 CFA of the rotated grid for the staggered `cfa`, which is relative to
  /// the top-left sample of the image data
  ///
  /// Fails if the colors of the photosites don't repeat in 2x2 positions of
  /// the rotated grid.
  pub fn rotated_cfa(&self, cfa: &CFA) -> crate::Result<CFA> {
    let mut pattern: [[Option<u8>; 2]; 2] = [[None; 2]; 2];
    // The photosites around the center cover several periods of the staggered pattern
    let stage3 = self.stage3();
    let (cx, cy) = (stage3.w / 2, stage3.h / 2);
    let center = self.rotated_position((cx + (cx + cy + self.parity) % 2, cy));
    for row in center.0.saturating_sub(8)..(center.0 + 8).min(self.rotated.h) {
      for col in center.1.saturating_sub(8)..(center.1 + 8).min(self.rotated.w) {
        let Some((r, c)) = self.photosite(row, col) else {
          continue;
        };
        let color = cfa.color_at(self.area.p.y + r, self.area.p.x + c) as u8;
        match &mut pattern[row % 2][col % 2] {
          Some(known) if *known != color => {
            return Err(input_error!("Staggered CFA pattern '{}' is no 2x2 CFA on the grid rotated by 45°", cfa));
          }
          known => *known = Some(color),
        }
      }
    }
    let colors: Option<Vec<u8>> = pattern.iter().flatten().copied().collect();
    colors
      .and_then(|colors| CFA::from_colors(&colors, 2, 2))
      .ok_or_else(|| input_error!("Staggered CFA pattern '{}' gives no CFA on the grid rotated by 45°", cfa))
  }

  /// Pattern of the area for the 2x2 pattern `rotated` of the rotated grid, as
  /// values and their width and height
  ///
  /// The pattern starts at the top-left photosite of the area. It has 4x2
  /// positions (rows x columns) if rows are offset, otherwise 2x4.
  pub fn staggered_pattern<T: Copy>(&self, rotated: [[T; 2]; 2]) -> (Vec<T>, usize, usize) {
    let (width, height) = if self.rows_offset { (2, 4) } else { (4, 2) };
    let values = (0..height)
      .flat_map(|r| (0..width).map(move |c| (r, c)))
      .map(|(r, c)| {
        let (row, col) = self.rotated_position(self.position(r, c));
        rotated[row % 2][col % 2]
      })
      .collect();
    (values, width, height)
  }

  /// Map the samples of the area onto the rotated grid. `src` is one plane of
  /// image data of `width` pixels per row. Positions without a photosite are 0.
  pub fn unstagger(&self, src: &[f32], width: usize) -> Vec<f32> {
    let mut dst = vec![0.0; self.rotated.w * self.rotated.h];
    dst.par_chunks_mut(self.rotated.w.max(1)).enumerate().for_each(|(row, line)| {
      for (col, value) in line.iter_mut().enumerate() {
        if let Some((r, c)) = self.photosite(row, col) {
          *value = src[(self.area.p.y + r) * width + self.area.p.x + c];
        }
      }
    });
    dst
  }

  /// Map an image of the rotated grid with `channels` samples per pixel onto
  /// the stage 3 grid
  ///
  /// The pixels of the photosites take their values, the pixels between them
  /// the mean of their neighbours above, below, left and right.
  pub fn to_stage3(&self, src: &[f32], channels: usize) -> Vec<f32> {
    let stage3 = self.stage3();
    let rotated_width = self.rotated.w;
    let sample = |(x, y): (usize, usize), channel: usize| {
      let (row, col) = self.rotated_position((x, y));
      src[(row * rotated_width + col) * channels + channel]
    };
    let mut dst = vec![0.0; stage3.w * stage3.h * channels];
    dst.par_chunks_mut((stage3.w * channels).max(1)).enumerate().for_each(|(y, line)| {
      for (x, pixel) in line.chunks_exact_mut(channels).enumerate() {
        if (x + y + self.parity).is_multiple_of(2) {
          for (channel, value) in pixel.iter_mut().enumerate() {
            *value = sample((x, y), channel);
          }
        } else {
          // The four neighbours are photosites, those outside the grid are skipped
          let neighbours: Vec<(usize, usize)> = [
            x.checked_sub(1).map(|x| (x, y)),
            (x + 1 < stage3.w).then_some((x + 1, y)),
            y.checked_sub(1).map(|y| (x, y)),
            (y + 1 < stage3.h).then_some((x, y + 1)),
          ]
          .into_iter()
          .flatten()
          .collect();
          for (channel, value) in pixel.iter_mut().enumerate() {
            let sum: f32 = neighbours.iter().map(|position| sample(*position, channel)).sum();
            *value = sum / neighbours.len().max(1) as f32;
          }
        }
      }
    });
    dst
  }

  /// DefaultCrop on the stage 3 grid: `default_crop` (relative to the area)
  /// scaled by [`Self::full_scale`], otherwise `crop_area` (relative to the
  /// image data) scaled the same way. `None` without one or for a crop that
  /// doesn't fit.
  pub fn stage3_crop(&self, crop_area: Option<Rect>, default_crop: Option<&DefaultCrop>) -> Option<Rect> {
    let scale = self.full_scale();
    match default_crop {
      Some(crop) => crop.rect_scaled(scale, self.stage3()),
      None => {
        let crop = crop_area?.rebase(&self.area)?;
        let rect = Rect::new(
          Point::new(crop.p.x * scale.0, crop.p.y * scale.1),
          Dim2::new(crop.d.w * scale.0, crop.d.h * scale.1),
        );
        rect.intersect(&Rect::new(Point::zero(), self.stage3()))
      }
    }
  }
}

/// Rotate an image of `width` x `height` pixels with `channels` samples per
/// pixel 90° clockwise
pub fn rotate_90cw(src: &[f32], width: usize, height: usize, channels: usize) -> Vec<f32> {
  let mut dst = vec![0.0; src.len()];
  // The rotated image has `height` pixels per row
  dst.par_chunks_mut((height * channels).max(1)).enumerate().for_each(|(row, line)| {
    for (col, pixel) in line.chunks_exact_mut(channels).enumerate() {
      let src_pos = ((height - 1 - col) * width + row) * channels;
      pixel.copy_from_slice(&src[src_pos..src_pos + channels]);
    }
  });
  dst
}

/// Rectangle `rect` of an image of `height` rows after the image is rotated 90° clockwise
pub fn rotate_rect_90cw(rect: Rect, height: usize) -> Rect {
  Rect::new(Point::new(height - (rect.p.y + rect.d.h), rect.p.x), Dim2::new(rect.d.h, rect.d.w))
}
