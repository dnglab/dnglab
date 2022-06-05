use crate::{alloc_image, bits::*, decoders::decode_threaded, pixarray::PixU16};

pub fn decode_8bit_wtable(buf: &[u8], tbl: &LookupTable, width: usize, height: usize, dummy: bool) -> PixU16 {
  decode_threaded(
    width,
    height,
    dummy,
    &(|out: &mut [u16], row| {
      let inb = &buf[(row * width)..];
      let mut random = LEu32(inb, 0);

      for (o, i) in out.chunks_exact_mut(1).zip(inb.chunks_exact(1)) {
        o[0] = tbl.dither(i[0] as u16, &mut random);
      }
    }),
  )
}

pub fn decode_10le_lsb16(buf: &[u8], width: usize, height: usize, dummy: bool) -> PixU16 {
  decode_threaded(
    width,
    height,
    dummy,
    &(|out: &mut [u16], row| {
      let inb = &buf[(row * width * 10 / 8)..];

      for (o, i) in out.chunks_exact_mut(8).zip(inb.chunks_exact(10)) {
        let g1: u16 = i[0] as u16;
        let g2: u16 = i[1] as u16;
        let g3: u16 = i[2] as u16;
        let g4: u16 = i[3] as u16;
        let g5: u16 = i[4] as u16;
        let g6: u16 = i[5] as u16;
        let g7: u16 = i[6] as u16;
        let g8: u16 = i[7] as u16;
        let g9: u16 = i[8] as u16;
        let g10: u16 = i[9] as u16;

        o[0] = g2 << 2 | g1 >> 6;
        o[1] = (g1 & 0x3f) << 4 | g4 >> 4;
        o[2] = (g4 & 0x0f) << 6 | g3 >> 2;
        o[3] = (g3 & 0x03) << 8 | g6;
        o[4] = g5 << 2 | g8 >> 6;
        o[5] = (g8 & 0x3f) << 4 | g7 >> 4;
        o[6] = (g7 & 0x0f) << 6 | g10 >> 2;
        o[7] = (g10 & 0x03) << 8 | g9;
      }
    }),
  )
}

pub fn decode_10le(buf: &[u8], width: usize, height: usize, dummy: bool) -> PixU16 {
  decode_threaded(
    width,
    height,
    dummy,
    &(|out: &mut [u16], row| {
      let inb = &buf[(row * width * 10 / 8)..];

      for (o, i) in out.chunks_exact_mut(4).zip(inb.chunks_exact(5)) {
        let g1: u16 = i[0] as u16;
        let g2: u16 = i[1] as u16;
        let g3: u16 = i[2] as u16;
        let g4: u16 = i[3] as u16;
        let g5: u16 = i[4] as u16;

        o[0] = g1 << 2 | g2 >> 6;
        o[1] = (g2 & 0x3f) << 4 | g3 >> 4;
        o[2] = (g3 & 0x0f) << 6 | g3 >> 2;
        o[3] = (g4 & 0x03) << 8 | g5;
      }
    }),
  )
}

pub fn decode_12be(buf: &[u8], width: usize, height: usize, dummy: bool) -> PixU16 {
  decode_threaded(
    width,
    height,
    dummy,
    &(|out: &mut [u16], row| {
      let inb = &buf[(row * width * 12 / 8)..];

      for (o, i) in out.chunks_exact_mut(2).zip(inb.chunks_exact(3)) {
        let g1: u16 = i[0] as u16;
        let g2: u16 = i[1] as u16;
        let g3: u16 = i[2] as u16;

        o[0] = (g1 << 4) | (g2 >> 4);
        o[1] = ((g2 & 0x0f) << 8) | g3;
      }
    }),
  )
}

