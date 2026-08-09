// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Nikolay Amiantov <ab@fmap.me>

//! Byte structure of the ARW6 strip: tile records, the tile sub-header,
//! region records and per-component chunk tables.

use std::io::{Cursor, SeekFrom};

use bitstream_io::BitRead;

use crate::Result;
use crate::bits::LEu32;

use super::BitPump;

/// Coding regions per tile: LL, 3 detail regions, green_hi.
pub(super) const REGIONS: usize = 5;

/// Orientations of a detail region (HL/LH/HH); LL and green_hi carry one.
pub(super) const MAX_ORIENTATIONS: usize = 3;

/// The sizes in the format are measured in "blocks".
const BLOCK_BYTES: usize = 16;
/// Each on-disk size field is a 24-bit big-endian count of blocks.
const SIZE_FIELD_BITS: u32 = 24;
const SIZE_SHIFT: usize = BLOCK_BYTES.ilog2() as usize;

const STRIP_HEADER_BYTES: usize = 8; // tile count, then 4 unknown bytes
const TILE_RECORD_BYTES: usize = 24; // offset, x, y, w, h
const TILE_SUBHEADER_BYTES: usize = BLOCK_BYTES; // leading block
const REGION_TOTALS_BYTES: usize = 2 * BLOCK_BYTES; // region-totals area

/// A component count above 9 is invalid; a region record never spans more
/// than two blocks.
const COMPONENT_COUNT_MAX: usize = 9;

// Expected values of the checked tile sub-header fields.
const DECODED_BITS: u32 = 16; // decoded-sample precision
const N_CHANNELS: u32 = 3; // colour components stored
const MODE: u32 = 3; // "mode" is libraw's name for the field; its purpose is unknown

/// One chunk-table entry.
pub(super) struct Chunk {
  /// Byte length (0 = empty chunk).
  pub length: usize,
  /// Per-orientation quantizers (3 for detail, 1 for LL/green_hi).
  pub q: [u8; MAX_ORIENTATIONS],
}

/// One component's data within one coding region. For example, chroma_b's D2.
pub(super) struct Component<'a> {
  /// The component's chunks, top to bottom.
  pub chunks: Vec<Chunk>,
  /// Orientation count declared by the component header.
  pub orient_count: usize,
  /// Encoded data.
  pub data: &'a [u8],
}

pub(super) struct Region<'a> {
  /// The region's per-component data.
  pub components: Vec<Component<'a>>,
}

/// One tile record from the strip header: the tile's blob and its mosaic
/// placement, parsed further by [`parse_tile`].
pub(super) struct TileRecord<'a> {
  pub blob: &'a [u8],
  pub x: usize,
  pub y: usize,
  pub w: usize,
  pub h: usize,
}

/// Reposition the reader to byte offset `byte_off` from the start of its
/// buffer.
fn seek_byte(reader: &mut BitPump<'_>, byte_off: usize) -> Result<()> {
  reader.seek_bits(SeekFrom::Start(8 * byte_off as u64))?;
  Ok(())
}

/// Advance the reader to the next block boundary (no-op if already on one).
fn align_block(reader: &mut BitPump<'_>) -> Result<()> {
  const STRIDE: u64 = 8 * BLOCK_BYTES as u64;
  let pos = reader.position_in_bits()?.div_ceil(STRIDE) * STRIDE;
  reader.seek_bits(SeekFrom::Start(pos))?;
  Ok(())
}

/// The reader's current position as a byte offset from the start of its
/// buffer. The reader must be byte-aligned (true right after `seek_byte`,
/// `align_block`, or whole-byte reads).
fn byte_pos(reader: &mut BitPump<'_>) -> Result<usize> {
  if !reader.byte_aligned() {
    return Err("ARW6: reader not byte-aligned".into());
  }
  Ok((reader.position_in_bits()? / 8) as usize)
}

