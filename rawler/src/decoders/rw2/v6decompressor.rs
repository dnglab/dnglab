/// Ported from Libraw

use crate::{
  decoders::*,
  pumps::{BitPump, BitPumpLSB},
};

/// This works for 12 and 14 bit depth images
pub(crate) fn decode_panasonic_v6(buf: &[u8], width: usize, height: usize, bps: u32, dummy: bool) -> PixU16 {
  log::debug!("width: {}", width);

  const V6_BYTES_PER_BLOCK: usize = 16;

  let pixels_per_block;

  let pixelbase0;
  let pixelbase_compare;
  let spix_compare;
  let pixel_mask;

  match bps {
    12 => {
      pixels_per_block = 14;
      pixelbase0 = 0x80;
      pixelbase_compare = 0x800;
      spix_compare = 0x3fff;
      pixel_mask = 0xfff;
    }
    14 => {
      pixels_per_block = 11;
      pixelbase0 = 0x200;
      pixelbase_compare = 0x2000;
      spix_compare = 0xffff;
      pixel_mask = 0x3fff;
    }
    _ => {
      unreachable!()
    }
  }

  let blocks_per_row = width / pixels_per_block;
  let bytes_per_row = V6_BYTES_PER_BLOCK * blocks_per_row;

  assert_eq!(width % pixels_per_block, 0);

  //log::debug!("RW2 V5 decoder: pixels per block: {}, bps: {}", pixels_per_block, bps);

  // We decode chunked at pixels_per_block boundary
  // Each block delivers the same amount of pixels.
  decode_threaded(
    width,
    height,
    dummy,
    &(|out, row| {
      let src = &buf[row * bytes_per_row..row * bytes_per_row + bytes_per_row];
      for (block_id, block) in src.chunks_exact(V6_BYTES_PER_BLOCK).enumerate() {
        let out = &mut out[block_id * pixels_per_block..];
        let mut pixelbuffer = [0_u16; 18];

        let mut pump = BitPumpLSB::new(block);

        match bps {
          14 => {
            // We fill from reverse, because bitstream is reversed
            pump.get_bits(4); // padding bits, ignore it
            pixelbuffer[13] = pump.get_bits(10) as u16;
            pixelbuffer[12] = pump.get_bits(10) as u16;
            pixelbuffer[11] = pump.get_bits(10) as u16;
            pixelbuffer[10] = pump.get_bits(2) as u16;
            pixelbuffer[9] = pump.get_bits(10) as u16;
            pixelbuffer[8] = pump.get_bits(10) as u16;
            pixelbuffer[7] = pump.get_bits(10) as u16;
            pixelbuffer[6] = pump.get_bits(2) as u16;
            pixelbuffer[5] = pump.get_bits(10) as u16;
            pixelbuffer[4] = pump.get_bits(10) as u16;
            pixelbuffer[3] = pump.get_bits(10) as u16;
            pixelbuffer[2] = pump.get_bits(2) as u16;
            pixelbuffer[1] = pump.get_bits(14) as u16;
            pixelbuffer[0] = pump.get_bits(14) as u16;
          }
          12 => {
            pixelbuffer[17] = pump.get_bits(8) as u16;
            pixelbuffer[16] = pump.get_bits(8) as u16;
            pixelbuffer[15] = pump.get_bits(8) as u16;
            pixelbuffer[14] = pump.get_bits(2) as u16;
            pixelbuffer[13] = pump.get_bits(8) as u16;
            pixelbuffer[12] = pump.get_bits(8) as u16;
            pixelbuffer[11] = pump.get_bits(8) as u16;
            pixelbuffer[10] = pump.get_bits(2) as u16;
            pixelbuffer[9] = pump.get_bits(8) as u16;
            pixelbuffer[8] = pump.get_bits(8) as u16;
            pixelbuffer[7] = pump.get_bits(8) as u16;
            pixelbuffer[6] = pump.get_bits(2) as u16;
            pixelbuffer[5] = pump.get_bits(8) as u16;
            pixelbuffer[4] = pump.get_bits(8) as u16;
            pixelbuffer[3] = pump.get_bits(8) as u16;
            pixelbuffer[2] = pump.get_bits(2) as u16;
            pixelbuffer[1] = pump.get_bits(12) as u16;
            pixelbuffer[0] = pump.get_bits(12) as u16;
          }
          _ => unreachable!(),
        }

        let mut curr_pixel = 0;

        let mut next_pixel = || -> u16 {
          curr_pixel += 1;
          pixelbuffer[curr_pixel - 1]
        };

        let mut oddeven = [0, 0];
        let mut nonzero = [0, 0];
        let mut pmul = 0;
        let mut pixel_base = 0;

        for pix in 0..pixels_per_block {
          if pix % 3 == 2 {
            let mut base = next_pixel();
            if base == 3 {
              base = 4;
            }
            pixel_base = pixelbase0 << base;
            pmul = 1 << base;
          }
          let mut epixel: u16 = next_pixel();
          if oddeven[pix % 2] != 0 {
            epixel *= pmul;
            if pixel_base < pixelbase_compare && nonzero[pix % 2] > pixel_base {
              epixel += nonzero[pix % 2] - pixel_base;
            }
            nonzero[pix % 2] = epixel;
          } else {
            oddeven[pix % 2] = epixel;
            if epixel != 0 {
              nonzero[pix % 2] = epixel;
            } else {
              epixel = nonzero[pix % 2];
            }
          }
          let spix = (epixel as i32).wrapping_sub(0xf);
          if spix <= spix_compare {
            out[pix] = (spix & spix_compare) as u16;
          } else {
            // FIXME: this is a convoluted way to compute zero.
            // What was this code trying to do, actually?
            epixel = ((epixel as i32).wrapping_add(0x7ffffff1) >> 0x1f) as u16;
            //epixel = static_cast<int>(epixel + 0x7ffffff1) >> 0x1f;
            //out(row, col) = epixel & 0x3fff;
            out[pix] = (epixel & pixel_mask) as u16;
          }
        }

        /*
        for (int pix = 0; pix < PanasonicV6Decompressor::PixelsPerBlock;
             pix++, col++) {
          if (pix % 3 == 2) {
            uint16_t base = page.nextpixel();
            if (base == 3)
              base = 4;
            pixel_base = 0x200 << base;
            pmul = 1 << base;
          }
          uint16_t epixel = page.nextpixel();
          if (oddeven[pix % 2]) {
            epixel *= pmul;
            if (pixel_base < 0x2000 && nonzero[pix % 2] > pixel_base)
              epixel += nonzero[pix % 2] - pixel_base;
            nonzero[pix % 2] = epixel;
          } else {
            oddeven[pix % 2] = epixel;
            if (epixel)
              nonzero[pix % 2] = epixel;
            else
              epixel = nonzero[pix % 2];
          }
          auto spix = static_cast<unsigned>(static_cast<int>(epixel) - 0xf);
          if (spix <= 0xffff)
            out(row, col) = spix & 0xffff;
          else {
            // FIXME: this is a convoluted way to compute zero.
            // What was this code trying to do, actually?
            epixel = static_cast<int>(epixel + 0x7ffffff1) >> 0x1f;
            out(row, col) = epixel & 0x3fff;
          }
        }
        */
      }
    }),
  )
}
