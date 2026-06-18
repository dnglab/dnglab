// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::{BoxHeader, ReadBox, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct VendorBox {
  pub header: BoxHeader,
}

impl<R: Read + Seek> ReadBox<&mut R> for VendorBox {
  fn read_box(reader: &mut R, header: BoxHeader) -> Result<Self> {
    // Use the saturating `end_offset()` instead of `offset + size`, which
    // overflows for a corrupt near-u64::MAX box size. Valid boxes are unchanged;
    // a corrupt size seeks past EOF and the box loop terminates.
    reader.seek(SeekFrom::Start(header.end_offset()))?;

    Ok(Self { header })
  }
}