pub fn decode_12be_msb16(buf: &[u8], width: usize, height: usize, dummy: bool) -> PixU16 {
  let mut out = alloc_image!(width, height, dummy);

  for (o, i) in out.pixels_mut().chunks_exact_mut(4).zip(buf.chunks_exact(6)) {
    let g1: u16 = i[0] as u16;
    let g2: u16 = i[1] as u16;
    let g3: u16 = i[2] as u16;
    let g4: u16 = i[3] as u16;
    let g5: u16 = i[4] as u16;
    let g6: u16 = i[5] as u16;

    o[0] = (g2 << 4) | (g1 >> 4);
    o[1] = ((g1 & 0x0f) << 8) | g4;
    o[2] = (g3 << 4) | (g6 >> 4);
    o[3] = ((g6 & 0x0f) << 8) | g5;
  }
  out
}

pub fn decode_12le_16bitaligned(buf: &[u8], width: usize, height: usize, dummy: bool) -> PixU16 {
  let stride = ((width * 12 / 8 + 1) >> 1) << 1;
  decode_threaded(
    width,
    height,
    dummy,
    &(|out: &mut [u16], row| {
      let inb = &buf[row * stride..];
      for (o, i) in out.chunks_exact_mut(2).zip(inb.chunks_exact(3)) {
        let g1: u16 = i[0] as u16;
        let g2: u16 = i[1] as u16;
        let g3: u16 = i[2] as u16;

        o[0] = (g1 << 4) | (g2 >> 4);
        o[1] = (g2 & 0x0f) << 8 | g3;
      }
    }),
  )
}

pub fn decode_12be_msb32(buf: &[u8], width: usize, height: usize, dummy: bool) -> PixU16 {
  let mut out = alloc_image!(width, height, dummy);

  for (o, i) in out.pixels_mut().chunks_exact_mut(8).zip(buf.chunks_exact(12)) {
    let g1: u16 = i[0] as u16;
    let g2: u16 = i[1] as u16;
    let g3: u16 = i[2] as u16;
    let g4: u16 = i[3] as u16;
    let g5: u16 = i[4] as u16;
    let g6: u16 = i[5] as u16;
    let g7: u16 = i[6] as u16;
    let g8: u16 = i[7] as u16;
    let g9: u16 = i[8] as u16;
    let g10: u16 = i[9] as u16;
    let g11: u16 = i[10] as u16;
    let g12: u16 = i[11] as u16;

    // | G1 | G2 | G3 | G4 | G5 | G6 | G7 | G8 | G9 | G10 | G11 | G12 |
    //    2    1   1    0    4    4     3   2     7    6     6     5

    o[0] = (g4 << 4) | (g3 >> 4);
    o[1] = ((g3 & 0x0f) << 8) | g2;
    o[2] = (g1 << 4) | (g8 >> 4);
    o[3] = ((g8 & 0x0f) << 8) | g7;
    o[4] = (g6 << 4) | (g5 >> 4);
    o[5] = ((g5 & 0x0f) << 8) | g12;
    o[6] = (g11 << 4) | (g10 >> 4);
    o[7] = ((g10 & 0x0f) << 8) | g9;
  }
  out
}

