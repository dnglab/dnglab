///
/// Original code by libraw and rawspeed, licensed under LGPL-2
///
/// Copyright (C) 2016 Alexey Danilchenko
/// Copyright (C) 2016 Alex Tutubalin
use byteorder::{BigEndian, ReadBytesExt};
use std::io::Cursor;

use crate::Result;
use crate::{alloc_image_ok, pixarray::PixU16};

pub(super) fn decode_dbp(buf: &[u8], width: usize, height: usize, dummy: bool) -> Result<PixU16> {
  let mut out = alloc_image_ok!(width, height, dummy);
  let mut cursor = Cursor::new(buf);
  let n_tiles = 8;
  let tile_width = width / n_tiles;
  let _tile_height = 3856;
  log::debug!("DBP width: {}, height: {}, tile: {}", width, height, tile_width);

  let mut tile_buffer = vec![0_u16; height * tile_width];

  for tile_n in 0..n_tiles {
    cursor.read_u16_into::<BigEndian>(&mut tile_buffer)?;
    for scan_line in 0..height {
      let off = scan_line * width + tile_n * tile_width;
      out[off..off + tile_width].copy_from_slice(&tile_buffer[scan_line * tile_width..scan_line * tile_width + tile_width]);
    }
  }
  Ok(out)
}
