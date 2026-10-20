// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Convert profile schema version 1
//!
//! This schema is frozen. Changes require a new schema version.

use serde::{Deserialize, Serialize};

use crate::dng::{ApplyDngOpcodes, CropMode, DngCompression, DngVersion, opcodes::OpcodeSelector};

/// How to handle files that contain multiple raw images
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MultiFrameMode {
  /// Convert only the first frame
  #[default]
  First,
  /// Convert every frame into a separate DNG file
  Split,
}

/// Convert profile, schema version 1
///
/// Only `title` is mandatory, all other fields fall back to their defaults.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConvertProfileV1 {
  /// Short human readable name
  pub title: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub description: Option<String>,
  /// Highest DNGBackwardVersion the output is allowed to declare
  #[serde(default = "default_max_dng_backward_version")]
  pub max_dng_backward_version: DngVersion,
  /// DNG opcode lists to apply to the image data, in addition to the lists
  /// that can't be stored with `max_dng_backward_version`
  #[serde(default)]
  pub apply_dng_opcodes: ApplyDngOpcodes,
  /// DNG opcodes to remove, as `[<list>:]<opcode>`, e.g. "WarpRectilinear" or "2:GainMap"
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub remove_dng_opcodes: Vec<OpcodeSelector>,
  #[serde(default = "default_compression")]
  pub compression: DngCompression,
  #[serde(default = "default_ljpeg92_predictor")]
  pub ljpeg92_predictor: u8,
  #[serde(default = "default_crop")]
  pub crop: CropMode,
  #[serde(default = "default_true")]
  pub add_preview: bool,
  #[serde(default = "default_true")]
  pub add_thumbnail: bool,
  /// Develop the preview from raw data instead of using an embedded preview
  #[serde(default)]
  pub force_preview: bool,
  /// Develop the thumbnail from raw data instead of using an embedded preview
  #[serde(default)]
  pub force_thumbnail: bool,
  #[serde(default = "default_true")]
  pub embed_raw: bool,
  #[serde(default)]
  pub multiframe: MultiFrameMode,
  #[serde(default)]
  pub keep_mtime: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub artist: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub copyright: Option<String>,
}

fn default_max_dng_backward_version() -> DngVersion {
  DngVersion::V1_4
}

fn default_compression() -> DngCompression {
  DngCompression::Lossless
}

fn default_ljpeg92_predictor() -> u8 {
  1
}

fn default_crop() -> CropMode {
  CropMode::Best
}

fn default_true() -> bool {
  true
}

/// Default values for all fields, `title` is empty and must be set
impl Default for ConvertProfileV1 {
  fn default() -> Self {
    Self {
      title: String::new(),
      description: None,
      max_dng_backward_version: default_max_dng_backward_version(),
      apply_dng_opcodes: ApplyDngOpcodes::Auto,
      remove_dng_opcodes: Vec::new(),
      compression: default_compression(),
      ljpeg92_predictor: default_ljpeg92_predictor(),
      crop: default_crop(),
      add_preview: true,
      add_thumbnail: true,
      force_preview: false,
      force_thumbnail: false,
      embed_raw: true,
      multiframe: MultiFrameMode::First,
      keep_mtime: false,
      artist: None,
      copyright: None,
    }
  }
}