/// This is packed by 32 bits in reverse order, so the concatenation
/// of 14 bit pairs is byte index: 3, 2, 1, 0, 7, 6, 5, 4, ...
pub fn decode_14be_msb32(buf: &[u8], width: usize, height: usize, dummy: bool) -> PixU16 {
  let mut out = alloc_image!(width, height, dummy);

  for (o, i) in out.pixels_mut().chunks_exact_mut(16).zip(buf.chunks_exact(28)) {
    let g1: u16 = i[0] as u16;
    let g2: u16 = i[1] as u16;
    let g3: u16 = i[2] as u16;
    let g4: u16 = i[3] as u16;

    let g5: u16 = i[4] as u16;
    let g6: u16 = i[5] as u16;
    let g7: u16 = i[6] as u16;
    let g8: u16 = i[7] as u16;

    let g9: u16 = i[8] as u16;
    let g10: u16 = i[9] as u16;
    let g11: u16 = i[10] as u16;
    let g12: u16 = i[11] as u16;

    let g13: u16 = i[12] as u16;
    let g14: u16 = i[13] as u16;
    let g15: u16 = i[14] as u16;
    let g16: u16 = i[15] as u16;

    let g17: u16 = i[16] as u16;
    let g18: u16 = i[17] as u16;
    let g19: u16 = i[18] as u16;
    let g20: u16 = i[19] as u16;

    let g21: u16 = i[20] as u16;
    let g22: u16 = i[21] as u16;
    let g23: u16 = i[22] as u16;
    let g24: u16 = i[23] as u16;

    let g25: u16 = i[24] as u16;
    let g26: u16 = i[25] as u16;
    let g27: u16 = i[26] as u16;
    let g28: u16 = i[27] as u16;

    o[0] = (g4 << 6) | (g3 >> 2);
    o[1] = ((g3 & 0x3) << 12) | (g2 << 4) | (g1 >> 4);
    o[2] = ((g1 & 0xf) << 10) | (g8 << 2) | (g7 >> 6);

    o[3] = ((g7 & 0x3f) << 8) | g6;
    o[4] = (g5 << 6) | (g12 >> 2);

    o[5] = ((g12 & 0x3) << 12) | (g11 << 4) | (g10 >> 4);
    o[6] = ((g10 & 0xf) << 10) | (g9 << 2) | (g16 >> 6);

    o[7] = ((g16 & 0x3f) << 8) | g15;
    o[8] = (g14 << 6) | (g13 >> 2);

    o[9] = ((g13 & 0x3) << 12) | (g20 << 4) | (g19 >> 4);
    o[10] = ((g19 & 0xf) << 10) | (g18 << 2) | (g17 >> 6);

    o[11] = ((g17 & 0x3f) << 8) | g24;
    o[12] = (g23 << 6) | (g22 >> 2);

    o[13] = ((g22 & 0x3) << 12) | (g21 << 4) | (g28 >> 4);
    o[14] = ((g28 & 0xf) << 10) | (g27 << 2) | (g26 >> 6);

    o[15] = ((g26 & 0x3f) << 8) | g25;
  }

  out
}

pub fn decode_12le_wcontrol(buf: &[u8], width: usize, height: usize, dummy: bool) -> PixU16 {
  // Calulate expected bytes per line.
  let perline = width * 12 / 8 + ((width + 2) / 10);

  decode_threaded(
    width,
    height,
    dummy,
    &(|out: &mut [u16], row| {
      let inb = &buf[(row * perline)..];

      for (oc, ic) in out.chunks_exact_mut(10).zip(inb.chunks_exact(16)) {
        for (o, i) in oc.chunks_exact_mut(2).zip(ic.chunks_exact(3)) {
          let g1: u16 = i[0] as u16;
          let g2: u16 = i[1] as u16;
          let g3: u16 = i[2] as u16;

          o[0] = ((g2 & 0x0f) << 8) | g1;
          o[1] = (g3 << 4) | (g2 >> 4);
        }
      }
    }),
  )
}

pub fn decode_12be_wcontrol(buf: &[u8], width: usize, height: usize, dummy: bool) -> PixU16 {
  // Calulate expected bytes per line.
  let perline = width * 12 / 8 + ((width + 2) / 10);

  decode_threaded(
    width,
    height,
    dummy,
    &(|out: &mut [u16], row| {
      let inb = &buf[(row * perline)..];

      for (oc, ic) in out.chunks_exact_mut(10).zip(inb.chunks_exact(16)) {
        for (o, i) in oc.chunks_exact_mut(2).zip(ic.chunks_exact(3)) {
          let g1: u16 = i[0] as u16;
          let g2: u16 = i[1] as u16;
          let g3: u16 = i[2] as u16;

          o[0] = (g1 << 4) | (g2 >> 4);
          o[1] = ((g2 & 0x0f) << 8) | g3;
        }
      }
    }),
  )
}

pub fn decode_12be_interlaced(buf: &[u8], width: usize, height: usize, dummy: bool) -> PixU16 {
  let half = (height + 1) >> 1;
  // Second field is 2048 byte aligned
  let second_field_offset = (((half * width * 3 / 2) >> 11) + 1) << 11;
  let second_field = &buf[second_field_offset..];

  decode_threaded(
    width,
    height,
    dummy,
    &(|out: &mut [u16], row| {
      let off = row / 2 * width * 12 / 8;
      let inb = if (row % 2) == 0 { &buf[off..] } else { &second_field[off..] };

      for (o, i) in out.chunks_exact_mut(2).zip(inb.chunks_exact(3)) {
        let g1: u16 = i[0] as u16;
        let g2: u16 = i[1] as u16;
        let g3: u16 = i[2] as u16;

        o[0] = (g1 << 4) | (g2 >> 4);
        o[1] = ((g2 & 0x0f) << 8) | g3;
      }
    }),
  )
}

