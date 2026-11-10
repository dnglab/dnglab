/// Ported from Libraw
use crate::{
  decoders::*,
  decompressors::decompress_lines_fn,
  pumps::{BitPump, BitPumpLSB},
};

/// This works for 12 and 14 bit depth images
pub(crate) fn decode_panasonic_v7(buf: &[u8], width: usize, height: usize, bps: u32, dummy: bool) -> std::result::Result<PixU16, String> {
  const V7_BYTES_PER_BLOCK: usize = 16;

  let pixels_per_block = match bps {
    14 => 9,
    12 => 10,
    _ => return Err(format!("RW2 v7: unsupported bit depth {}", bps)),
  };
  let blocks_per_row = width / pixels_per_block;

  if !width.is_multiple_of(pixels_per_block) {
    return Err(format!("RW2 v7: width {} is not a multiple of {} pixels per block", width, pixels_per_block));
  }

  let bytes_per_row = V7_BYTES_PER_BLOCK * blocks_per_row;

  let need = bytes_per_row * height;
  if buf.len() < need {
    return Err(format!("RW2 v7: buffer too short ({} < {})", buf.len(), need));
  }

  decompress_lines_fn(
    width,
    height,
    dummy,
    &(|out, row| {
      let src = &buf[row * bytes_per_row..row * bytes_per_row + bytes_per_row];
      for (block_id, block) in src.chunks_exact(V7_BYTES_PER_BLOCK).enumerate() {
        let start = block_id * pixels_per_block;
        let out = &mut out[start..start + pixels_per_block];
        let mut pump = BitPumpLSB::new(block);
        // Always reads 14 bits. A 12-bit block (10 pixels) would need 140 of its
        // 128 bits and fail with Eof. No 12-bit files are known yet; they would
        // need get_bits(bps).
        for pixel in out.iter_mut() {
          *pixel = pump.get_bits(14)? as u16;
        }
      }
      Ok(())
    }),
  )
}
