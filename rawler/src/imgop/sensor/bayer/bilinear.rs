use multiversion::multiversion;

use crate::{
  cfa::PlaneColor,
  imgop::{Dim2, Rect},
  pixarray::{Color2D, Pix2D},
  CFA,
};

use super::Demosaic;

#[derive(Default)]
pub struct Bilinear4Channel {}

impl Bilinear4Channel {
  pub fn new() -> Self {
    Self {}
  }
}

impl Demosaic<f32, 4> for Bilinear4Channel {
  /// Debayer image by using bilinear method.
  fn demosaic(&self, pixels: &[f32], dim: Dim2, cfa: &CFA, colors: &PlaneColor, roi: Rect) -> Color2D<f32, 4> {
    Self::demosaic_4ch(pixels, dim, cfa, colors, roi)
  }
}

impl Bilinear4Channel {
  #[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
  fn demosaic_4ch(pixels: &[f32], dim: Dim2, cfa: &CFA, colors: &PlaneColor, roi: Rect) -> Color2D<f32, 4> {
    if colors.plane_count() != 4 {
      panic!("Demosaic for 4 channels needs 4 color planes, but {} given", colors.plane_count());
    }
    log::debug!("Bilinear debayer ROI: {:?}", roi);
    let plane_map = colors.plane_lookup_table();
    let ch = |row: usize, col: usize| -> usize { plane_map[cfa.color_at(row, col)] };

    let pixels = Pix2D::new_with(pixels.to_vec(), dim.w, dim.h); // TODO: prevent to_vec(), add Pix2D as input argument

    let mut out = Color2D::new(dim.w, dim.h);

    // Process edges
    out.at_mut(0, 0)[ch(0, 0)] = *pixels.at(0, 0);
    out.at_mut(0, 0)[ch(0, 1)] = *pixels.at(0, 1);
    out.at_mut(0, 0)[ch(1, 0)] = *pixels.at(1, 0);
    out.at_mut(0, 0)[ch(1, 1)] = *pixels.at(1, 1);

    out.at_mut(0, dim.w - 1)[ch(0, dim.w - 1)] = *pixels.at(0, dim.w - 1);
    out.at_mut(0, dim.w - 1)[ch(0, dim.w - 2)] = *pixels.at(0, dim.w - 2);
    out.at_mut(0, dim.w - 1)[ch(1, dim.w - 1)] = *pixels.at(1, dim.w - 1);
    out.at_mut(0, dim.w - 1)[ch(1, dim.w - 2)] = *pixels.at(1, dim.w - 2);

    out.at_mut(dim.h - 1, 0)[ch(0, 0)] = *pixels.at(0, 0);
    out.at_mut(dim.h - 1, 0)[ch(0, 1)] = *pixels.at(0, 1);
    out.at_mut(dim.h - 1, 0)[ch(1, 0)] = *pixels.at(1, 0);
    out.at_mut(dim.h - 1, 0)[ch(1, 1)] = *pixels.at(1, 1);

    out.at_mut(dim.h - 1, dim.w - 1)[ch(dim.h - 1, dim.w - 1)] = *pixels.at(dim.h - 1, dim.w - 1);
    out.at_mut(dim.h - 1, dim.w - 1)[ch(dim.h - 1, dim.w - 2)] = *pixels.at(dim.h - 1, dim.w - 2);
    out.at_mut(dim.h - 1, dim.w - 1)[ch(dim.h - 2, dim.w - 1)] = *pixels.at(dim.h - 2, dim.w - 1);
    out.at_mut(dim.h - 1, dim.w - 1)[ch(dim.h - 2, dim.w - 2)] = *pixels.at(dim.h - 2, dim.w - 2);

    // Process borders
    for i in 1..out.width - 1 {
      // Top line
      out.at_mut(0, i)[ch(0, i)] = *pixels.at(0, i);
      out.at_mut(0, i)[ch(0, i + 1)] = (*pixels.at(0, i - 1) + *pixels.at(0, i + 1)) / 2.0;
      out.at_mut(0, i)[ch(1, i)] = *pixels.at(1, i);
      out.at_mut(0, i)[ch(1, i + 1)] = (*pixels.at(1, i - 1) + *pixels.at(1, i + 1)) / 2.0;
      // Bottom line
      out.at_mut(dim.h - 1, i)[ch(dim.h - 1, i)] = *pixels.at(dim.h - 1, i);
      out.at_mut(dim.h - 1, i)[ch(dim.h - 1, i + 1)] = (*pixels.at(dim.h - 1, i - 1) + *pixels.at(dim.h - 1, i + 1)) / 2.0;
      out.at_mut(dim.h - 1, i)[ch(dim.h - 2, i)] = *pixels.at(1, i);
      out.at_mut(dim.h - 1, i)[ch(dim.h - 2, i + 1)] = (*pixels.at(dim.h - 2, i - 1) + *pixels.at(dim.h - 2, i + 1)) / 2.0;
    }

    for i in 1..out.height - 1 {
      // Left
      out.at_mut(i, 0)[ch(i, 0)] = *pixels.at(i, 0);
      out.at_mut(i, 0)[ch(i + 1, 0)] = (*pixels.at(i - 1, 0) + *pixels.at(i + 1, 0)) / 2.0;
      out.at_mut(i, 0)[ch(i, 1)] = *pixels.at(i, 1);
      out.at_mut(i, 0)[ch(i + 1, 1)] = (*pixels.at(i - 1, 1) + *pixels.at(i + 1, 1)) / 2.0;
      // Right
      out.at_mut(i, dim.w - 1)[ch(i, dim.w - 1)] = *pixels.at(i, dim.w - 1);
      out.at_mut(i, dim.w - 1)[ch(i + 1, dim.w - 1)] = (*pixels.at(i - 1, dim.w - 1) + *pixels.at(i + 1, dim.w - 1)) / 2.0;
      out.at_mut(i, dim.w - 1)[ch(i, dim.w - 2)] = *pixels.at(i, dim.w - 2);
      out.at_mut(i, dim.w - 1)[ch(i + 1, dim.w - 2)] = (*pixels.at(i - 1, dim.w - 2) + *pixels.at(i + 1, dim.w - 2)) / 2.0;
    }

    /*
    A B A B
    C D C D
    A B A B
    C D C D
     */
    out.for_each_row(|row, pix| {
      if row == 0 || row == dim.h - 1 {
        return; // Skip border rows
      }
      for col in 1..dim.w - 1 {
        pix[col][ch(row, col)] = *pixels.at(row, col);
        pix[col][ch(row + 1, col)] = (*pixels.at(row - 1, col) + *pixels.at(row + 1, col)) / 2.0;
        pix[col][ch(row, col + 1)] = (*pixels.at(row, col - 1) + *pixels.at(row, col + 1)) / 2.0;
        pix[col][ch(row + 1, col + 1)] =
          (*pixels.at(row - 1, col - 1) + *pixels.at(row - 1, col + 1) + *pixels.at(row + 1, col - 1) + *pixels.at(row + 1, col + 1)) / 4.0;
      }
    });

    out
  }
}