pub fn decode_12be_interlaced_unaligned(buf: &[u8], width: usize, height: usize, dummy: bool) -> PixU16 {
  let half = (height + 1) >> 1;
  let second_field = &buf[half * width * 12 / 8..];

  decode_threaded(
    width,
    height,
    dummy,
    &(|out: &mut [u16], row| {
      let off = row / 2 * width * 12 / 8;
      let inb = if (row % 2) == 0 { &buf[off..] } else { &second_field[off..] };

      for (o, i) in out.chunks_exact_mut(2).zip(inb.chunks_exact(3)) {
        let g1: u16 = i[0] as u16;
        let g2: u16 = i[1] as u16;
        let g3: u16 = i[2] as u16;

        o[0] = (g1 << 4) | (g2 >> 4);
        o[1] = ((g2 & 0x0f) << 8) | g3;
      }
    }),
  )
}

pub fn decode_12le(buf: &[u8], width: usize, height: usize, dummy: bool) -> PixU16 {
  decode_threaded(
    width,
    height,
    dummy,
    &(|out: &mut [u16], row| {
      let inb = &buf[(row * width * 12 / 8)..];

      for (o, i) in out.chunks_exact_mut(2).zip(inb.chunks_exact(3)) {
        let g1: u16 = i[0] as u16;
        let g2: u16 = i[1] as u16;
        let g3: u16 = i[2] as u16;

        o[0] = ((g2 & 0x0f) << 8) | g1;
        o[1] = (g3 << 4) | (g2 >> 4);
      }
    }),
  )
}

pub fn decode_12le_padded(buf: &[u8], width: usize, height: usize, stripesize: usize, dummy: bool) -> PixU16 {
  decode_threaded(
    width,
    height,
    dummy,
    &(|out: &mut [u16], row| {
      let inb = &buf[(row * stripesize)..];

      for (o, i) in out.chunks_exact_mut(2).zip(inb.chunks_exact(3)) {
        let g1: u16 = i[0] as u16;
        let g2: u16 = i[1] as u16;
        let g3: u16 = i[2] as u16;

        o[0] = ((g2 & 0x0f) << 8) | g1;
        o[1] = (g3 << 4) | (g2 >> 4);
      }
    }),
  )
}

pub fn decode_14le_padded(buf: &[u8], width: usize, height: usize, stripesize: usize, dummy: bool) -> PixU16 {
  decode_threaded(
    width,
    height,
    dummy,
    &(|out: &mut [u16], row| {
      let inb = &buf[(row * stripesize)..];

      for (o, i) in out.chunks_exact_mut(4).zip(inb.chunks_exact(7)) {
        let g1: u16 = i[0] as u16;
        let g2: u16 = i[1] as u16;
        let g3: u16 = i[2] as u16;
        let g4: u16 = i[3] as u16;
        let g5: u16 = i[4] as u16;
        let g6: u16 = i[5] as u16;
        let g7: u16 = i[6] as u16;
        o[0] = ((g2 & 0x3f) << 8) | g1;
        o[1] = ((g4 & 0xf) << 10) | (g3 << 2) | (g2 >> 6);
        o[2] = ((g6 & 0x3) << 12) | (g5 << 4) | (g4 >> 4);
        o[3] = (g7 << 6) | (g6 >> 2);
      }
    }),
  )
}

pub fn decode_12le_unpacked(buf: &[u8], width: usize, height: usize, dummy: bool) -> PixU16 {
  decode_threaded(
    width,
    height,
    dummy,
    &(|out: &mut [u16], row| {
      let inb = &buf[(row * width * 2)..];

      for (i, bytes) in (0..width).zip(inb.chunks_exact(2)) {
        out[i] = LEu16(bytes, 0) & 0x0fff;
      }
    }),
  )
}

