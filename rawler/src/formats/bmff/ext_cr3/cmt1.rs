// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use crate::{formats::bmff::BmffError, formats::tiff::GenericTiffReader};

use super::super::{BoxHeader, FourCC, ReadBox, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cmt1Box {
  pub header: BoxHeader,
  pub data: Vec<u8>,
  pub tiff: GenericTiffReader,
}

impl Cmt1Box {
  pub const TYP: FourCC = FourCC::with(['C', 'M', 'T', '1']);
}

impl<R: Read + Seek> ReadBox<&mut R> for Cmt1Box {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {
    let current = reader.stream_position()?;
    // `end_offset() - current` underflows for a corrupt box whose declared end
    // is before the current position, and (with a near-u64::MAX size) yields a
    // huge `data_len` that would over-allocate. Compute the length with
    // `checked_sub` and clamp it to the bytes actually remaining in the stream:
    // a valid CMT box's payload is fully present, so the clamp is a no-op and the
    // data read is unchanged; a corrupt box becomes a decode error / bounded read
    // instead of a panic or OOM.
    let stream_end = reader.seek(SeekFrom::End(0))?;
    reader.seek(SeekFrom::Start(current))?;
    let data_len = header
      .end_offset()
      .checked_sub(current)
      .ok_or_else(|| BmffError::Parse("CMT1: box end before start, corrupt file?".into()))?
      .min(stream_end.saturating_sub(current));
    let mut data = vec![0; data_len as usize];
    reader.read_exact(&mut data)?;

    reader.seek(SeekFrom::Start(header.end_offset()))?;

    let tiff = GenericTiffReader::new_with_buffer(&data, 0, 0, None).map_err(|e| BmffError::Parse(e.to_string()))?;

    Ok(Self { header, data, tiff })
  }
}
