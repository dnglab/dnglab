// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Digests of the raw image data for the RawImageDigest and NewRawImageDigest tags
//!
//! All digests are MD5 over the samples of the stored image (before the
//! LinearizationTable and the opcode lists), with multi-byte samples in
//! little-endian byte order:
//!
//! - The legacy digest of RawImageDigest covers the whole image in row-scan
//!   order with interleaved samples, 8-bit samples are widened to 16 bits.
//!   Floating point data and images with a transparency mask have no legacy
//!   digest, the tag holds the tiled digest for them.
//! - The tiled digest of NewRawImageDigest is the MD5 over the digests of
//!   tiles of 256x256 pixels in row-major order, each tile with planar
//!   samples. A transparency mask is included by the MD5 over the digests
//!   of the image and of the mask.
//! - Lossy JPEG and JPEG XL data are covered by the MD5 over the digests of
//!   the compressed tiles in stored order and of the JPEGTables, for both tags.

use rayon::prelude::*;

use crate::RawImageData;

/// An MD5 digest
pub type Digest = [u8; 16];

/// Width and height of the tiles of the tiled digest
const TILE_SIZE: usize = 256;

/// Size of the buffer that collects the encoded samples of the legacy digest
const LEGACY_BUFFER_BYTES: usize = 64 * 1024;

/// Representation of the samples in a digest
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelType {
  /// 8-bit integer (the low byte of the value)
  Byte,
  /// 16-bit integer
  Short,
  /// 32-bit integer
  Long,
  /// 32-bit floating point
  Float,
}

impl PixelType {
  /// Pixel type of stored samples with `bits` per sample. Half and 24-bit
  /// floats are covered as 32-bit floats.
  pub fn of_stored(float: bool, bits: u32) -> Self {
    match bits {
      _ if float => Self::Float,
      0..=8 => Self::Byte,
      9..=16 => Self::Short,
      _ => Self::Long,
    }
  }

  /// Pixel type of stored samples for the tiled digest. Integer samples with a
  /// LinearizationTable of up to 256 entries are covered by their low byte.
  pub fn of_stored_tiled(float: bool, bits: u32, linearization_entries: Option<usize>) -> Self {
    match Self::of_stored(float, bits) {
      Self::Short if linearization_entries.is_some_and(|entries| entries <= 256) => Self::Byte,
      pixel_type => pixel_type,
    }
  }
}

/// Samples of an image, row-major with interleaved planes
#[derive(Clone, Copy, Debug)]
pub struct Samples<'a> {
  data: SampleData<'a>,
  width: usize,
  height: usize,
  planes: usize,
}

#[derive(Clone, Copy, Debug)]
enum SampleData<'a> {
  Integer(&'a [u16]),
  /// Floating point values, or integer values above 16 bits
  Float(&'a [f32]),
}

impl<'a> Samples<'a> {
  /// Samples of `data` with `width`*`height` pixels of `planes` samples each.
  /// Returns `None` if the sample count doesn't match.
  pub fn new(data: &'a RawImageData, width: usize, height: usize, planes: usize) -> Option<Self> {
    let (data, len) = match data {
      RawImageData::Integer(samples) => (SampleData::Integer(samples), samples.len()),
      RawImageData::Float(samples) => (SampleData::Float(samples), samples.len()),
    };
    (width.checked_mul(height).and_then(|pixels| pixels.checked_mul(planes)) == Some(len)).then_some(Self { data, width, height, planes })
  }

  /// Append the samples at `indices` encoded as `pixel_type` to `out`
  fn encode(&self, pixel_type: PixelType, indices: impl Iterator<Item = usize>, out: &mut Vec<u8>) {
    match (self.data, pixel_type) {
      (SampleData::Integer(d), PixelType::Byte) => out.extend(indices.map(|i| d[i] as u8)),
      (SampleData::Integer(d), PixelType::Short) => indices.for_each(|i| out.extend_from_slice(&d[i].to_le_bytes())),
      (SampleData::Integer(d), PixelType::Long) => indices.for_each(|i| out.extend_from_slice(&(d[i] as u32).to_le_bytes())),
      (SampleData::Integer(d), PixelType::Float) => indices.for_each(|i| out.extend_from_slice(&(d[i] as f32).to_bits().to_le_bytes())),
      (SampleData::Float(d), PixelType::Byte) => out.extend(indices.map(|i| d[i] as u32 as u8)),
      (SampleData::Float(d), PixelType::Short) => indices.for_each(|i| out.extend_from_slice(&(d[i] as u32 as u16).to_le_bytes())),
      (SampleData::Float(d), PixelType::Long) => indices.for_each(|i| out.extend_from_slice(&(d[i] as u32).to_le_bytes())),
      (SampleData::Float(d), PixelType::Float) => indices.for_each(|i| out.extend_from_slice(&d[i].to_bits().to_le_bytes())),
    }
  }
}

/// Legacy digest of RawImageDigest for integer data without transparency mask
pub fn legacy_digest(samples: &Samples, pixel_type: PixelType) -> Digest {
  // 8-bit data is covered as 16 bits
  let pixel_type = match pixel_type {
    PixelType::Byte => PixelType::Short,
    pixel_type => pixel_type,
  };
  let row_len = samples.width * samples.planes;
  let rows_per_chunk = (LEGACY_BUFFER_BYTES / (row_len * 4).max(1)).max(1);
  let mut context = md5::Context::new();
  let mut buffer = Vec::with_capacity(rows_per_chunk * row_len * 4);
  for first_row in (0..samples.height).step_by(rows_per_chunk) {
    let rows = rows_per_chunk.min(samples.height - first_row);
    buffer.clear();
    samples.encode(pixel_type, first_row * row_len..(first_row + rows) * row_len, &mut buffer);
    context.consume(&buffer);
  }
  context.finalize().0
}

/// Tiled digest of NewRawImageDigest, without transparency mask
pub fn tiled_digest(samples: &Samples, pixel_type: PixelType) -> Digest {
  let tiles_across = samples.width.div_ceil(TILE_SIZE);
  let tiles_down = samples.height.div_ceil(TILE_SIZE);
  let tile_digests: Vec<Digest> = (0..tiles_across * tiles_down)
    .into_par_iter()
    .map(|index| {
      let x = (index % tiles_across) * TILE_SIZE;
      let y = (index / tiles_across) * TILE_SIZE;
      let w = TILE_SIZE.min(samples.width - x);
      let h = TILE_SIZE.min(samples.height - y);
      let mut buffer = Vec::with_capacity(w * h * samples.planes * 4);
      for plane in 0..samples.planes {
        for row in y..y + h {
          let start = (row * samples.width + x) * samples.planes + plane;
          samples.encode(pixel_type, (0..w).map(|col| start + col * samples.planes), &mut buffer);
        }
      }
      md5::compute(&buffer).0
    })
    .collect();
  combine(&tile_digests)
}

/// Digest of the digests in `digests`
pub fn combine(digests: &[Digest]) -> Digest {
  let mut context = md5::Context::new();
  for digest in digests {
    context.consume(digest);
  }
  context.finalize().0
}

/// Digest of lossy JPEG or JPEG XL data from the compressed `tiles` in
/// stored order and the JPEGTables
pub fn lossy_digest<'t>(tiles: impl Iterator<Item = &'t [u8]>, jpeg_tables: Option<&'t [u8]>) -> Digest {
  let digests: Vec<Digest> = tiles.chain(jpeg_tables).map(|data| md5::compute(data).0).collect();
  combine(&digests)
}
