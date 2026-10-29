// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! DNG opcode MapTable (DNG 1.3)

use serde::{Deserialize, Serialize};

use crate::{RawlerError, Result};

use super::{
  OpcodeData, OpcodeImage, OpcodeStage,
  helpers::{AreaSpec, ParamReader, map_area, put_u32},
};

/// Maps an area of the image through a 16 bit lookup table
#[derive(Debug, Clone, Eq, PartialEq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MapTable {
  pub version: u32,
  pub flags: u32,
  pub area: AreaSpec,
  /// Table entries, values beyond the table are mapped to the last entry
  pub table: Vec<u16>,
}

/// Maximum number of MapTable entries
const MAP_TABLE_MAX_ENTRIES: usize = 0x10000;

impl MapTable {
  pub(super) fn parse(reader: &mut ParamReader, version: u32, flags: u32) -> Result<Self> {
    let area = AreaSpec::parse(reader)?;
    let count = reader.u32()? as usize;
    if count == 0 || count > MAP_TABLE_MAX_ENTRIES {
      return Err(RawlerError::DecoderFailed(format!(
        "DNG opcode MapTable: table size {} must be 1 to {}",
        count, MAP_TABLE_MAX_ENTRIES
      )));
    }
    reader.check_count(count, 2, "MapTable")?;
    let mut table = Vec::with_capacity(count);
    for _ in 0..count {
      table.push(reader.u16()?);
    }
    Ok(Self { version, flags, area, table })
  }

  pub(super) fn write_params(&self, buf: &mut Vec<u8>) {
    self.area.write(buf);
    put_u32(buf, self.table.len() as u32);
    for entry in &self.table {
      buf.extend_from_slice(&entry.to_be_bytes());
    }
  }

  /// Table entry, the last entry is replicated for indices beyond the table
  fn lookup(&self, index: usize) -> u16 {
    self.table[index.min(self.table.len() - 1)]
  }

