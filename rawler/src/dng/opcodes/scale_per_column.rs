// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! DNG opcode ScalePerColumn (DNG 1.3)

use serde::{Deserialize, Serialize};

use crate::Result;

use super::{
  OpcodeImage, OpcodeStage,
  helpers::{AreaSpec, LineTable, ParamReader, TableAxis, parse_line_table, write_line_table},
};

/// Per-column scale (DNG 1.3)
#[derive(Debug, Clone, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ScalePerColumn {
  pub version: u32,
  pub flags: u32,
  pub area: AreaSpec,
  pub scales: Vec<f32>,
}

impl ScalePerColumn {
  pub(super) fn parse(reader: &mut ParamReader, version: u32, flags: u32) -> Result<Self> {
    let area = AreaSpec::parse(reader)?;
    let scales = parse_line_table(reader, &area, TableAxis::Column, "ScalePerColumn")?;
    Ok(Self { version, flags, area, scales })
  }

  pub(super) fn write_params(&self, buf: &mut Vec<u8>) {
    write_line_table(buf, &self.area, &self.scales);
  }

  pub fn apply(&self, image: &mut OpcodeImage<'_>, stage: OpcodeStage) -> Result<()> {
    self.table().apply_scale(image, stage)
  }

  fn table(&self) -> LineTable<'_> {
    LineTable {
      name: "ScalePerColumn",
      area: &self.area,
      axis: TableAxis::Column,
      values: &self.scales,
    }
  }
}
