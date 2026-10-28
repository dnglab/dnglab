// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Summary of the camera profiles of a raw image for `dnglab analyze`
//!
//! Matrices are stored as rows: one dimension of each camera profile matrix
//! is fixed to 3 (XYZ or reduced planes), so the shape is given by the tag.
//! Large tables are summarized by their dimensions.

use std::{
  fmt::{self, Display, Formatter},
  path::Path,
};

use rawler::{
  RawImage,
  camera_profile::{
    CalibrationSlot, CameraCalibration, CameraProfile, GainData, GainTableMap, IlluminantData, RgbTable, RgbTables, TableData, illuminant_temperature,
  },
  decoders::RawDecodeParams,
  imgop::xyz::Illuminant,
  rawsource::RawSource,
};
use serde::Serialize;

/// Matrix in rows
pub type MatrixRows = Vec<Vec<f32>>;

const EMBED_POLICY: [&str; 4] = ["Allow copying", "Embed if used", "Embed never", "No restrictions"];
const DEFAULT_BLACK_RENDER: [&str; 2] = ["Auto", "None"];
const TABLE_ENCODING: [&str; 2] = ["Linear", "sRGB"];
const COMPOSITE_METHOD: [&str; 2] = ["Weighted sum", "Sequential"];
const RGB_TABLE_GAMMA: [&str; 5] = ["Linear", "sRGB", "1.8", "2.2", "Rec. 2020"];
const RGB_TABLE_PRIMARIES: [&str; 5] = ["sRGB", "Adobe RGB", "ProPhoto", "P3", "Rec. 2020"];
const RGB_TABLE_GAMUT: [&str; 2] = ["Clip", "Extend"];

/// Camera profiles of a raw image with the calibration data of IFD 0
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraProfilesReport {
  pub file_name: String,
  pub as_shot_profile_name: Option<String>,
  /// Index of the profile selected by AsShotProfileName
  pub as_shot_profile_index: Option<usize>,
  pub analog_balance: Option<Vec<f32>>,
  pub camera_calibration: Option<CameraCalibrationSummary>,
  pub profiles: Vec<CameraProfileSummary>,
}

