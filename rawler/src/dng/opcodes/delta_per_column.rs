// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! DNG opcode DeltaPerColumn (DNG 1.3)

use serde::{Deserialize, Serialize};

use crate::{RawImage, Result};

use super::{
  OpcodeStage,
  helpers::{AreaSpec, LineTable, ParamReader, TableAxis, parse_line_table, write_line_table},
};

/// Per-column delta (DNG 1.3), deltas are in the same units as for DeltaPerRow
#[derive(Debug, Clone, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct DeltaPerColumn {
  pub version: u32,
  pub flags: u32,
  pub area: AreaSpec,
  pub deltas: Vec<f32>,
}

impl DeltaPerColumn {
  pub(super) fn parse(reader: &mut ParamReader, version: u32, flags: u32) -> Result<Self> {
    let area = AreaSpec::parse(reader)?;
    let deltas = parse_line_table(reader, &area, TableAxis::Column, "DeltaPerColumn")?;
    Ok(Self { version, flags, area, deltas })
  }

  pub(super) fn write_params(&self, buf: &mut Vec<u8>) {
    write_line_table(buf, &self.area, &self.deltas);
  }

  pub fn apply(&self, rawimage: &mut RawImage, stage: OpcodeStage) -> Result<()> {
    self.table().apply_delta(rawimage, stage)
  }

  fn table(&self) -> LineTable<'_> {
    LineTable {
      name: "DeltaPerColumn",
      area: &self.area,
      axis: TableAxis::Column,
      values: &self.deltas,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{
    dng::opcodes::testutil::*,
    imgop::{Dim2, Point, Rect},
  };

  #[test]
  fn delta_per_column_float_data() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // OpcodeList2 is relative to the ActiveArea: columns 1 and 3 of plane 1 of 3 planes
    let area = [0, 1, 2, 4, 1, 1, 1, 2];
    let mut image = float_image(6, 3, 3, 0.5);
    image.active_area = Some(Rect::new(Point::new(2, 1), Dim2::new(4, 2)));
    single_line_table(11, area, &[0.25, -2.0])?.apply(&mut image, OpcodeStage::List2)?;
    for (i, value) in floats(&image).iter().enumerate() {
      let (row, col, plane) = (i / 18, (i / 3) % 6, i % 3);
      let expected = match (row, col, plane) {
        (1..=2, 3, 1) => 0.75,
        (1..=2, 5, 1) => -1.0,
        _ => 0.5,
      };
      assert_eq!(*value, expected, "pixel {}", i);
    }
    // Data scaled from 16 bit integers has deltas in 16 bit sample values
    let mut image = float_image(6, 3, 3, 0.5);
    image.active_area = Some(Rect::new(Point::new(2, 1), Dim2::new(4, 2)));
    image.scaled_from_integer = true;
    single_line_table(11, area, &[16383.75, 65535.0])?.apply(&mut image, OpcodeStage::List2)?;
    assert_close(floats(&image)[18 + 3 * 3 + 1], 0.75);
    assert_eq!(floats(&image)[2 * 18 + 5 * 3 + 1], 1.0);
    assert_eq!(floats(&image)[18 + 5 * 3 + 2], 0.5);
    Ok(())
  }
}