/// Split the compressed strip into its per-tile records.
pub(super) fn parse_records(strip: &[u8]) -> Result<Vec<TileRecord<'_>>> {
  if strip.len() < STRIP_HEADER_BYTES {
    return Err("ARW6: strip too short for its header".into());
  }
  let count = LEu32(strip, 0) as usize;
  let records_end = STRIP_HEADER_BYTES + count * TILE_RECORD_BYTES;
  if count == 0 || strip.len() < records_end {
    return Err(format!("ARW6: strip of {} bytes cannot hold {} tile records", strip.len(), count).into());
  }
  // Each record is [u64 offset][u32 x, y, w, h], all little-endian.
  let record = |i: usize| &strip[STRIP_HEADER_BYTES + i * TILE_RECORD_BYTES..];
  let offset = |i: usize| u64::from_le_bytes(record(i)[0..8].try_into().expect("record length checked above")) as usize;
  (0..count)
    .map(|i| {
      let rec = record(i);
      // each tile ends where the next begins
      let start = offset(i);
      let end = if i + 1 < count { offset(i + 1) } else { strip.len() };
      let blob = strip
        .get(start..end)
        .ok_or_else(|| format!("ARW6: tile {} spans bytes {}..{} of a {}-byte strip", i, start, end, strip.len()))?;
      Ok(TileRecord {
        blob,
        x: LEu32(rec, 8) as usize,
        y: LEu32(rec, 12) as usize,
        w: LEu32(rec, 16) as usize,
        h: LEu32(rec, 20) as usize,
      })
    })
    .collect()
}

/// Read `n` packed sizes forward from `reader`'s current position.
fn read_sizes(reader: &mut BitPump<'_>, n: usize) -> Result<Vec<usize>> {
  (0..n).map(|_| Ok((reader.read_var::<u32>(SIZE_FIELD_BITS)? as usize) << SIZE_SHIFT)).collect()
}

/// Blocks a region record spans: one holding fewer than 5 sizes, two from 5 up.
///
/// Note that exactly 5 sizes would still fit one block, yet the record is a
/// two-block one -- so this cannot be written as "round the sizes up to a
/// block".
fn region_record_size(count: usize) -> Result<usize> {
  if count > COMPONENT_COUNT_MAX {
    return Err(format!("ARW6: region record declares {} components, at most {} fit", count, COMPONENT_COUNT_MAX).into());
  }
  Ok(if count < 5 { 1 } else { 2 })
}

/// Parse one tile record's blob into its coding regions.
pub(super) fn parse_tile<'a>(rec: &TileRecord<'a>) -> Result<Vec<Region<'a>>> {
  let TileRecord { blob, w, h, .. } = *rec;
  let mut reader = BitPump::endian(Cursor::new(blob), bitstream_io::BigEndian);
  // The leading sub-header block (leaves the reader at the totals).
  // The magic is not a constant and is not meant to be checked: "0000" on the
  // a7R V / a7R VI, "A000" on the a7 V are the values observed so far.
  let _magic = reader.read_var::<u32>(32)?;
  let _unknown0 = reader.read_var::<u32>(32)?; // unknown (sequential in tiles)
  let width = reader.read_var::<u32>(16)? as usize; // tile width in mosaic pixels (== record w)
  let comp_height = reader.read_var::<u32>(16)? as usize; // tile height / 2, before the channel split
  let _unknown1 = reader.read_var::<u32>(6)?; // unknown (15 in every sample)
  let decoded_bits = reader.read_var::<u32>(6)?; // decoded-sample precision (16 in every sample)
  let _unknown2 = reader.read_var::<u32>(4)?; // unknown (0 in every sample)
  let n_channels = reader.read_var::<u32>(3)?; // colour components stored (3)
  let _unknown3 = reader.read_var::<u32>(1)?; // unknown (0 in every sample)
  let mode = reader.read_var::<u32>(2)?; // "mode", purpose unknown (3 in every sample)
  let _unknown4 = reader.read_var::<u32>(10)?; // unknown (512 in every sample)
  if width != w || comp_height != h / 2 || n_channels != N_CHANNELS || decoded_bits != DECODED_BITS || mode != MODE {
    return Err(
      format!(
        "ARW6: tile sub-header mismatch: width={} (record w={}) comp_height={} (record h/2={}) n_channels={} mode={} decoded_bits={}",
        width,
        w,
        comp_height,
        h / 2,
        n_channels,
        mode,
        decoded_bits
      )
      .into(),
    );
  }
  let totals = read_sizes(&mut reader, REGIONS)?; // per-region byte-size totals
  seek_byte(&mut reader, TILE_SUBHEADER_BYTES + REGION_TOTALS_BYTES)?; // first per-region record block
  let mut region_sizes: Vec<Vec<usize>> = Vec::with_capacity(REGIONS);
  for _ in 0..REGIONS {
    let record_start = byte_pos(&mut reader)?;
    let _unknown = reader.read_var::<u32>(4)?; // unknown (0 in every sample)
    let component_count = reader.read_var::<u32>(4)? as usize;
    let blocks = region_record_size(component_count)?;
    region_sizes.push(read_sizes(&mut reader, component_count)?);
    seek_byte(&mut reader, record_start + blocks * BLOCK_BYTES)?; // advance to the next record
  }
  // The component data starts past the record blocks.
  let bitstream = blob.get(byte_pos(&mut reader)?..).ok_or("ARW6: tile ends inside its region records")?;

  let mut regions: Vec<Region<'_>> = Vec::with_capacity(REGIONS);
  let mut region_base = 0;
  for (region_i, (total, component_sizes)) in totals.iter().zip(region_sizes.iter()).enumerate() {
    let mut off = region_base;
    let mut components: Vec<Component<'_>> = Vec::with_capacity(component_sizes.len());
    for &size in component_sizes {
      let buf = bitstream
        .get(off..off + size)
        .ok_or_else(|| format!("ARW6: region {} component data runs past the tile", region_i))?;
      components.push(parse_component(buf)?);
      off += size;
    }
    regions.push(Region { components });
    region_base += total; // a region spans its total, which may pad past its components
  }
  Ok(regions)
}

