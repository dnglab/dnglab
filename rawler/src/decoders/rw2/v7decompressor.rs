/// Ported from Libraw

use crate::{
  decoders::*,
  pumps::{BitPump, BitPumpLSB},
};

/// This works for 12 and 14 bit depth images
pub(crate) fn decode_panasonic_v7(buf: &[u8], width: usize, height: usize, bps: u32, dummy: bool) -> PixU16 {
  const V7_BYTES_PER_BLOCK: usize = 16;

  let pixels_per_block = match bps {
    14 => 9,
    12 => 10,
    _ => unreachable!(),
  };
  let blocks_per_row = width / pixels_per_block;

  assert_eq!(width % pixels_per_block, 0);

  let bytes_per_row = V7_BYTES_PER_BLOCK * blocks_per_row;

  decode_threaded(
    width,
    height,
    dummy,
    &(|out, row| {
      let src = &buf[row * bytes_per_row..row * bytes_per_row + bytes_per_row];
      for (block_id, block) in src.chunks_exact(V7_BYTES_PER_BLOCK).enumerate() {
        let start = block_id * pixels_per_block;
        let out = &mut out[start..start + pixels_per_block];
        let mut pump = BitPumpLSB::new(block);
        out.iter_mut().for_each(|pixel| *pixel = pump.get_bits(14) as u16);
      }
    }),
  )
}
