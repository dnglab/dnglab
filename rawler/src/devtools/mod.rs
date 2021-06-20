// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use image::{ImageBuffer, ImageFormat, Luma};

pub fn dump_image_u16(data: &Vec<u16>, width: usize, height: usize, path: impl AsRef<str>) {
  let img = ImageBuffer::<Luma<u16>, Vec<u16>>::from_vec(width as u32, height as u32, data.clone()).unwrap();
  img.save_with_format(path.as_ref(), ImageFormat::Tiff).unwrap();
}