// The component header is 1 header block + `table_blocks` chunk-table blocks.
// The tile is cut into chunks across all regions; each chunk holds this
// component's coefficients for one 8-row cut. Each of the chunk_count entries
// is bit-packed `[16b length][orient_count * 4b q]`: chunk i's byte length
// then its per-orientation quantizers, which decode_component applies to that
// chunk's coefficients.
const CHUNK_LEN_BITS: u32 = 16; // per-chunk-entry byte-length field width
const Q_BITS: u32 = 4; // per-orientation quantizer field width

/// Parse the component header + chunk table at the start of `buf`.
fn parse_component(buf: &[u8]) -> Result<Component<'_>> {
  let mut reader = BitPump::endian(Cursor::new(buf), bitstream_io::BigEndian);
  let table_blocks = reader.read_var::<u32>(16)? as usize; // chunk-table size in blocks
  let chunk_blocks = reader.read_var::<u32>(24)? as usize; // chunk data size in blocks
  let _unknown0 = reader.read_var::<u32>(8)?; // unknown (0x40 in every sample)
  let orient_count = reader.read_var::<u32>(2)? as usize; // orientation count
  let _unknown1 = reader.read_var::<u32>(6)?; // unknown (0 in every sample)
  let chunk_count = reader.read_var::<u32>(16)? as usize; // chunk count
  let _unknown2 = reader.read_var::<u32>(8)?; // unknown (0x10 in every sample)
  align_block(&mut reader)?;
  let mut chunks: Vec<Chunk> = Vec::with_capacity(chunk_count);
  for _ in 0..chunk_count {
    let length = reader.read_var::<u32>(CHUNK_LEN_BITS)? as usize;
    let mut q = [0u8; MAX_ORIENTATIONS];
    for entry in q.iter_mut().take(orient_count) {
      *entry = reader.read_var::<u32>(Q_BITS)? as u8;
    }
    chunks.push(Chunk { length, q });
  }
  align_block(&mut reader)?;

  // cross-check the header's declared block counts against what we parsed
  let bitstream_offset = byte_pos(&mut reader)?;
  if bitstream_offset != (1 + table_blocks) * BLOCK_BYTES {
    return Err(
      format!(
        "ARW6: parsed chunk table ends at byte {}, but the component header declares {} table blocks",
        bitstream_offset, table_blocks
      )
      .into(),
    );
  }
  let chunk_bytes: usize = chunks.iter().map(|chunk| chunk.length).sum();
  if chunk_blocks != chunk_bytes.div_ceil(BLOCK_BYTES) {
    return Err(
      format!(
        "ARW6: chunks total {} bytes ({} blocks), but the component header declares chunk_blocks = {}",
        chunk_bytes,
        chunk_bytes.div_ceil(BLOCK_BYTES),
        chunk_blocks
      )
      .into(),
    );
  }
  Ok(Component {
    chunks,
    orient_count,
    data: buf.get(bitstream_offset..).ok_or("ARW6: component ends inside its chunk table")?,
  })
}
