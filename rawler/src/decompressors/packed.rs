use multiversion::multiversion;

use crate::bits::{BEf16, BEf24, BEf32, BEu16, LEf16, LEf24, LEf32, LEu16};
use crate::decompressors::LineIteratorMut;
use crate::pumps::{BitPump, BitPumpMSB};
use crate::{bits::Endian, decompressors::Decompressor};

/// Decompressor for packed data
pub struct PackedDecompressor {
  bps: u32,
  endian: Endian,
}

impl PackedDecompressor {
  pub fn new(bps: u32, endian: Endian) -> Self {
    Self { bps, endian }
  }
}

impl<'a> Decompressor<'a, u16> for PackedDecompressor {
  fn decompress(&self, src: &[u8], skip_rows: usize, lines: impl LineIteratorMut<'a, u16>, line_width: usize) -> std::result::Result<(), String> {
    match (self.endian, self.bps) {
      // 16 bits, encoding depends on TIFF endianess
      (Endian::Big, 16) => unpack_16be(lines, src, skip_rows, line_width),
      (Endian::Little, 16) => unpack_16le(lines, src, skip_rows, line_width),
      // 12 Bits, DNG spec says it must be always encoded as big-endian
      (_, 12) => unpack_12be(lines, src, skip_rows, line_width),
      // 10 Bits, DNG spec says it must be always encoded as big-endian
      (_, 10) => unpack_10be(lines, src, skip_rows, line_width),
      // 8 bits
      (_, 8) => unpack_8bit(lines, src, skip_rows, line_width),
      // Generic MSB decoder for exotic packed bit sizes
      (_, bps) if bps > 0 && bps < 16 => unpack_generic_msb(lines, src, skip_rows, line_width, self.bps),
      // Unhandled bits
      (_, bps) => return Err(format_args!("DNG: Don't know how to handle DNG with {} bps", bps).to_string()),
    }
    Ok(())
  }

  fn strips_optimized(&self) -> bool {
    true
  }

  fn tile_optimized(&self) -> bool {
    true
  }
}

#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn unpack_16be<'a>(lines: impl LineIteratorMut<'a, u16>, src: &[u8], skip_rows: usize, width: usize) {
  for (row, line) in lines.enumerate() {
    let inb = &src[((skip_rows + row) * width * 2)..];
    for (out, bytes) in line.iter_mut().zip(inb.chunks_exact(2)) {
      *out = BEu16(bytes, 0);
    }
  }
}

#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn unpack_16le<'a>(lines: impl LineIteratorMut<'a, u16>, src: &[u8], skip_rows: usize, width: usize) {
  for (row, line) in lines.enumerate() {
    let inb = &src[((skip_rows + row) * width * 2)..];
    for (i, bytes) in (0..width).zip(inb.chunks_exact(2)) {
      line[i] = LEu16(bytes, 0);
    }
  }
}

#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn unpack_12be<'a>(lines: impl LineIteratorMut<'a, u16>, src: &[u8], skip_rows: usize, width: usize) {
  for (row, line) in lines.enumerate() {
    let inb = &src[((skip_rows + row) * width * 12 / 8)..];
    for (o, i) in line.chunks_exact_mut(2).zip(inb.chunks_exact(3)) {
      let g1: u16 = i[0] as u16;
      let g2: u16 = i[1] as u16;
      let g3: u16 = i[2] as u16;

      o[0] = (g1 << 4) | (g2 >> 4);
      o[1] = ((g2 & 0x0f) << 8) | g3;
    }
  }
}

#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn unpack_10be<'a>(lines: impl LineIteratorMut<'a, u16>, src: &[u8], skip_rows: usize, width: usize) {
  for (row, line) in lines.enumerate() {
    let inb = &src[((skip_rows + row) * width * 10 / 8)..];

    for (o, i) in line.chunks_exact_mut(4).zip(inb.chunks_exact(5)) {
      let g1: u16 = i[0] as u16;
      let g2: u16 = i[1] as u16;
      let g3: u16 = i[2] as u16;
      let g4: u16 = i[3] as u16;
      let g5: u16 = i[4] as u16;

      o[0] = (g1 << 2) | (g2 >> 6);
      o[1] = ((g2 & 0x3f) << 4) | (g3 >> 4);
      o[2] = ((g3 & 0x0f) << 6) | (g4 >> 2);
      o[3] = ((g4 & 0x03) << 8) | g5;
    }
  }
}

#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn unpack_8bit<'a>(lines: impl LineIteratorMut<'a, u16>, src: &[u8], skip_rows: usize, width: usize) {
  for (row, line) in lines.enumerate() {
    let inb = &src[((skip_rows + row) * width)..];
    for (o, i) in line.iter_mut().zip(inb.iter()) {
      *o = *i as u16;
    }
  }
}

