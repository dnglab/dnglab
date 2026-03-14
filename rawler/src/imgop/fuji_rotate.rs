// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

use crate::imgop::Dim2;
use crate::pixarray::Color2D;

/// Rotates a Fujifilm sensor image by 45° CW to correct the
/// 45° CCW rotation of the Super CCD / X-Trans sensor layout.
///
/// `fuji_rotation_width` is the split point T used to compute the inscribed
/// rectangle dimensions after rotation.
pub(crate) fn fuji_normalize_rotation(src: &Color2D<f32, 3>, fuji_rotation_width: usize, extra_rotate: bool) -> Color2D<f32, 3> {
  let src = if extra_rotate {
    // These camera needs an extra rotate 90° CW to align with
    // other cameras.
    &src.rotate_90cw()
  } else {
    src
  };
  rotate_45cw(src, fuji_rotation_width)
}

/// Calculate the final image dimension after correcting rotation.
pub(crate) fn fuji_calc_dimension(width: usize, fuji_rotation_width: usize) -> Dim2 {
  // formula: T = fuji_rotation_width
  // crop_w = T * √2, crop_h = (S - T) * √2, where S ≈ src_w
  let t = fuji_rotation_width as f64;
  let s = width as f64;
  let crop_w = (t * std::f64::consts::SQRT_2).floor() as usize;
  let crop_h = ((s - t) * std::f64::consts::SQRT_2).floor() as usize;
  if crop_w > crop_h {
    Dim2::new(crop_w, crop_h)
  } else {
    Dim2::new(crop_h, crop_w) // Fuji rotate_alt
  }
}

/// Rotate a Color2D<f32, 3> image 45 degrees clockwise using bilinear interpolation,
/// and crop to the inscribed rectangle.
fn rotate_45cw(src: &Color2D<f32, 3>, fuji_rotation_width: usize) -> Color2D<f32, 3> {
  let src_w = src.width;
  let src_h = src.height;
  let inv_sqrt2: f64 = std::f64::consts::FRAC_1_SQRT_2;

  let src_cx = src_w as f64 / 2.0;
  let src_cy = src_h as f64 / 2.0;

  let Dim2 { w: crop_w, h: crop_h } = fuji_calc_dimension(src_w, fuji_rotation_width);

  let crop_cx = crop_w as f64 / 2.0;
  let crop_cy = crop_h as f64 / 2.0;

  let mut dst = Color2D::<f32, 3>::new(crop_w, crop_h);

  for row in 0..crop_h {
    for col in 0..crop_w {
      // Position relative to center (crop is centered in the rotated square)
      let dx = col as f64 - crop_cx;
      let dy = row as f64 - crop_cy;

      // Inverse of 45° CW = 45° CCW rotation to find source coordinates
      let src_x = (dx + dy) * inv_sqrt2 + src_cx;
      let src_y = (dy - dx) * inv_sqrt2 + src_cy;

      // Nearest-neighbor check for overall bounds
      let sx = src_x.round() as isize;
      let sy = src_y.round() as isize;
      if sx < 0 || sx >= src_w as isize || sy < 0 || sy >= src_h as isize {
        continue;
      }

      let x0 = src_x.floor() as isize;
      let y0 = src_y.floor() as isize;
      let x1 = x0 + 1;
      let y1 = y0 + 1;

      let pixel = dst.at_mut(row, col);
      if x0 < 0 || y0 < 0 || x1 >= src_w as isize || y1 >= src_h as isize {
        // Nearest-neighbor at boundaries
        *pixel = *src.at(sy as usize, sx as usize);
      } else {
        // Bilinear interpolation
        let fx = (src_x - x0 as f64) as f32;
        let fy = (src_y - y0 as f64) as f32;

        let p00 = src.at(y0 as usize, x0 as usize);
        let p10 = src.at(y0 as usize, x1 as usize);
        let p01 = src.at(y1 as usize, x0 as usize);
        let p11 = src.at(y1 as usize, x1 as usize);

        for ch in 0..3 {
          pixel[ch] = p00[ch] * (1.0 - fx) * (1.0 - fy) // keep
            + p10[ch] * fx * (1.0 - fy) // keep
            + p01[ch] * (1.0 - fx) * fy // keep
            + p11[ch] * fx * fy;
        }
      }
    }
  }

  dst
}