/// CameraCalibration1..3 with its signature
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraCalibrationSummary {
  pub signature: Option<String>,
  pub matrices: Vec<SlotMatrix>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotMatrix {
  /// Number of the tag, slot + 1
  pub tag: usize,
  pub matrix: MatrixRows,
}

/// Summary of a camera profile, see [`CameraProfile`]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraProfileSummary {
  pub name: Option<String>,
  pub calibration_signature: Option<String>,
  pub copyright: Option<String>,
  pub embed_policy: Option<String>,
  pub group_name: Option<String>,
  pub color_planes: Option<usize>,
  pub baseline_exposure_offset: Option<f32>,
  pub default_black_render: Option<String>,
  pub dynamic_range: Option<DynamicRangeSummary>,
  pub slots: Vec<CalibrationSlotSummary>,
  pub hue_sat_map: Option<TableSummary>,
  pub look_table: Option<TableSummary>,
  pub tone_curve_points: Option<usize>,
  pub rgb_tables: Option<RgbTablesSummary>,
  pub gain_table_map: Option<GainTableMapSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DynamicRangeSummary {
  pub hdr: bool,
  pub hint_max_output_value: Option<f32>,
}

/// One calibration illuminant of a camera profile
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalibrationSlotSummary {
  /// Number of the tags, slot + 1
  pub tag: usize,
  pub illuminant: Illuminant,
  pub temperature: Option<f32>,
  pub illuminant_data: Option<IlluminantDataSummary>,
  pub color_matrix: MatrixRows,
  pub forward_matrix: Option<MatrixRows>,
  pub reduction_matrix: Option<MatrixRows>,
  /// ProfileHueSatMapData of the slot is present
  pub hue_sat_map: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum IlluminantDataSummary {
  Xy { x: f32, y: f32 },
  Spectral { min_lambda: f32, lambda_spacing: f32, samples: usize },
}

/// HueSatMap or LookTable
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableSummary {
  /// Hue, saturation and value divisions
  pub dims: [u32; 3],
  pub encoding: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RgbTablesSummary {
  pub composite_method: String,
  pub tables: Vec<RgbTableSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RgbTableSummary {
  /// `None` for the background table
  pub semantic_name: Option<String>,
  pub divisions: u8,
  pub gamma: String,
  pub primaries: String,
  pub gamut: String,
  pub pixel_type: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GainTableMapSummary {
  pub points: [u32; 3],
  pub spacing: [f64; 2],
  pub origin: [f64; 2],
  pub input_weights: [f32; 5],
  pub gamma: f32,
  pub gain_min: f32,
  pub gain_max: f32,
  pub data_type: String,
  pub gain_count: usize,
}

/// Decode a raw file without pixels and summarize its camera profiles
pub fn analyze_camera_profiles(path: &Path) -> crate::Result<CameraProfilesReport> {
  let rawfile = RawSource::new(path)?;
  let decoder = rawler::get_decoder(&rawfile)?;
  let rawimage = decoder.raw_image(&rawfile, &RawDecodeParams::default(), true)?;
  let file_name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
  Ok(CameraProfilesReport::new(file_name, &rawimage))
}

impl CameraProfilesReport {
  pub fn new(file_name: String, rawimage: &RawImage) -> Self {
    Self {
      file_name,
      as_shot_profile_name: rawimage.as_shot_profile_name.clone(),
      as_shot_profile_index: rawimage.as_shot_camera_profile_index(),
      analog_balance: rawimage.analog_balance.clone(),
      camera_calibration: (!rawimage.camera_calibration.is_empty()).then(|| CameraCalibrationSummary::from(&rawimage.camera_calibration)),
      profiles: rawimage.camera_profiles.iter().map(CameraProfileSummary::from).collect(),
    }
  }
}

impl From<&CameraCalibration> for CameraCalibrationSummary {
  fn from(calibration: &CameraCalibration) -> Self {
    Self {
      signature: calibration.signature.clone(),
      matrices: calibration
        .matrices
        .iter()
        .enumerate()
        .filter_map(|(slot, matrix)| {
          // Color planes x color planes
          let matrix = matrix.as_ref()?;
          Some(SlotMatrix {
            tag: slot + 1,
            matrix: matrix_rows(matrix, matrix.len().isqrt()),
          })
        })
        .collect(),
    }
  }
}

impl From<&CameraProfile> for CameraProfileSummary {
  fn from(profile: &CameraProfile) -> Self {
    let has_hue_sat_map = profile.hue_sat_map_data.iter().any(Option::is_some);
    Self {
      name: profile.name.clone(),
      calibration_signature: profile.calibration_signature.clone(),
      copyright: profile.copyright.clone(),
      embed_policy: profile.embed_policy.map(|code| code_name(&EMBED_POLICY, code)),
      group_name: profile.group_name.clone(),
      color_planes: profile.color_planes(),
      baseline_exposure_offset: profile.baseline_exposure_offset,
      default_black_render: profile.default_black_render.map(|code| code_name(&DEFAULT_BLACK_RENDER, code)),
      dynamic_range: profile.dynamic_range.map(|range| DynamicRangeSummary {
        hdr: range.is_hdr(),
        hint_max_output_value: Some(range.hint_max_output_value).filter(|value| *value > 0.0),
      }),
      slots: profile.unpack(None).iter().map(CalibrationSlotSummary::from).collect(),
      hue_sat_map: profile
        .hue_sat_map_dims
        .filter(|_| has_hue_sat_map)
        .map(|dims| TableSummary::new(dims, profile.hue_sat_map_encoding)),
      look_table: profile
        .look_table_dims
        .filter(|_| profile.look_table_data.is_some())
        .map(|dims| TableSummary::new(dims, profile.look_table_encoding)),
      tone_curve_points: profile.tone_curve.as_ref().map(|curve| curve.len() / 2),
      rgb_tables: profile.rgb_tables.as_ref().map(RgbTablesSummary::from),
      gain_table_map: profile.gain_table_map.as_ref().map(GainTableMapSummary::from),
    }
  }
}

impl From<&CalibrationSlot<'_>> for CalibrationSlotSummary {
  fn from(slot: &CalibrationSlot<'_>) -> Self {
    let temperature = illuminant_temperature(slot.illuminant);
    Self {
      tag: slot.slot + 1,
      illuminant: slot.illuminant,
      temperature: (temperature > 0.0).then_some(temperature),
      illuminant_data: slot.illuminant_data.map(IlluminantDataSummary::from),
      // Color planes x 3
      color_matrix: matrix_rows(slot.color_matrix, 3),
      // 3 x color planes
      forward_matrix: slot.forward_matrix.map(|matrix| matrix_rows(matrix, matrix.len() / 3)),
      reduction_matrix: slot.reduction_matrix.map(|matrix| matrix_rows(matrix, matrix.len() / 3)),
      hue_sat_map: slot.hue_sat_map_data.is_some(),
    }
  }
}

impl From<&IlluminantData> for IlluminantDataSummary {
  fn from(data: &IlluminantData) -> Self {
    match data {
      IlluminantData::Xy { x, y } => Self::Xy { x: *x, y: *y },
      IlluminantData::Spectral {
        min_lambda,
        lambda_spacing,
        values,
      } => Self::Spectral {
        min_lambda: *min_lambda,
        lambda_spacing: *lambda_spacing,
        samples: values.len(),
      },
    }
  }
}

impl TableSummary {
  /// A missing encoding is linear
  fn new(dims: [u32; 3], encoding: Option<u32>) -> Self {
    Self {
      dims,
      encoding: code_name(&TABLE_ENCODING, encoding.unwrap_or(0)),
    }
  }
}

impl From<&RgbTables> for RgbTablesSummary {
  fn from(tables: &RgbTables) -> Self {
    Self {
      composite_method: code_name(&COMPOSITE_METHOD, tables.composite_method),
      tables: tables.tables.iter().map(RgbTableSummary::from).collect(),
    }
  }
}

impl From<&RgbTable> for RgbTableSummary {
  fn from(table: &RgbTable) -> Self {
    Self {
      semantic_name: table.semantic_name.clone(),
      divisions: table.divisions,
      gamma: code_name(&RGB_TABLE_GAMMA, table.gamma_encoding.into()),
      primaries: code_name(&RGB_TABLE_PRIMARIES, table.color_primaries.into()),
      gamut: code_name(&RGB_TABLE_GAMUT, table.gamut_extension.into()),
      pixel_type: match table.data {
        TableData::U8(_) => "u8",
        TableData::U16(_) => "u16",
        TableData::F32(_) => "f32",
      }
      .into(),
    }
  }
}

impl From<&GainTableMap> for GainTableMapSummary {
  fn from(map: &GainTableMap) -> Self {
    Self {
      points: map.points,
      spacing: map.spacing,
      origin: map.origin,
      input_weights: map.input_weights,
      gamma: map.gamma,
      gain_min: map.gain_min,
      gain_max: map.gain_max,
      data_type: match map.gains {
        GainData::U8(_) => "u8",
        GainData::U16(_) => "u16",
        GainData::F16(_) => "f16",
        GainData::F32(_) => "f32",
      }
      .into(),
      gain_count: map.gains.len(),
    }
  }
}

impl Display for CameraProfilesReport {
  fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
    let mut fields = vec![Field::new("File", &self.file_name)];
    if let Some(name) = &self.as_shot_profile_name {
      fields.push(Field::new("AsShotProfileName", name));
    }
    if let Some(balance) = &self.analog_balance {
      fields.push(Field::new("AnalogBalance", format_matrix(std::slice::from_ref(balance)).concat()));
    }
    if let Some(signature) = self.camera_calibration.as_ref().and_then(|calibration| calibration.signature.as_ref()) {
      fields.push(Field::new("CameraCalibrationSignature", signature));
    }
    let mut lines = field_lines(&fields);
    for calibration in self.camera_calibration.iter().flat_map(|calibration| &calibration.matrices) {
      lines.extend(matrix_lines(&format!("CameraCalibration{}", calibration.tag), None, &calibration.matrix));
    }
    if self.profiles.is_empty() {
      lines.push("No camera profiles".into());
    }
    for (index, profile) in self.profiles.iter().enumerate() {
      let mut roles = Vec::new();
      if index == 0 {
        roles.push("primary");
      }
      if self.as_shot_profile_index == Some(index) {
        roles.push("as shot");
      }
      let mut header = format!("Camera profile {} of {}", index + 1, self.profiles.len());
      if !roles.is_empty() {
        header.push_str(&format!(" ({})", roles.join(", ")));
      }
      lines.push(String::new());
      lines.push(header);
      lines.extend(indent(profile.lines()));
    }
    write_lines(f, &lines)
  }
}

impl Display for CameraProfileSummary {
  fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
    write_lines(f, &self.lines())
  }
}

impl CameraProfileSummary {
  fn lines(&self) -> Vec<String> {
    let mut fields = Vec::new();
    let strings = [
      ("Name", &self.name),
      ("Calibration signature", &self.calibration_signature),
      ("Copyright", &self.copyright),
      ("Embed policy", &self.embed_policy),
      ("Group name", &self.group_name),
    ];
    for (label, value) in strings {
      if let Some(value) = value {
        fields.push(Field::new(label, value));
      }
    }
    if let Some(planes) = self.color_planes {
      fields.push(Field::new("Color planes", planes.to_string()));
    }
    if let Some(offset) = self.baseline_exposure_offset {
      fields.push(Field::new("Baseline exposure offset", format!("{:+.2} EV", offset)));
    }
    if let Some(render) = &self.default_black_render {
      fields.push(Field::new("Default black render", render));
    }
    if let Some(range) = &self.dynamic_range {
      let mut value = String::from(if range.hdr { "HDR" } else { "SDR" });
      if let Some(max) = range.hint_max_output_value {
        value.push_str(&format!(", max output {:.2}", max));
      }
      fields.push(Field::new("Dynamic range", value));
    }
    let mut lines = field_lines(&fields);

    for slot in &self.slots {
      lines.extend(self.slot_lines(slot));
    }

    let mut fields = Vec::new();
    if let Some(table) = &self.look_table {
      fields.push(Field::new("LookTable", table.to_string()));
    }
    if let Some(points) = self.tone_curve_points {
      fields.push(Field::new("ToneCurve", format!("{} points", points)));
    }
    if let Some(tables) = &self.rgb_tables {
      let count = tables.tables.len();
      fields.push(
        Field::new(
          "RGBTables",
          format!("{}, {} {}", tables.composite_method, count, if count == 1 { "table" } else { "tables" }),
        )
        .details(tables.tables.iter().map(ToString::to_string).collect()),
      );
    }
    if let Some(map) = &self.gain_table_map {
      let [v, h, n] = map.points;
      fields.push(
        Field::new(
          "GainTableMap",
          format!("{} × {} × {} points, {} {} gains", v, h, n, map.gain_count, map.data_type),
        )
        .details(map.detail_lines()),
      );
    }
    lines.extend(field_lines(&fields));
    lines
  }

  fn slot_lines(&self, slot: &CalibrationSlotSummary) -> Vec<String> {
    let mut header = format!("Slot {}: {:?}", slot.tag, slot.illuminant);
    if let Some(temperature) = slot.temperature {
      header.push_str(&format!(" ({:.0} K)", temperature));
    }
    let mut lines = Vec::new();
    match &slot.illuminant_data {
      Some(IlluminantDataSummary::Xy { x, y }) => {
        lines.push(format!("IlluminantData{}: x = {:.4}, y = {:.4}", slot.tag, x, y));
      }
      Some(IlluminantDataSummary::Spectral {
        min_lambda,
        lambda_spacing,
        samples,
      }) => {
        let max_lambda = min_lambda + lambda_spacing * samples.saturating_sub(1) as f32;
        lines.push(format!(
          "IlluminantData{}: {} samples, {} .. {} nm in steps of {} nm",
          slot.tag, samples, min_lambda, max_lambda, lambda_spacing
        ));
      }
      None => {}
    }
    lines.extend(matrix_lines(&format!("ColorMatrix{}", slot.tag), Some("XYZ → camera"), &slot.color_matrix));
    if let Some(matrix) = &slot.forward_matrix {
      lines.extend(matrix_lines(&format!("ForwardMatrix{}", slot.tag), Some("camera → XYZ D50"), matrix));
    }
    if let Some(matrix) = &slot.reduction_matrix {
      lines.extend(matrix_lines(&format!("ReductionMatrix{}", slot.tag), Some("camera → 3 planes"), matrix));
    }
    if let Some(table) = self.hue_sat_map.as_ref().filter(|_| slot.hue_sat_map) {
      lines.push(format!("ProfileHueSatMapData{}: {}", slot.tag, table));
    }
    std::iter::once(header).chain(indent(lines)).collect()
  }
}

impl Display for TableSummary {
  fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
    let [hue, sat, value] = self.dims;
    write!(f, "{} × {} × {}, {}", hue, sat, value, self.encoding)
  }
}

impl Display for RgbTableSummary {
  fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
    write!(
      f,
      "{}: {}³ {}, {} gamma, {} primaries, gamut {}",
      self.semantic_name.as_deref().unwrap_or("Background"),
      self.divisions,
      self.pixel_type,
      self.gamma,
      self.primaries,
      self.gamut.to_lowercase()
    )
  }
}