  pub fn apply(&self, image: &mut OpcodeImage<'_>, stage: OpcodeStage) -> Result<()> {
    if self.table.is_empty() || self.table.len() > MAP_TABLE_MAX_ENTRIES {
      return Err(RawlerError::DecoderFailed(format!(
        "DNG opcode MapTable: invalid table size {}",
        self.table.len()
      )));
    }
    let Some(rect) = self.area.buffer_rect(image, stage) else {
      return Ok(());
    };
    let (width, cpp) = (image.width(), image.channels());
    match (&mut image.data, stage) {
      // Floating point data is mapped through the 16 bit table
      (OpcodeData::Float(view), _) => {
        let samples = view.as_mut_slice();
        map_area(samples, width, cpp, rect, &self.area, &|x: f32| {
          self.lookup((x.clamp(0.0, 1.0) * 65535.0).round() as usize) as f32 / 65535.0
        });
      }
      (OpcodeData::Integer(view), OpcodeStage::List1) => {
        let samples = view.as_mut_slice();
        map_area(samples, width, cpp, rect, &self.area, &|x: u16| self.lookup(x as usize));
      }
      (OpcodeData::Integer(_), _) => {
        return Err(RawlerError::DecoderFailed(format!(
          "DNG opcode MapTable in {} requires image data mapped to linear reference values",
          stage
        )));
      }
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{
    RawImageData,
    dng::opcodes::{Opcode, OpcodeList, testutil::*},
    imgop::{Dim2, Point, Rect},
  };

  /// MapTable parameters, area is [top, left, bottom, right, plane, planes, row_pitch, col_pitch]
  fn map_table(area: [u32; 8], table: &[u16]) -> Vec<u8> {
    let mut buf = Vec::new();
    area.iter().for_each(|v| put_u32(&mut buf, *v));
    put_u32(&mut buf, table.len() as u32);
    table.iter().for_each(|v| buf.extend_from_slice(&v.to_be_bytes()));
    buf
  }

  fn single_map_table(area: [u32; 8], table: &[u16]) -> Result<OpcodeList> {
    OpcodeList::parse(&opcode_list(&[opcode(7, V1_3, 0, &map_table(area, table))]))
  }

  #[test]
  fn map_table_parse() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let data = opcode_list(&[opcode(7, V1_3, 0, &map_table([0, 0, 4, 4, 0, 1, 1, 1], &[100, 200, 300]))]);
    let list = OpcodeList::parse(&data)?;
    assert!(matches!(&list.opcodes()[0], Opcode::MapTable(op) if op.table == [100, 200, 300]));
    assert_eq!(list.to_bytes()?, data);
    // Empty table
    let params = map_table([0, 0, 4, 4, 0, 1, 1, 1], &[]);
    assert!(OpcodeList::parse(&opcode_list(&[opcode(7, V1_3, 0, &params)])).is_err());
    let list = OpcodeList::parse(&opcode_list(&[opcode(7, V1_3, 1, &params)]))?;
    assert!(matches!(&list.opcodes()[0], Opcode::Unknown(op) if op.id == 7));
    // Size must match the table size, invalid area
    let mut params = map_table([0, 0, 4, 4, 0, 1, 1, 1], &[1, 2]);
    params.push(0);
    assert!(OpcodeList::parse(&opcode_list(&[opcode(7, V1_3, 0, &params)])).is_err());
    assert!(single_map_table([0, 0, 4, 4, 0, 1, 0, 1], &[1, 2]).is_err());
    Ok(())
  }

  #[test]
  fn map_table_integer_data() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Values beyond the table are mapped to the last entry
    let mut image = float_image(4, 4, 1, 0.0);
    image.data = RawImageData::Integer((0..16).collect());
    single_map_table([0, 0, 4, 4, 0, 1, 1, 1], &[100, 200, 300])?.apply(&mut image, OpcodeStage::List1)?;
    let expected: Vec<u16> = (0..16).map(|v| [100, 200, 300][v.min(2)]).collect();
    assert_eq!(integers(&image), expected);
    // Row and column pitch
    let mut image = float_image(4, 4, 1, 0.0);
    image.data = RawImageData::Integer((0..16).collect());
    single_map_table([0, 0, 4, 4, 0, 1, 2, 2], &[1000])?.apply(&mut image, OpcodeStage::List1)?;
    for (i, value) in integers(&image).iter().enumerate() {
      let expected = if (i / 4) % 2 == 0 && (i % 4) % 2 == 0 { 1000 } else { i as u16 };
      assert_eq!(*value, expected, "pixel {}", i);
    }
    // Integer data is not mapped to linear reference values
    assert!(single_map_table([0, 0, 4, 4, 0, 1, 1, 1], &[1])?.apply(&mut image, OpcodeStage::List2).is_err());
    Ok(())
  }

  #[test]
  fn map_table_float_data() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Values are mapped through the 16 bit table index, plane range 1 of 3 planes
    let mut image = float_image(4, 1, 3, 0.0);
    image.data = RawImageData::Float(vec![0.0, 0.0, 0.0, 0.4 / 65535.0, 0.4 / 65535.0, 0.0, 0.5, 0.5, 0.0, 1.2, 1.2, 0.0]);
    single_map_table([0, 0, 1, 4, 1, 1, 1, 1], &[0, 65535])?.apply(&mut image, OpcodeStage::List3)?;
    assert_eq!(floats(&image), [0.0, 0.0, 0.0, 0.4 / 65535.0, 0.0, 0.0, 0.5, 1.0, 0.0, 1.2, 1.0, 0.0]);
    // OpcodeList2 is relative to the ActiveArea
    let mut image = float_image(6, 4, 1, 0.25);
    image.active_area = Some(Rect::new(Point::new(2, 1), Dim2::new(4, 3)));
    single_map_table([0, 0, 1, 1, 0, 1, 1, 1], &[0, 65535])?.apply(&mut image, OpcodeStage::List2)?;
    assert_eq!(floats(&image)[6 + 2], 1.0);
    assert_eq!(floats(&image).iter().filter(|v| **v == 0.25).count(), 23);
    Ok(())
  }
}
