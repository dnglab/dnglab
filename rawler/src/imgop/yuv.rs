// SPDX-License-Identifier: LGPL-2.1
// Copyright 2022 Daniel Vogelbacher <daniel@chaospixel.com>

use crate::bits::clampbits;
use rayon::prelude::*;

/// Color conversion for Y Cb Cr to RGB
///
/// Matrix source: https://web.archive.org/web/20180421030430/http://www.equasys.de/colorconversion.html
pub fn ycbcr_to_rgb(buf: &mut [u16]) {
  let cpp = 3;
  assert_eq!(buf.len() % cpp, 0, "pixel buffer must contains 3 samples/pixel");

  // Correction value to bring cb and cr into range.
  // This assumes the range is 2^15. For a 2^8 range this would be 128.
  let corr: i32 = 16383;
  //let corr: i32 = (16383 << 1) | 1;
  buf.par_chunks_exact_mut(cpp).for_each(|pix| {
    let y = pix[0] as f32;
    let cb = (pix[1] as i32 - corr) as f32;
    let cr = (pix[2] as i32 - corr) as f32;
    //let cb = 100.0;
    //let cr = 100.0;
    let r = y + 1.4 * cr;
    let g = y + (-0.343 * cb) + (-0.711 * cr);
    let b = y + 1.765 * cb;
    pix[0] = clampbits(r as i32, 16);
    pix[1] = clampbits(g as i32, 16);
    pix[2] = clampbits(b as i32, 16);
  })
}

/// Interpolate YCbCr (YUV) data
pub fn interpolate_yuv(super_h: usize, super_v: usize, width: usize, _height: usize, image: &mut [u16]) {
  if super_h == 1 && super_v == 1 {
    return; // No interpolation needed
  }
  // Iterate over a block of 3 rows, smaller chunks are okay
  // but must always a multiple of row width.
  image.par_chunks_mut(width * 3).for_each(|slice| {
    // Do horizontal interpolation.
    // [y1 Cb Cr ] [ y2 . . ] [y1 Cb Cr ] [ y2 . . ] ...
    if super_h == 2 {
      debug_assert_eq!(slice.len() % width, 0);
      for row in 0..(slice.len() / width) {
        for col in (6..width).step_by(6) {
          let pix1 = row * width + col - 6;
          let pix2 = pix1 + 3;
          let pix3 = row * width + col;
          slice[pix2 + 1] = ((slice[pix1 + 1] as i32 + slice[pix3 + 1] as i32 + 1) / 2) as u16;
          slice[pix2 + 2] = ((slice[pix1 + 2] as i32 + slice[pix3 + 2] as i32 + 1) / 2) as u16;
        }
      }
    }
    // Do vertical interpolation
    //          pixel n      pixel n+1       pixel n+2    pixel n+3       ...
    // row i  : [y1 Cb  Cr ] [ y2 Cb*  Cr* ] [y1 Cb  Cr ] [ y2 Cb*  Cr* ] ...
    // row i+1: [y3 Cb* Cr*] [ y4 Cb** Cr**] [y3 Cb* Cr*] [ y4 Cb** Cr**] ...
    // row i+2: [y1 Cb  Cr ] [ y2 Cb*  Cr* ] [y1 Cb  Cr ] [ y2 Cb*  Cr* ] ...
    // row i+3: [y3 Cb* Cr*] [ y4 Cb** Cr**] [y3 Cb* Cr*] [ y4 Cb** Cr**] ...
    if super_v == 2 && slice.len() == width * 3 {
      for col in (0..width).step_by(3) {
        let pix1 = col;
        let pix2 = width + col;
        let pix3 = 2 * width + col;
        slice[pix2 + 1] = ((slice[pix1 + 1] as i32 + slice[pix3 + 1] as i32 + 1) / 2) as u16;
        slice[pix2 + 2] = ((slice[pix1 + 2] as i32 + slice[pix3 + 2] as i32 + 1) / 2) as u16;
      }
    }
  });
}
