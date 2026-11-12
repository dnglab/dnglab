// SPDX-License-Identifier: MIT
// Copyright 2020 Alfred Gutierrez
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::{BmffError, BoxHeader, FourCC, ReadBox, Result, co64::Co64Box, stsc::StscBox, stsd::StsdBox, stsz::StszBox, stts::SttsBox, vendor::VendorBox};
use log::debug;
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct StblBox {
  pub header: BoxHeader,
  pub stsd: StsdBox,
  pub stts: SttsBox,
  //pub ctts: Option<CttsBox>,
  pub stsc: StscBox,
  pub stsz: StszBox,
  pub co64: Option<Co64Box>,
  pub vendor: Vec<VendorBox>,
}

impl StblBox {
  pub const TYP: FourCC = FourCC::with(['s', 't', 'b', 'l']);

  /// File offset and size of the sample number `sample`, which starts
  /// with 1. None if the sample tables have no valid entries for it.
  pub fn get_sample_offset(&self, sample: u32) -> Option<(usize, usize)> {
    let co64 = self.co64.as_ref()?;
    let desc = self.stsc.get_entry_for_sample(sample)?;
    if desc.samples_per_chunk == 0 {
      return None;
    }
    // Samples before ours in the run of chunks of this entry
    let run_sample = sample.checked_sub(desc.first_sample)?;
    // Chunk number for our sample
    let chunk = desc.first_chunk.checked_add(run_sample / desc.samples_per_chunk)?;
    // Index of sample inside chunk
    let chunk_sample_idx = run_sample % desc.samples_per_chunk;
    // Offset of chunk
    let chunk_offset = *co64.entries.get((chunk as usize).checked_sub(1)?)?;

    // Offset of sample in chunk, a constant sample size needs no loop over
    // up to 2^32 samples
    let sample_offset = if self.stsz.sample_size > 0 {
      chunk_sample_idx as u64 * self.stsz.sample_size as u64
    } else {
      let mut sample_offset = 0_u64;
      for i in 0..chunk_sample_idx {
        sample_offset += self.stsz.sample_size(sample - chunk_sample_idx + i)? as u64;
      }
      sample_offset
    };
    let sample_size = self.stsz.sample_size(sample)?;

    Some((usize::try_from(chunk_offset.checked_add(sample_offset)?).ok()?, sample_size as usize))
  }
}

impl<R: Read + Seek> ReadBox<&mut R> for StblBox {
  fn read_box(mut reader: &mut R, header: BoxHeader) -> Result<Self> {
    let mut stsd = None;
    let mut stts = None;
    // let mut ctts = None;
    //let mut stss = None;
    let mut stsc = None;
    let mut stsz = None;
    //let mut stco = None;
    let mut co64 = None;

    let mut vendors = Vec::new();

    let mut current = reader.stream_position()?;

    while current < header.end_offset() {
      // get box?

      let header = BoxHeader::parse_child(&mut reader, &header)?;

      //let ftyp = Some(FtypBox::read_box(&mut file, header)?);

      match header.typ {
        StsdBox::TYP => {
          stsd = Some(StsdBox::read_box(&mut reader, header)?);
        }
        SttsBox::TYP => {
          stts = Some(SttsBox::read_box(&mut reader, header)?);
        }
        StscBox::TYP => {
          stsc = Some(StscBox::read_box(&mut reader, header)?);
        }
        StszBox::TYP => {
          stsz = Some(StszBox::read_box(&mut reader, header)?);
        }
        Co64Box::TYP => {
          co64 = Some(Co64Box::read_box(&mut reader, header)?);
        }

        _ => {
          debug!("Vendor box found in stbl: {:?}", header.typ);
          let vendor = VendorBox::read_box(&mut reader, header)?;
          vendors.push(vendor);
        }
      }

      current = reader.stream_position()?;
    }

    header.finish(&mut *reader)?;

    Ok(Self {
      header,
      stsd: stsd.ok_or_else(|| BmffError::Parse("stsd box not found, corrupt file?".into()))?,
      stts: stts.ok_or_else(|| BmffError::Parse("stts box not found, corrupt file?".into()))?,
      stsc: stsc.ok_or_else(|| BmffError::Parse("stsc box not found, corrupt file?".into()))?,
      stsz: stsz.ok_or_else(|| BmffError::Parse("stsz box not found, corrupt file?".into()))?,
      co64,
      vendor: vendors,
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::formats::bmff::stsc::StscEntry;

  /// Sample tables with the stsc `entries`, chunks at 1000, 2000 and 3000
  /// and the sample `sizes`
  fn stbl(entries: &[(u32, u32, u32)], sample_size: u32, sizes: &[u32]) -> StblBox {
    let mut stbl = StblBox::default();
    stbl.stsc.entries = entries
      .iter()
      .map(|&(first_chunk, samples_per_chunk, first_sample)| StscEntry {
        first_chunk,
        samples_per_chunk,
        sample_description_index: 1,
        first_sample,
      })
      .collect();
    stbl.stsz.sample_size = sample_size;
    stbl.stsz.sample_sizes = sizes.to_vec();
    stbl.co64 = Some(Co64Box {
      entries: vec![1000, 2000, 3000],
      ..Default::default()
    });
    stbl
  }

  #[test]
  fn sample_offsets() {
    // 2 samples in chunk 1, then 1 sample per chunk
    let stbl = stbl(&[(1, 2, 1), (2, 1, 3)], 0, &[10, 20, 30, 40]);
    assert_eq!(stbl.get_sample_offset(1), Some((1000, 10)));
    assert_eq!(stbl.get_sample_offset(2), Some((1010, 20)));
    assert_eq!(stbl.get_sample_offset(3), Some((2000, 30)));
    assert_eq!(stbl.get_sample_offset(4), Some((3000, 40)));
    // Sample 0, beyond the sizes and beyond the chunks
    assert_eq!(stbl.get_sample_offset(0), None);
    assert_eq!(stbl.get_sample_offset(5), None);
    // Chunk 0, no samples per chunk, no stsc entries
    assert_eq!(self::stbl(&[(0, 1, 1)], 0, &[10]).get_sample_offset(1), None);
    assert_eq!(self::stbl(&[(1, 0, 1)], 0, &[10]).get_sample_offset(1), None);
    assert_eq!(self::stbl(&[], 0, &[10]).get_sample_offset(1), None);
    // A constant sample size with u32::MAX samples per chunk
    assert_eq!(
      self::stbl(&[(1, u32::MAX, 1)], 8, &[8]).get_sample_offset(u32::MAX),
      Some((1000 + (u32::MAX as usize - 1) * 8, 8))
    );
  }
}
