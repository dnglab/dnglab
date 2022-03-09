// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::{fs::File, io::{BufWriter, Write}};

use byteorder::{LittleEndian, WriteBytesExt};
use image::{ImageBuffer, ImageFormat, Luma, Rgb};
pub(crate) mod inspector;

pub fn dump_image_u16(data: &[u16], width: usize, height: usize, path: impl AsRef<str>) {
  let img = ImageBuffer::<Luma<u16>, Vec<u16>>::from_vec(width as u32, height as u32, data.to_vec()).unwrap();
  img.save_with_format(path.as_ref(), ImageFormat::Tiff).unwrap();
}

pub fn dump_image_u16_rgb(data: &[u16], width: usize, height: usize, path: impl AsRef<str>) {
  let img = ImageBuffer::<Rgb<u16>, Vec<u16>>::from_vec(width as u32, height as u32, data.to_vec()).unwrap();
  img.save_with_format(path.as_ref(), ImageFormat::Tiff).unwrap();
}

pub fn dump_buf<T>(path: &str, buf: T) where T: AsRef<[u8]> {
  let mut f = BufWriter::new(File::create(path).expect("Unable to create file"));
  f.write_all(buf.as_ref()).expect("Failed to dump buffer to file");
  f.flush().expect("Failed to flush file");
}

pub fn dump_buf_u16<T>(path: &str, buf: T) where T: AsRef<[u16]> {
  let mut f = BufWriter::new(File::create(path).expect("Unable to create file"));
  for v in buf.as_ref().iter() {
    f.write_u16::<LittleEndian>(*v).expect("Failed to dump buffer to file");
  }
  f.flush().expect("Failed to flush file");
}
