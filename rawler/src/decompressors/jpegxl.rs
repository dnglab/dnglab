// SPDX-License-Identifier: LGPL-2.1
// Copyright 2025 Daniel Vogelbacher <daniel@chaospixel.com>

use jxl_oxide::JxlImage;

use crate::decompressors::{Decompressor, LineIteratorMut};

pub struct JpegXLDecompressor {}

impl JpegXLDecompressor {
  pub fn new() -> Self {
    Self {}
  }
}

impl<'a> Decompressor<'a, u16> for JpegXLDecompressor {
  fn decompress(&self, src: &[u8], skip_rows: usize, lines: impl LineIteratorMut<'a, u16>, line_width: usize) -> std::result::Result<(), String> {
    let image = JxlImage::builder()
      .read(src)
      .map_err(|err| format!("Failed to read JPEG-XL image: {:?}", err))?;
    let frame = image.render_frame(0).map_err(|err| format!("Failed to render JPEG-XL image: {:?}", err))?;
    let all_ch = frame.image_all_channels();
    let pixbuf = all_ch.buf();
    for (line, buf) in lines.zip(pixbuf.chunks_exact(line_width).skip(skip_rows)) {
      for (p, f) in line.iter_mut().zip(buf.iter()) {
        *p = (f * u16::MAX as f32) as u16;
      }
    }
    Ok(())
  }
}