#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn unpack_generic_msb<'a>(lines: impl LineIteratorMut<'a, u16>, src: &[u8], skip_rows: usize, width: usize, bits: u32) {
  assert!(bits <= 16);

  let skip_bits = skip_rows * width * bits as usize;
  let skip_bytes = skip_bits / 8;
  let skip_rem = (skip_bits % 8) as u32;

  let mut pump = BitPumpMSB::new(&src[skip_bytes..]);

  if skip_rem > 0 {
    pump.get_bits(skip_rem);
  }

  for line in lines {
    for p in line {
      *p = pump.get_bits(bits) as u16;
    }
  }
}

impl<'a> Decompressor<'a, f32> for PackedDecompressor {
  fn decompress(&self, src: &[u8], skip_rows: usize, lines: impl LineIteratorMut<'a, f32>, line_width: usize) -> std::result::Result<(), String> {
    match (self.endian, self.bps) {
      // 16 bits, encoding depends on TIFF endianess
      (Endian::Big, 32) => unpack_f32be(lines, src, skip_rows, line_width),
      (Endian::Little, 32) => unpack_f32le(lines, src, skip_rows, line_width),

      (Endian::Big, 24) => unpack_f24be(lines, src, skip_rows, line_width),
      (Endian::Little, 24) => unpack_f24le(lines, src, skip_rows, line_width),

      (Endian::Big, 16) => unpack_f16be(lines, src, skip_rows, line_width),
      (Endian::Little, 16) => unpack_f16le(lines, src, skip_rows, line_width),

      // Unhandled bits
      (_, bps) => return Err(format_args!("DNG: Don't know how to handle FP DNG with {} bps", bps).to_string()),
    }
    Ok(())
  }

  fn strips_optimized(&self) -> bool {
    true
  }

  fn tile_optimized(&self) -> bool {
    true
  }
}

#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn unpack_f32be<'a>(lines: impl LineIteratorMut<'a, f32>, src: &[u8], skip_rows: usize, width: usize) {
  for (row, line) in lines.enumerate() {
    let inb = &src[((skip_rows + row) * width * size_of::<f32>())..];
    for (i, bytes) in (0..width).zip(inb.chunks_exact(4)) {
      line[i] = BEf32(bytes, 0);
    }
  }
}

#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn unpack_f32le<'a>(lines: impl LineIteratorMut<'a, f32>, src: &[u8], skip_rows: usize, width: usize) {
  for (row, line) in lines.enumerate() {
    let inb = &src[((skip_rows + row) * width * size_of::<f32>())..];
    for (i, bytes) in (0..width).zip(inb.chunks_exact(4)) {
      line[i] = LEf32(bytes, 0);
    }
  }
}

#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn unpack_f24le<'a>(lines: impl LineIteratorMut<'a, f32>, src: &[u8], skip_rows: usize, width: usize) {
  const SIZEOF_FP24: usize = 3;
  for (row, line) in lines.enumerate() {
    let inb = &src[((skip_rows + row) * width * SIZEOF_FP24)..];
    for (i, bytes) in (0..width).zip(inb.chunks_exact(SIZEOF_FP24)) {
      line[i] = LEf24(bytes, 0);
    }
  }
}

#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn unpack_f24be<'a>(lines: impl LineIteratorMut<'a, f32>, src: &[u8], skip_rows: usize, width: usize) {
  const SIZEOF_FP24: usize = 3;
  for (row, line) in lines.enumerate() {
    let inb = &src[((skip_rows + row) * width * SIZEOF_FP24)..];
    for (i, bytes) in (0..width).zip(inb.chunks_exact(SIZEOF_FP24)) {
      line[i] = BEf24(bytes, 0);
    }
  }
}

#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn unpack_f16le<'a>(lines: impl LineIteratorMut<'a, f32>, src: &[u8], skip_rows: usize, width: usize) {
  const SIZEOF_FP16: usize = 2;
  for (row, line) in lines.enumerate() {
    let inb = &src[((skip_rows + row) * width * SIZEOF_FP16)..];
    for (i, bytes) in (0..width).zip(inb.chunks_exact(SIZEOF_FP16)) {
      line[i] = LEf16(bytes, 0);
    }
  }
}

#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn unpack_f16be<'a>(lines: impl LineIteratorMut<'a, f32>, src: &[u8], skip_rows: usize, width: usize) {
  const SIZEOF_FP16: usize = 2;
  for (row, line) in lines.enumerate() {
    let inb = &src[((skip_rows + row) * width * SIZEOF_FP16)..];
    for (i, bytes) in (0..width).zip(inb.chunks_exact(SIZEOF_FP16)) {
      line[i] = BEf16(bytes, 0);
    }
  }
}