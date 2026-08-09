// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Nikolay Amiantov <ab@fmap.me>

//! Golomb-Rice entropy decoding of the chunks.

use std::io::Cursor;

use bitstream_io::BitRead;

use crate::Result;
use crate::bits::log2ceil;

use super::BitPump;
use super::Plane;
use super::container::Chunk;
use super::geometry::LatticeSpan;
use super::pixelops::dequant;

/// Coefficients are entropy-coded in fixed-size groups (one Rice parameter k
/// per group).
const GROUP_SIZE: usize = 4;

/// Read a unary run of zeros terminated by a one, but once `max` bits have
/// been read stop without consuming a terminator.
#[inline]
fn read_unary_capped(reader: &mut BitPump<'_>, max: u32) -> Result<u32> {
  let mut count = 0;
  while count < max {
    if reader.read_bit()? {
      break;
    }
    count += 1;
  }
  Ok(count)
}

/// Adaptive update of the Rice parameter k, read as a variable-length code:
/// '0' keeps k; '10' + (run of zeros, terminated by 1) raises k by 1+run;
/// '11' + (run of zeros) lowers k by 1+run.
fn update_k(reader: &mut BitPump<'_>, k: u32) -> Result<u32> {
  if !reader.read_bit()? {
    // Prefix 0
    return Ok(k);
  }
  if !reader.read_bit()? {
    // Prefix 10
    return Ok(k + 1 + reader.read_unary::<1>()?);
  }
  // Prefix 11
  // k is a non-negative Rice parameter, so a decrease at k <= 0 is an
  // impossible state.
  if k == 0 {
    return Err("ARW6: decrease of Rice parameter k below 0".into());
  }
  let k = k - 1;
  // The decrease is a unary run of zeros, but once k hits 0 no terminating 1
  // is emitted.
  Ok(k - read_unary_capped(reader, k)?)
}

/// Decode one group of coefficients into out[] and return the next k.
/// Each coefficient is a k-bit absolute value; then k is updated (unless this
/// is the line's last group), then one sign bit per non-zero coefficient is
/// read.
fn decode_group(reader: &mut BitPump<'_>, k: u32, not_last: bool, out: &mut [i32]) -> Result<u32> {
  debug_assert!(k > 0);
  for value in out.iter_mut() {
    *value = reader.read_var::<u32>(k)? as i32;
  }
  let new_k = if not_last { update_k(reader, k)? } else { k };
  for value in out.iter_mut() {
    if *value != 0 && reader.read_bit()? {
      *value = -*value;
    }
  }
  Ok(new_k)
}

/// Decode one coefficient line of four-coefficient groups filling all of
/// `out` (whose length must be a multiple of [`GROUP_SIZE`]). `k` is stored
/// first. While k==0 the line is coded as runs of all-zero groups; otherwise
/// each group is decode_group coded.
fn decode_line(reader: &mut BitPump<'_>, out: &mut [i32]) -> Result<()> {
  debug_assert!(out.len() % GROUP_SIZE == 0);
  let groups = out.len() / GROUP_SIZE;
  out.fill(0);
  let mut k = update_k(reader, 0)?;
  let mut group_i = 0;
  while group_i < groups {
    if k == 0 {
      let remaining = groups - group_i;
      if remaining == 1 {
        // A lone trailing group is implicitly zero; nothing more to read
        break;
      }
      // Elias-gamma-style coding for the number of zero groups. First, read
      // the exponent part.
      let exponent = read_unary_capped(reader, log2ceil(remaining) as u32)?;
      let mut run = 1usize << exponent;
      if run >= remaining {
        // If the run already covers the remaining groups, end the line.
        break;
      }
      run += reader.read_var::<u32>(exponent)? as usize;
      if run > remaining {
        return Err("ARW6: zero-run overruns the line".into());
      }
      group_i += run;
      k = 1 + reader.read_unary::<1>()?; // then k climbs
    } else {
      let not_last = group_i < groups - 1;
      k = decode_group(reader, k, not_last, &mut out[GROUP_SIZE * group_i..GROUP_SIZE * (group_i + 1)])?;
      group_i += 1;
    }
  }
  Ok(())
}

/// Decode one component's data within a region (all its orientations). The
/// component's data (`data`) is a sequence of byte-aligned chunks.
///
/// `orientations` describes the orientations of the component's region.
/// Returns one dequantized 2D plane per orientation.
///
/// For each orientation `span` a chunk `chunk_i` covers the coordinate window
/// [span.row_base + chunk_i*rows_per_chunk, span.row_base +
/// (chunk_i+1)*rows_per_chunk) of each orientation, capped at span.top and
/// span.bottom (so a chunk may contain fewer than rows_per_chunk rows at
/// borders).
pub(super) fn decode_component(data: &[u8], width: usize, rows_per_chunk: usize, orientations: &[LatticeSpan], chunks: &[Chunk]) -> Result<Vec<Plane>> {
  let groups = width.div_ceil(GROUP_SIZE);
  // One dense plane per orientation.
  let mut orient: Vec<Plane> = orientations.iter().map(|span| Plane::new(width, span.bottom - span.top)).collect();
  // Work buffer.
  let mut out = vec![0i32; GROUP_SIZE * groups];
  let mut offset = 0;
  for (chunk_i, chunk) in chunks.iter().enumerate() {
    if chunk.length == 0 {
      continue;
    }
    let window_offset = chunk_i * rows_per_chunk;
    let chunk_data = data
      .get(offset..offset + chunk.length)
      .ok_or_else(|| format!("ARW6: chunk {} runs past the component data", chunk_i))?;
    let mut reader = BitPump::endian(Cursor::new(chunk_data), bitstream_io::BigEndian);
    offset += chunk.length;
    for (orient_i, (span, plane)) in orientations.iter().zip(orient.iter_mut()).enumerate() {
      let q = chunk.q[orient_i];
      let top_row = span.top.max(span.row_base + window_offset);
      let bottom_row = span.bottom.min(span.row_base + window_offset + rows_per_chunk);
      for row_i in top_row..bottom_row {
        decode_line(&mut reader, &mut out)?;
        let dst = plane.row_mut(row_i - span.top);
        for (d, &value) in dst.iter_mut().zip(out.iter()) {
          *d = dequant(value, q);
        }
      }
    }
  }
  Ok(orient)
}
