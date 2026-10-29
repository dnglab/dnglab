// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! DNG opcode DeltaPerRow (DNG 1.3)

use serde::{Deserialize, Serialize};

use crate::Result;

use super::{
  OpcodeImage, OpcodeStage,
  helpers::{AreaSpec, LineTable, ParamReader, TableAxis, parse_line_table, write_line_table},
};

/// Per-row delta (DNG 1.3). Deltas are in pixel units of the image data,
/// for integer raw data in 16 bit sample values.
#[derive(Debug, Clone, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct DeltaPerRow {
  pub version: u32,
  pub flags: u32,
  pub area: AreaSpec,
  pub deltas: Vec<f32>,
}

impl DeltaPerRow {
  pub(super) fn parse(reader: &mut ParamReader, version: u32, flags: u32) -> Result<Self> {
    let area = AreaSpec::parse(reader)?;
    let deltas = parse_line_table(reader, &area, TableAxis::Row, "DeltaPerRow")?;
    Ok(Self { version, flags, area, deltas })
  }

  pub(super) fn write_params(&self, buf: &mut Vec<u8>) {
    write_line_table(buf, &self.area, &self.deltas);
  }

  pub fn apply(&self, image: &mut OpcodeImage<'_>, stage: OpcodeStage) -> Result<()> {
    self.table().apply_delta(image, stage)
  }

  fn table(&self) -> LineTable<'_> {
    LineTable {
      name: "DeltaPerRow",
      area: &self.area,
      axis: TableAxis::Row,
      values: &self.deltas,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{RawImageData, dng::opcodes::testutil::*, rawimage::WhiteLevel};

  #[test]
  fn delta_per_row_integer_data() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Rows 1 and 3, columns 0 and 2. Deltas are in 16 bit sample values.
    let mut image = float_image(4, 4, 1, 0.0);
    let mut samples: Vec<u16> = (0..16).map(|v| v * 1000).collect();
    samples[6] = 65530;
    samples[12] = 3000;
    image.data = RawImageData::Integer(samples.clone());
    single_line_table(10, [1, 0, 4, 4, 0, 1, 2, 2], &[10.6, -5000.0])?.apply(&mut image, OpcodeStage::List1)?;
    // Rounded and clipped to the 16 bit range
    let mut expected = samples;
    expected[4] = 4011;
    expected[6] = 65535;
    expected[12] = 0;
    expected[14] = 9000;
    assert_eq!(integers(&image), expected);
    // Integer data is not mapped to linear reference values
    assert!(
      single_line_table(10, [0, 0, 4, 4, 0, 1, 1, 1], &[1.0; 4])?
        .apply(&mut image, OpcodeStage::List2)
        .is_err()
    );
    Ok(())
  }

  #[test]
  fn delta_per_row_after_scaling() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut image = float_image(2, 1, 1, 0.0);
    image.data = RawImageData::Integer(vec![1000, 2000]);
    image.whitelevel = WhiteLevel::new(vec![65535]);
    image.apply_scaling()?;
    assert!(image.scaled_from_integer);
    single_line_table(10, [0, 0, 1, 2, 0, 1, 1, 1], &[500.0])?.apply(&mut image, OpcodeStage::List2)?;
    assert_close(floats(&image)[0], 1500.0 / 65535.0);
    assert_close(floats(&image)[1], 2500.0 / 65535.0);
    Ok(())
  }
}