impl GainTableMapSummary {
  fn detail_lines(&self) -> Vec<String> {
    field_lines(&[
      Field::new("Spacing", format!("{} × {}", self.spacing[0], self.spacing[1])),
      Field::new("Origin", format!("{} × {}", self.origin[0], self.origin[1])),
      Field::new("Input weights", format_matrix(&[self.input_weights.to_vec()]).concat()),
      Field::new("Gamma", self.gamma.to_string()),
      Field::new("Gain", format!("{} .. {}", self.gain_min, self.gain_max)),
    ])
  }
}

/// A `label: value` line, details are indented below it
struct Field<'a> {
  label: &'a str,
  value: String,
  details: Vec<String>,
}

impl<'a> Field<'a> {
  fn new(label: &'a str, value: impl Into<String>) -> Self {
    Self {
      label,
      value: value.into(),
      details: Vec::new(),
    }
  }

  fn details(self, details: Vec<String>) -> Self {
    Self { details, ..self }
  }
}

/// Lines of fields with aligned values
fn field_lines(fields: &[Field<'_>]) -> Vec<String> {
  let width = fields.iter().map(|field| field.label.chars().count() + 1).max().unwrap_or(0);
  fields
    .iter()
    .flat_map(|field| std::iter::once(format!("{:width$} {}", format!("{}:", field.label), field.value, width = width)).chain(indent(field.details.clone())))
    .collect()
}

/// Lines of a matrix with a heading like `ColorMatrix1 (XYZ → camera, 4 × 3)`
fn matrix_lines(name: &str, description: Option<&str>, rows: &[Vec<f32>]) -> Vec<String> {
  let shape = format!("{} × {}", rows.len(), rows.first().map_or(0, Vec::len));
  let heading = match description {
    Some(description) => format!("{} ({}, {})", name, description, shape),
    None => format!("{} ({})", name, shape),
  };
  std::iter::once(heading).chain(indent(format_matrix(rows))).collect()
}

/// Format a matrix with aligned columns in brackets, one line for each row
fn format_matrix(rows: &[Vec<f32>]) -> Vec<String> {
  let cells: Vec<Vec<String>> = rows
    .iter()
    .map(|row| {
      row
        .iter()
        // Avoid -0.0000 for small negative values
        .map(|value| format!("{:.4}", if value.abs() < 0.00005 { 0.0 } else { *value }))
        .collect()
    })
    .collect();
  let width = cells.iter().flatten().map(String::len).max().unwrap_or(0);
  cells
    .iter()
    .enumerate()
    .map(|(index, row)| {
      let (open, close) = match index {
        _ if rows.len() == 1 => ('[', ']'),
        0 => ('⎡', '⎤'),
        _ if index + 1 == rows.len() => ('⎣', '⎦'),
        _ => ('⎢', '⎥'),
      };
      let values: Vec<String> = row.iter().map(|cell| format!("{:>width$}", cell, width = width)).collect();
      format!("{} {} {}", open, values.join("  "), close)
    })
    .collect()
}

/// Name of an enumerated tag value
fn code_name(names: &[&str], code: u32) -> String {
  names.get(code as usize).map_or_else(|| format!("Unknown ({})", code), |name| name.to_string())
}

/// Split a row-major matrix into rows of `cols` values
fn matrix_rows(values: &[f32], cols: usize) -> MatrixRows {
  values.chunks(cols.max(1)).map(<[f32]>::to_vec).collect()
}

fn indent(lines: Vec<String>) -> impl Iterator<Item = String> {
  lines.into_iter().map(|line| format!("  {}", line))
}

fn write_lines(f: &mut Formatter<'_>, lines: &[String]) -> fmt::Result {
  for line in lines {
    writeln!(f, "{}", line)?;
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn identity() -> Vec<f32> {
    vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
  }

  fn profile() -> CameraProfile {
    CameraProfile {
      name: Some("Test".into()),
      embed_policy: Some(7),
      calibration_illuminant: [Some(Illuminant::Other), Some(Illuminant::D65), None],
      illuminant_data: [Some(IlluminantData::Xy { x: 0.3457, y: 0.3585 }), None, None],
      color_matrix: [Some(identity()), Some(vec![0.5, -0.25, 0.0, -0.00001, 1.0, 0.125, 0.0, 0.0, 1.0]), None],
      hue_sat_map_data: [None, Some(vec![0.0; 6 * 3]), None],
      hue_sat_map_dims: Some([6, 1, 1]),
      tone_curve: Some(vec![0.0, 0.0, 1.0, 1.0]),
      ..Default::default()
    }
  }

  #[test]
  fn format_matrix_aligns_columns() {
    assert_eq!(format_matrix(&[vec![1.0, 0.5]]), ["[ 1.0000  0.5000 ]"]);
    assert_eq!(
      format_matrix(&[vec![1.0, -0.5], vec![10.0, 0.0]]),
      ["⎡  1.0000  -0.5000 ⎤", "⎣ 10.0000   0.0000 ⎦"]
    );
    assert_eq!(
      format_matrix(&[vec![1.0], vec![-0.00001], vec![0.0], vec![-1.0]]),
      ["⎡  1.0000 ⎤", "⎢  0.0000 ⎥", "⎢  0.0000 ⎥", "⎣ -1.0000 ⎦"]
    );
  }

  #[test]
  fn matrix_shapes_of_four_color_planes() {
    let profile = CameraProfile {
      calibration_illuminant: [Some(Illuminant::D65), None, None],
      color_matrix: [Some(vec![0.0; 12]), None, None],
      forward_matrix: [Some(vec![0.0; 12]), None, None],
      reduction_matrix: [Some(vec![0.0; 12]), None, None],
      ..Default::default()
    };
    let summary = CameraProfileSummary::from(&profile);
    assert_eq!(summary.color_planes, Some(4));
    let slot = &summary.slots[0];
    assert_eq!(slot.color_matrix, vec![vec![0.0; 3]; 4]);
    assert_eq!(slot.forward_matrix, Some(vec![vec![0.0; 4]; 3]));
    assert_eq!(slot.reduction_matrix, Some(vec![vec![0.0; 4]; 3]));
  }

  #[test]
  fn display_profile() {
    let expected = "\
Name:         Test
Embed policy: Unknown (7)
Color planes: 3
Slot 1: Other
  IlluminantData1: x = 0.3457, y = 0.3585
  ColorMatrix1 (XYZ → camera, 3 × 3)
    ⎡ 1.0000  0.0000  0.0000 ⎤
    ⎢ 0.0000  1.0000  0.0000 ⎥
    ⎣ 0.0000  0.0000  1.0000 ⎦
Slot 2: D65 (6500 K)
  ColorMatrix2 (XYZ → camera, 3 × 3)
    ⎡  0.5000  -0.2500   0.0000 ⎤
    ⎢  0.0000   1.0000   0.1250 ⎥
    ⎣  0.0000   0.0000   1.0000 ⎦
  ProfileHueSatMapData2: 6 × 1 × 1, Linear
ToneCurve: 2 points
";
    assert_eq!(CameraProfileSummary::from(&profile()).to_string(), expected);
  }

  #[test]
  fn display_report() {
    let report = CameraProfilesReport {
      file_name: "test.dng".into(),
      as_shot_profile_name: Some("Extra".into()),
      as_shot_profile_index: Some(1),
      analog_balance: Some(vec![1.0, 1.0, 1.0]),
      camera_calibration: Some(CameraCalibrationSummary::from(&CameraCalibration {
        matrices: [None, Some(identity()), None],
        signature: None,
      })),
      profiles: vec![CameraProfileSummary::from(&CameraProfile::default()); 2],
    };
    let expected = "\
File:              test.dng
AsShotProfileName: Extra
AnalogBalance:     [ 1.0000  1.0000  1.0000 ]
CameraCalibration2 (3 × 3)
  ⎡ 1.0000  0.0000  0.0000 ⎤
  ⎢ 0.0000  1.0000  0.0000 ⎥
  ⎣ 0.0000  0.0000  1.0000 ⎦

Camera profile 1 of 2 (primary)

Camera profile 2 of 2 (as shot)
";
    assert_eq!(report.to_string(), expected);
  }

  #[test]
  fn json_has_matrix_rows_and_no_table_data() {
    let json = serde_json::to_value(CameraProfileSummary::from(&profile())).unwrap();
    assert_eq!(json["embedPolicy"], "Unknown (7)");
    assert_eq!(json["slots"][0]["illuminant"], "Other");
    assert_eq!(json["slots"][0]["illuminantData"]["xy"]["x"], 0.3457_f32 as f64);
    assert_eq!(json["slots"][1]["colorMatrix"][0], serde_json::json!([0.5, -0.25, 0.0]));
    assert_eq!(json["hueSatMap"], serde_json::json!({ "dims": [6, 1, 1], "encoding": "Linear" }));
    assert_eq!(json["toneCurvePoints"], 2);
  }
}