pub fn decode_12be_unpacked(buf: &[u8], width: usize, height: usize, dummy: bool) -> PixU16 {
  decode_threaded(
    width,
    height,
    dummy,
    &(|out: &mut [u16], row| {
      let inb = &buf[(row * width * 2)..];

      for (i, bytes) in (0..width).zip(inb.chunks_exact(2)) {
        out[i] = BEu16(bytes, 0) & 0x0fff;
      }
    }),
  )
}

pub fn decode_12be_unpacked_left_aligned(buf: &[u8], width: usize, height: usize, dummy: bool) -> PixU16 {
  decode_threaded(
    width,
    height,
    dummy,
    &(|out: &mut [u16], row| {
      let inb = &buf[(row * width * 2)..];

      for (i, bytes) in (0..width).zip(inb.chunks_exact(2)) {
        out[i] = BEu16(bytes, 0) >> 4;
      }
    }),
  )
}

pub fn decode_12le_unpacked_left_aligned(buf: &[u8], width: usize, height: usize, dummy: bool) -> PixU16 {
  decode_threaded(
    width,
    height,
    dummy,
    &(|out: &mut [u16], row| {
      let inb = &buf[(row * width * 2)..];

      for (i, bytes) in (0..width).zip(inb.chunks_exact(2)) {
        out[i] = LEu16(bytes, 0) >> 4;
      }
    }),
  )
}

pub fn decode_14le_unpacked(buf: &[u8], width: usize, height: usize, dummy: bool) -> PixU16 {
  decode_threaded(
    width,
    height,
    dummy,
    &(|out: &mut [u16], row| {
      let inb = &buf[(row * width * 2)..];

      for (i, bytes) in (0..width).zip(inb.chunks_exact(2)) {
        out[i] = LEu16(bytes, 0) & 0x3fff;
      }
    }),
  )
}

pub fn decode_14le_unpacked_padded(buf: &[u8], width: usize, height: usize, stripsize: usize, dummy: bool) -> PixU16 {
  decode_threaded(
    width,
    height,
    dummy,
    &(|out: &mut [u16], row| {
      let inb = &buf[(row * stripsize)..];

      for (i, bytes) in (0..width).zip(inb.chunks_exact(2)) {
        out[i] = LEu16(bytes, 0) & 0x3fff;
      }
    }),
  )
}

pub fn decode_14be_unpacked(buf: &[u8], width: usize, height: usize, dummy: bool) -> PixU16 {
  decode_threaded(
    width,
    height,
    dummy,
    &(|out: &mut [u16], row| {
      let inb = &buf[(row * width * 2)..];

      for (i, bytes) in (0..width).zip(inb.chunks_exact(2)) {
        out[i] = BEu16(bytes, 0) & 0x3fff;
      }
    }),
  )
}

pub fn decode_16le(buf: &[u8], width: usize, height: usize, dummy: bool) -> PixU16 {
  decode_threaded(
    width,
    height,
    dummy,
    &(|out: &mut [u16], row| {
      let inb = &buf[(row * width * 2)..];

      for (i, bytes) in (0..width).zip(inb.chunks_exact(2)) {
        out[i] = LEu16(bytes, 0);
      }
    }),
  )
}

pub fn decode_16le_skiplines(buf: &[u8], width: usize, height: usize, dummy: bool) -> PixU16 {
  decode_threaded(
    width,
    height,
    dummy,
    &(|out: &mut [u16], row| {
      let inb = &buf[(row * width * 4)..];

      for (i, bytes) in (0..width).zip(inb.chunks_exact(2)) {
        out[i] = LEu16(bytes, 0);
      }
    }),
  )
}

pub fn decode_16be(buf: &[u8], width: usize, height: usize, dummy: bool) -> PixU16 {
  decode_threaded(
    width,
    height,
    dummy,
    &(|out: &mut [u16], row| {
      let inb = &buf[(row * width * 2)..];

      for (i, bytes) in (0..width).zip(inb.chunks_exact(2)) {
        out[i] = BEu16(bytes, 0);
      }
    }),
  )
}
