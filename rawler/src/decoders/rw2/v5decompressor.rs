/// Ported from Libraw
use crate::{
  decoders::*,
  pumps::{BitPump, BitPumpLSB},
};

/// This works for 12 and 14 bit depth images
pub(crate) fn decode_panasonic_v5(buf: &[u8], width: usize, height: usize, bps: u32, dummy: bool) -> PixU16 {
  // Raw data is divided into blocks of same size
  const V5_BLOCK_SIZE: usize = 0x4000;
  // Each block is splitted and swapped, we need to swap back
  const V5_SECTION_SPLIT_OFFSET: usize = 0x1FF8;
  // Each block contains multiple packets
  const V5_BYTES_PER_PACKET: usize = 16;
  // Count of packets in a block
  const V5_PACKETS_PER_BLOCK: usize = V5_BLOCK_SIZE / V5_BYTES_PER_PACKET;
  // Depending on bit depth, a packet forms different amount of pixels
  let pixels_per_packet = match bps {
    12 => 10,
    14 => 9,
    _ => unreachable!(),
  };
  // Pixel count per full block
  let pixels_per_block = V5_PACKETS_PER_BLOCK * pixels_per_packet;

  log::debug!("RW2 V5 decoder: pixels per block: {}, bps: {}", pixels_per_block, bps);

  // We decode chunked at pixels_per_block boundary
  // Each block delivers the same amount of pixels.
  decode_threaded_chunked(
    width,
    height,
    pixels_per_block,
    dummy,
    &(|chunk, block_id| {
      // Block offset
      let src = &buf[block_id * V5_BLOCK_SIZE..block_id * V5_BLOCK_SIZE + V5_BLOCK_SIZE];
      // Now swap the two parts of the block
      let mut swapped = Vec::with_capacity(V5_BLOCK_SIZE);
      swapped.extend_from_slice(&src[V5_SECTION_SPLIT_OFFSET..]);
      swapped.extend_from_slice(&src[..V5_SECTION_SPLIT_OFFSET]);
      // Transform the packets into final pixels
      for (out, bytes) in chunk.chunks_exact_mut(pixels_per_packet).zip(swapped.chunks_exact(V5_BYTES_PER_PACKET)) {
        // The packet is a bitstream in LSB order.
        let mut pump = BitPumpLSB::new(bytes);
        out.iter_mut().for_each(|p| *p = pump.get_bits(bps) as u16);
      }
    }),
  )
}
