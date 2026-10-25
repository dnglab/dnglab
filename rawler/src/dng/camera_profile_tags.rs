// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! DNG tags of camera profiles
//!
//! The primary camera profile is stored in IFD 0, extra profiles are stored
//! in camera profile IFDs referenced by the ExtraCameraProfiles tag.

use std::io::{Cursor, Read, Seek, SeekFrom};

use crate::bits::Endian;
use crate::camera_profile::{
  CALIBRATION_SLOTS, CameraCalibration, CameraProfile, GainData, GainTableMap, IlluminantData, ProfileDynamicRange, RgbTable, RgbTables, TableData,
};
use crate::formats::tiff::{DirectoryWriter, IFD, Rational, SRational, TiffWriter, Value};
use crate::imgop::matrix::FlatMatrix;
use crate::imgop::xyz::Illuminant;
use crate::tags::DngTag;

pub(crate) const CALIBRATION_ILLUMINANT_TAGS: [DngTag; CALIBRATION_SLOTS] =
  [DngTag::CalibrationIlluminant1, DngTag::CalibrationIlluminant2, DngTag::CalibrationIlluminant3];
pub(crate) const COLOR_MATRIX_TAGS: [DngTag; CALIBRATION_SLOTS] = [DngTag::ColorMatrix1, DngTag::ColorMatrix2, DngTag::ColorMatrix3];
pub(crate) const FORWARD_MATRIX_TAGS: [DngTag; CALIBRATION_SLOTS] = [DngTag::ForwardMatrix1, DngTag::ForwardMatrix2, DngTag::ForwardMatrix3];
pub(crate) const REDUCTION_MATRIX_TAGS: [DngTag; CALIBRATION_SLOTS] = [DngTag::ReductionMatrix1, DngTag::ReductionMatrix2, DngTag::ReductionMatrix3];
pub(crate) const HUE_SAT_MAP_DATA_TAGS: [DngTag; CALIBRATION_SLOTS] =
  [DngTag::ProfileHueSatMapData1, DngTag::ProfileHueSatMapData2, DngTag::ProfileHueSatMapData3];
pub(crate) const ILLUMINANT_DATA_TAGS: [DngTag; CALIBRATION_SLOTS] = [DngTag::IlluminantData1, DngTag::IlluminantData2, DngTag::IlluminantData3];
pub(crate) const CAMERA_CALIBRATION_TAGS: [DngTag; CALIBRATION_SLOTS] = [DngTag::CameraCalibration1, DngTag::CameraCalibration2, DngTag::CameraCalibration3];

/// Magic number of a camera profile header ("CR")
const CAMERA_PROFILE_MAGIC: u16 = 0x4352;

/// Denominator for matrices and other values written as (S)RATIONAL
const RATIONAL_DENOMINATOR: i32 = 10_000;

/// Byte order of TIFF data written by [`TiffWriter`]
pub(crate) fn native_endian() -> Endian {
  if cfg!(target_endian = "little") { Endian::Little } else { Endian::Big }
}

/// Convert values to SRATIONAL, rounded to [`RATIONAL_DENOMINATOR`]
pub(crate) fn srationals(values: &[f32]) -> Vec<SRational> {
  values
    .iter()
    .map(|v| SRational::new((v * RATIONAL_DENOMINATOR as f32).round() as i32, RATIONAL_DENOMINATOR))
    .collect()
}

/// Convert values to RATIONAL, rounded to [`RATIONAL_DENOMINATOR`]
pub(crate) fn rationals(values: &[f32]) -> Vec<Rational> {
  values
    .iter()
    .map(|v| Rational::new((v * RATIONAL_DENOMINATOR as f32).round().max(0.0) as u32, RATIONAL_DENOMINATOR as u32))
    .collect()
}

/// String value as ASCII, or as UTF-8 encoded BYTE if it isn't plain ASCII
fn string_value(s: &str) -> Value {
  if s.is_ascii() {
    Value::from(s)
  } else {
    let mut bytes = s.as_bytes().to_vec();
    bytes.push(0);
    Value::Byte(bytes)
  }
}

fn read_f32s(ifd: &IFD, tag: DngTag) -> Option<Vec<f32>> {
  let entry = ifd.get_entry(tag)?;
  (0..entry.count() as usize).map(|i| entry.value.get_f32(i).ok().flatten()).collect()
}

fn read_u32(ifd: &IFD, tag: DngTag) -> Option<u32> {
  ifd.get_entry(tag).and_then(|entry| entry.value.get_u32(0).ok().flatten())
}

/// ASCII or BYTE (UTF-8) string, empty strings are `None`
pub(crate) fn read_string(ifd: &IFD, tag: DngTag) -> Option<String> {
  let s = match &ifd.get_entry(tag)?.value {
    Value::Ascii(ascii) => ascii.first().cloned()?,
    Value::Byte(bytes) => {
      let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
      String::from_utf8_lossy(&bytes[..end]).into_owned()
    }
    value => {
      log::warn!("Invalid type {} for {:?}", value.value_type_name(), tag);
      return None;
    }
  };
  Some(s).filter(|s| !s.is_empty())
}

fn read_undefined(ifd: &IFD, tag: DngTag) -> Option<&[u8]> {
  match &ifd.get_entry(tag)?.value {
    Value::Undefined(data) | Value::Byte(data) => Some(data),
    value => {
      log::warn!("Invalid type {} for {:?}", value.value_type_name(), tag);
      None
    }
  }
}

/// Matrix of `tag` if it has `len` values
fn read_matrix(ifd: &IFD, tag: DngTag, len: usize) -> Option<FlatMatrix> {
  let matrix = read_f32s(ifd, tag)?;
  if matrix.len() == len {
    Some(matrix)
  } else {
    log::warn!("Invalid {:?} dimensions: length {}, expected {}", tag, matrix.len(), len);
    None
  }
}

/// Table dimensions (hue, saturation and value divisions)
fn read_dims(ifd: &IFD, tag: DngTag) -> Option<[u32; 3]> {
  let entry = ifd.get_entry(tag)?;
  let dims = [0, 1, 2].map(|i| entry.value.get_u32(i).ok().flatten().unwrap_or(0));
  Some(dims)
}

/// Table data with 3 values for each entry of `dims`
fn read_table_data(ifd: &IFD, tag: DngTag, dims: [u32; 3]) -> Option<Vec<f32>> {
  let data = read_f32s(ifd, tag)?;
  if dims.iter().map(|d| *d as usize).product::<usize>() * 3 == data.len() {
    Some(data)
  } else {
    log::warn!("Invalid {:?} size {} for dimensions {:?}", tag, data.len(), dims);
    None
  }
}

/// Decode a binary tag, invalid data is ignored with a warning
fn read_binary<T>(ifd: &IFD, tag: DngTag, decode: impl Fn(&[u8], Endian) -> std::result::Result<T, String>) -> Option<T> {
  let data = read_undefined(ifd, tag)?;
  decode(data, ifd.endian).map_err(|err| log::warn!("Invalid {:?}: {}", tag, err)).ok()
}

/// Read the camera profile tags of `ifd` (IFD 0 or a camera profile IFD)
///
/// Returns `None` if there is no valid ColorMatrix1.
pub fn read_camera_profile(ifd: &IFD) -> Option<CameraProfile> {
  let color_matrix1 = read_f32s(ifd, DngTag::ColorMatrix1)?;
  if color_matrix1.is_empty() || color_matrix1.len() % 3 != 0 || color_matrix1.len() > 12 {
    log::warn!("Invalid ColorMatrix1 dimensions: length {}", color_matrix1.len());
    return None;
  }
  let planes = color_matrix1.len() / 3;
  let mut profile = CameraProfile {
    name: read_string(ifd, DngTag::ProfileName),
    calibration_signature: read_string(ifd, DngTag::ProfileCalibrationSignature),
    copyright: read_string(ifd, DngTag::ProfileCopyright),
    embed_policy: read_u32(ifd, DngTag::ProfileEmbedPolicy),
    group_name: read_string(ifd, DngTag::ProfileGroupName),
    hue_sat_map_encoding: read_u32(ifd, DngTag::ProfileHueSatMapEncoding),
    look_table_encoding: read_u32(ifd, DngTag::ProfileLookTableEncoding),
    baseline_exposure_offset: read_f32s(ifd, DngTag::BaselineExposureOffset).and_then(|v| v.first().copied()),
    default_black_render: read_u32(ifd, DngTag::DefaultBlackRender),
    dynamic_range: read_binary(ifd, DngTag::ProfileDynamicRange, decode_dynamic_range),
    rgb_tables: read_binary(ifd, DngTag::RGBTables, decode_rgb_tables),
    gain_table_map: read_binary(ifd, DngTag::ProfileGainTableMap2, decode_gain_table_map),
    ..Default::default()
  };
  for slot in 0..CALIBRATION_SLOTS {
    let Some(color_matrix) = read_matrix(ifd, COLOR_MATRIX_TAGS[slot], planes * 3) else {
      continue;
    };
    let illuminant = ifd.get_entry(CALIBRATION_ILLUMINANT_TAGS[slot]).map(|entry| {
      let value = entry.value.force_u16(0);
      Illuminant::try_from(value).unwrap_or_else(|_| {
        log::warn!("Unknown calibration illuminant {} in {:?}", value, CALIBRATION_ILLUMINANT_TAGS[slot]);
        Illuminant::Unknown
      })
    });
    profile.color_matrix[slot] = Some(color_matrix);
    profile.calibration_illuminant[slot] = illuminant;
    if illuminant == Some(Illuminant::Other) {
      profile.illuminant_data[slot] = read_binary(ifd, ILLUMINANT_DATA_TAGS[slot], decode_illuminant_data);
    }
    profile.forward_matrix[slot] = read_matrix(ifd, FORWARD_MATRIX_TAGS[slot], 3 * planes);
    if planes > 3 {
      profile.reduction_matrix[slot] = read_matrix(ifd, REDUCTION_MATRIX_TAGS[slot], 3 * planes);
    }
  }
  if let Some(dims) = read_dims(ifd, DngTag::ProfileHueSatMapDims) {
    let slots: Vec<usize> = profile.slots().collect();
    for slot in slots {
      profile.hue_sat_map_data[slot] = read_table_data(ifd, HUE_SAT_MAP_DATA_TAGS[slot], dims);
    }
    profile.hue_sat_map_dims = Some(dims);
  }
  if let Some(dims) = read_dims(ifd, DngTag::ProfileLookTableDims) {
    profile.look_table_data = read_table_data(ifd, DngTag::ProfileLookTableData, dims);
    profile.look_table_dims = profile.look_table_data.as_ref().map(|_| dims);
  }
  profile.tone_curve = read_f32s(ifd, DngTag::ProfileToneCurve).filter(|curve| {
    let valid = curve.len() >= 4 && curve.len() % 2 == 0;
    if !valid {
      log::warn!("Invalid ProfileToneCurve length {}", curve.len());
    }
    valid
  });
  Some(profile)
}

/// Read the camera profiles referenced by ExtraCameraProfiles of `root`
///
/// Each profile starts with a byte order mark, the magic number 0x4352 and the
/// offset of its IFD. All offsets are relative to the start of the profile.
pub fn read_extra_camera_profiles<R: Read + Seek>(reader: &mut R, root: &IFD) -> Vec<CameraProfile> {
  let Some(entry) = root.get_entry(DngTag::ExtraCameraProfiles) else {
    return Vec::new();
  };
  (0..entry.count() as usize)
    .filter_map(|i| {
      // Offsets are relative to the TIFF header, like all offsets of `root`
      let offset = root.base.checked_add(entry.value.get_u32(i).ok().flatten()?)?;
      read_extra_camera_profile(reader, offset)
        .map_err(|err| log::warn!("Extra camera profile {} at offset {} is invalid: {}", i, offset, err))
        .ok()
    })
    .collect()
}

fn read_extra_camera_profile<R: Read + Seek>(reader: &mut R, offset: u32) -> std::result::Result<CameraProfile, String> {
  let mut header = [0_u8; 8];
  reader
    .seek(SeekFrom::Start(offset as u64))
    .and_then(|_| reader.read_exact(&mut header))
    .map_err(|err| err.to_string())?;
  let endian = match &header[0..2] {
    b"II" => Endian::Little,
    b"MM" => Endian::Big,
    _ => return Err("invalid byte order mark".into()),
  };
  if endian.read_u16(&header, 2) != CAMERA_PROFILE_MAGIC {
    return Err("invalid magic number".into());
  }
  let ifd_offset = endian.read_u32(&header, 4);
  if ifd_offset < 8 {
    return Err(format!("invalid IFD offset {}", ifd_offset));
  }
  let ifd = IFD::new(reader, ifd_offset, offset, 0, endian, &[]).map_err(|err| err.to_string())?;
  read_camera_profile(&ifd).ok_or_else(|| "no valid ColorMatrix1".into())
}

/// Read CameraCalibration1..3 and CameraCalibrationSignature of IFD 0
pub fn read_camera_calibration(ifd: &IFD, planes: usize) -> CameraCalibration {
  CameraCalibration {
    matrices: CAMERA_CALIBRATION_TAGS.map(|tag| read_matrix(ifd, tag, planes * planes)),
    signature: read_string(ifd, DngTag::CameraCalibrationSignature),
  }
}

/// Write the tags of `profile` to `ifd` (IFD 0 or a camera profile IFD)
pub fn write_camera_profile(ifd: &mut DirectoryWriter, profile: &CameraProfile) {
  for slot in profile.slots() {
    if let Some(illuminant) = profile.calibration_illuminant[slot] {
      ifd.add_tag(CALIBRATION_ILLUMINANT_TAGS[slot], u16::from(illuminant));
    }
    if let Some(matrix) = &profile.color_matrix[slot] {
      ifd.add_tag(COLOR_MATRIX_TAGS[slot], srationals(matrix).as_slice());
    }
    if let Some(matrix) = &profile.forward_matrix[slot] {
      ifd.add_tag(FORWARD_MATRIX_TAGS[slot], srationals(matrix).as_slice());
    }
    if let Some(matrix) = &profile.reduction_matrix[slot] {
      ifd.add_tag(REDUCTION_MATRIX_TAGS[slot], srationals(matrix).as_slice());
    }
    if let Some(data) = &profile.illuminant_data[slot] {
      ifd.add_tag_undefined(ILLUMINANT_DATA_TAGS[slot], encode_illuminant_data(data, native_endian()));
    }
    if let (Some(_), Some(data)) = (profile.hue_sat_map_dims, &profile.hue_sat_map_data[slot]) {
      ifd.add_tag(HUE_SAT_MAP_DATA_TAGS[slot], data.as_slice());
    }
  }
  if let Some(dims) = profile.hue_sat_map_dims {
    ifd.add_tag(DngTag::ProfileHueSatMapDims, &dims[..]);
  }
  if let Some(encoding) = profile.hue_sat_map_encoding {
    ifd.add_tag(DngTag::ProfileHueSatMapEncoding, encoding);
  }
  if let (Some(dims), Some(data)) = (profile.look_table_dims, &profile.look_table_data) {
    ifd.add_tag(DngTag::ProfileLookTableDims, &dims[..]);
    ifd.add_tag(DngTag::ProfileLookTableData, data.as_slice());
  }
  if let Some(encoding) = profile.look_table_encoding {
    ifd.add_tag(DngTag::ProfileLookTableEncoding, encoding);
  }
  if let Some(curve) = &profile.tone_curve {
    ifd.add_tag(DngTag::ProfileToneCurve, curve.as_slice());
  }
  for (tag, value) in [
    (DngTag::ProfileName, &profile.name),
    (DngTag::ProfileCalibrationSignature, &profile.calibration_signature),
    (DngTag::ProfileCopyright, &profile.copyright),
    (DngTag::ProfileGroupName, &profile.group_name),
  ] {
    if let Some(value) = value {
      ifd.add_value(tag, string_value(value));
    }
  }
  if let Some(policy) = profile.embed_policy {
    ifd.add_tag(DngTag::ProfileEmbedPolicy, policy);
  }
  if let Some(offset) = profile.baseline_exposure_offset {
    ifd.add_tag(DngTag::BaselineExposureOffset, srationals(&[offset])[0]);
  }
  if let Some(render) = profile.default_black_render {
    ifd.add_tag(DngTag::DefaultBlackRender, render);
  }
  if let Some(range) = &profile.dynamic_range {
    ifd.add_tag_undefined(DngTag::ProfileDynamicRange, encode_dynamic_range(range, native_endian()));
  }
  if let Some(tables) = &profile.rgb_tables {
    ifd.add_tag_undefined(DngTag::RGBTables, encode_rgb_tables(tables, native_endian()));
  }
  if let Some(map) = &profile.gain_table_map {
    ifd.add_tag_undefined(DngTag::ProfileGainTableMap2, encode_gain_table_map(map, native_endian()));
  }
}

/// Camera profile for ExtraCameraProfiles: header and IFD, offsets are relative to the start
pub fn extra_camera_profile_data(profile: &CameraProfile) -> crate::formats::tiff::Result<Vec<u8>> {
  let mut buf = Cursor::new(Vec::new());
  let tiff = TiffWriter::new_with_magic(&mut buf, CAMERA_PROFILE_MAGIC)?;
  let mut ifd = DirectoryWriter::new();
  write_camera_profile(&mut ifd, profile);
  tiff.build(ifd)?;
  Ok(buf.into_inner())
}

/// Write CameraCalibration1..3 and CameraCalibrationSignature
pub fn write_camera_calibration(ifd: &mut DirectoryWriter, calibration: &CameraCalibration) {
  for (tag, matrix) in CAMERA_CALIBRATION_TAGS.iter().zip(&calibration.matrices) {
    if let Some(matrix) = matrix {
      ifd.add_tag(*tag, srationals(matrix).as_slice());
    }
  }
  if let Some(signature) = &calibration.signature {
    ifd.add_value(DngTag::CameraCalibrationSignature, string_value(signature));
  }
}

/// Write the string tag `tag`, ASCII or UTF-8 encoded BYTE
pub(crate) fn write_string(ifd: &mut DirectoryWriter, tag: DngTag, value: &str) {
  ifd.add_value(tag, string_value(value));
}

/// Reader for binary tag data
struct BinaryReader<'a> {
  data: &'a [u8],
  pos: usize,
  endian: Endian,
}

impl<'a> BinaryReader<'a> {
  fn new(data: &'a [u8], endian: Endian) -> Self {
    Self { data, pos: 0, endian }
  }

  fn bytes(&mut self, len: usize) -> std::result::Result<&'a [u8], String> {
    let bytes = self.data.get(self.pos..self.pos.saturating_add(len)).ok_or_else(|| {
      format!(
        "data is truncated, {} bytes needed at offset {}, but only {} available",
        len,
        self.pos,
        self.data.len()
      )
    })?;
    self.pos += len;
    Ok(bytes)
  }

  fn array<const N: usize>(&mut self) -> std::result::Result<[u8; N], String> {
    let mut array = [0; N];
    array.copy_from_slice(self.bytes(N)?);
    if self.endian.big() == cfg!(target_endian = "little") {
      array.reverse();
    }
    Ok(array)
  }

  fn u8(&mut self) -> std::result::Result<u8, String> {
    Ok(self.bytes(1)?[0])
  }

  fn u16(&mut self) -> std::result::Result<u16, String> {
    self.array().map(u16::from_ne_bytes)
  }

  fn u32(&mut self) -> std::result::Result<u32, String> {
    self.array().map(u32::from_ne_bytes)
  }

  fn f32(&mut self) -> std::result::Result<f32, String> {
    self.array().map(f32::from_ne_bytes)
  }

  fn f64(&mut self) -> std::result::Result<f64, String> {
    self.array().map(f64::from_ne_bytes)
  }

  fn rational(&mut self) -> std::result::Result<f32, String> {
    let n = self.u32()?;
    let d = self.u32()?;
    if d == 0 {
      return Err("rational with zero denominator".into());
    }
    Ok(n as f32 / d as f32)
  }

  fn remaining(&self) -> usize {
    self.data.len() - self.pos
  }
}

/// Writer for binary tag data
struct BinaryWriter {
  data: Vec<u8>,
  endian: Endian,
}

impl BinaryWriter {
  fn new(endian: Endian) -> Self {
    Self { data: Vec::new(), endian }
  }

  fn array<const N: usize>(&mut self, mut array: [u8; N]) {
    if self.endian.big() == cfg!(target_endian = "little") {
      array.reverse();
    }
    self.data.extend_from_slice(&array);
  }

  fn u8(&mut self, v: u8) {
    self.data.push(v);
  }

  fn u16(&mut self, v: u16) {
    self.array(v.to_ne_bytes());
  }

  fn u32(&mut self, v: u32) {
    self.array(v.to_ne_bytes());
  }

  fn f32(&mut self, v: f32) {
    self.array(v.to_ne_bytes());
  }

  fn f64(&mut self, v: f64) {
    self.array(v.to_ne_bytes());
  }

  fn rational(&mut self, v: f32) {
    let rational = rationals(&[v])[0];
    self.u32(rational.n);
    self.u32(rational.d);
  }
}

/// Decode IlluminantData
pub fn decode_illuminant_data(data: &[u8], endian: Endian) -> std::result::Result<IlluminantData, String> {
  let mut r = BinaryReader::new(data, endian);
  match r.u16()? {
    0 => Ok(IlluminantData::Xy {
      x: r.rational()?,
      y: r.rational()?,
    }),
    1 => {
      let count = r.u32()? as usize;
      if count < 2 {
        return Err(format!("{} spectral samples, at least 2 required", count));
      }
      let min_lambda = r.rational()?;
      let lambda_spacing = r.rational()?;
      if r.remaining() < count * 8 {
        return Err(format!("{} spectral samples, but only {} bytes", count, r.remaining()));
      }
      let values = (0..count).map(|_| r.rational()).collect::<std::result::Result<_, _>>()?;
      Ok(IlluminantData::Spectral {
        min_lambda,
        lambda_spacing,
        values,
      })
    }
    data_type => Err(format!("unknown data type {}", data_type)),
  }
}

/// Encode IlluminantData
pub fn encode_illuminant_data(value: &IlluminantData, endian: Endian) -> Vec<u8> {
  let mut w = BinaryWriter::new(endian);
  match value {
    IlluminantData::Xy { x, y } => {
      w.u16(0);
      w.rational(*x);
      w.rational(*y);
    }
    IlluminantData::Spectral {
      min_lambda,
      lambda_spacing,
      values,
    } => {
      w.u16(1);
      w.u32(values.len() as u32);
      w.rational(*min_lambda);
      w.rational(*lambda_spacing);
      values.iter().for_each(|v| w.rational(*v));
    }
  }
  w.data
}

/// Decode ProfileDynamicRange
pub fn decode_dynamic_range(data: &[u8], endian: Endian) -> std::result::Result<ProfileDynamicRange, String> {
  let mut r = BinaryReader::new(data, endian);
  let range = ProfileDynamicRange {
    version: r.u16()?,
    dynamic_range: r.u16()?,
    hint_max_output_value: r.f32()?,
  };
  if range.version != 1 {
    return Err(format!("unsupported version {}", range.version));
  }
  Ok(range)
}

/// Encode ProfileDynamicRange
pub fn encode_dynamic_range(value: &ProfileDynamicRange, endian: Endian) -> Vec<u8> {
  let mut w = BinaryWriter::new(endian);
  w.u16(value.version);
  w.u16(value.dynamic_range);
  w.f32(value.hint_max_output_value);
  w.data
}

/// Decode RGBTables
///
/// Table samples are always little-endian, independent of the byte order
/// of the IFD.
pub fn decode_rgb_tables(data: &[u8], endian: Endian) -> std::result::Result<RgbTables, String> {
  let mut r = BinaryReader::new(data, endian);
  let count = r.u32()?;
  if !(1..=20).contains(&count) {
    return Err(format!("invalid table count {}", count));
  }
  let composite_method = r.u32()?;
  let mut tables = Vec::with_capacity(count as usize);
  for _ in 0..count {
    let name_len = r.u16()? as usize;
    let name = r.bytes(name_len)?;
    let semantic_name = (name_len > 0).then(|| String::from_utf8_lossy(name).into_owned());
    let divisions = r.u8()?;
    if !(2..=32).contains(&divisions) {
      return Err(format!("invalid divisions {}", divisions));
    }
    let pixel_type = r.u8()?;
    let gamma_encoding = r.u8()?;
    let color_primaries = r.u8()?;
    let gamut_extension = r.u8()?;
    let values = (divisions as usize).pow(3) * 3;
    let data = match pixel_type {
      0 => TableData::U8(r.bytes(values)?.to_vec()),
      1 => TableData::U16(r.bytes(values * 2)?.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect()),
      2 => TableData::F32(r.bytes(values * 4)?.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect()),
      _ => return Err(format!("invalid pixel type {}", pixel_type)),
    };
    tables.push(RgbTable {
      semantic_name,
      divisions,
      gamma_encoding,
      color_primaries,
      gamut_extension,
      data,
    });
  }
  Ok(RgbTables { composite_method, tables })
}

/// Encode RGBTables, see [`decode_rgb_tables`] for the byte order of the samples
pub fn encode_rgb_tables(value: &RgbTables, endian: Endian) -> Vec<u8> {
  let mut w = BinaryWriter::new(endian);
  w.u32(value.tables.len() as u32);
  w.u32(value.composite_method);
  for table in &value.tables {
    let name = table.semantic_name.as_deref().unwrap_or_default();
    w.u16(name.len() as u16);
    w.data.extend_from_slice(name.as_bytes());
    w.u8(table.divisions);
    w.u8(match table.data {
      TableData::U8(_) => 0,
      TableData::U16(_) => 1,
      TableData::F32(_) => 2,
    });
    w.u8(table.gamma_encoding);
    w.u8(table.color_primaries);
    w.u8(table.gamut_extension);
    match &table.data {
      TableData::U8(v) => w.data.extend_from_slice(v),
      TableData::U16(v) => v.iter().for_each(|v| w.data.extend_from_slice(&v.to_le_bytes())),
      TableData::F32(v) => v.iter().for_each(|v| w.data.extend_from_slice(&v.to_le_bytes())),
    }
  }
  w.data
}

/// Decode ProfileGainTableMap2
pub fn decode_gain_table_map(data: &[u8], endian: Endian) -> std::result::Result<GainTableMap, String> {
  let mut r = BinaryReader::new(data, endian);
  let points_v = r.u32()?;
  let points_h = r.u32()?;
  let spacing = [r.f64()?, r.f64()?];
  let origin = [r.f64()?, r.f64()?];
  let points_n = r.u32()?;
  let input_weights = [r.f32()?, r.f32()?, r.f32()?, r.f32()?, r.f32()?];
  let data_type = r.u32()?;
  let gamma = r.f32()?;
  let gain_min = r.f32()?;
  let gain_max = r.f32()?;
  if points_v < 1 || points_h < 1 || points_n < 1 {
    return Err(format!("invalid map points {}x{}x{}", points_v, points_h, points_n));
  }
  let count = (points_v as usize)
    .checked_mul(points_h as usize)
    .and_then(|n| n.checked_mul(points_n as usize))
    .ok_or("map size overflows")?;
  let bytes_per_value = match data_type {
    0 => 1,
    1 | 2 => 2,
    3 => 4,
    _ => return Err(format!("unsupported data type {}", data_type)),
  };
  if r.remaining() < count * bytes_per_value {
    return Err(format!(
      "{} gain values need {} bytes, but only {} available",
      count,
      count * bytes_per_value,
      r.remaining()
    ));
  }
  let gains = match data_type {
    0 => GainData::U8(r.bytes(count)?.to_vec()),
    1 => GainData::U16((0..count).map(|_| r.u16()).collect::<std::result::Result<_, _>>()?),
    2 => GainData::F16((0..count).map(|_| r.u16()).collect::<std::result::Result<_, _>>()?),
    _ => GainData::F32((0..count).map(|_| r.f32()).collect::<std::result::Result<_, _>>()?),
  };
  Ok(GainTableMap {
    points: [points_v, points_h, points_n],
    spacing,
    origin,
    input_weights,
    gamma,
    gain_min,
    gain_max,
    gains,
  })
}

/// Encode ProfileGainTableMap2
pub fn encode_gain_table_map(value: &GainTableMap, endian: Endian) -> Vec<u8> {
  let mut w = BinaryWriter::new(endian);
  w.u32(value.points[0]);
  w.u32(value.points[1]);
  value.spacing.iter().for_each(|v| w.f64(*v));
  value.origin.iter().for_each(|v| w.f64(*v));
  w.u32(value.points[2]);
  value.input_weights.iter().for_each(|v| w.f32(*v));
  w.u32(match value.gains {
    GainData::U8(_) => 0,
    GainData::U16(_) => 1,
    GainData::F16(_) => 2,
    GainData::F32(_) => 3,
  });
  w.f32(value.gamma);
  w.f32(value.gain_min);
  w.f32(value.gain_max);
  match &value.gains {
    GainData::U8(v) => w.data.extend_from_slice(v),
    GainData::U16(v) | GainData::F16(v) => v.iter().for_each(|v| w.u16(*v)),
    GainData::F32(v) => v.iter().for_each(|v| w.f32(*v)),
  }
  w.data
}

#[cfg(test)]
mod tests {
  use super::*;

  const ENDIANS: [Endian; 2] = [Endian::Little, Endian::Big];

  fn gain_table_map(gains: GainData) -> GainTableMap {
    GainTableMap {
      points: [2, 1, (gains.len() / 2) as u32],
      spacing: [0.5, 1.0],
      origin: [0.0, 0.0],
      input_weights: [0.25, 0.5, 0.25, 0.0, 0.0],
      gamma: 1.0,
      gain_min: 0.5,
      gain_max: 2.0,
      gains,
    }
  }

  fn rgb_tables(data: TableData) -> RgbTables {
    RgbTables {
      composite_method: 1,
      tables: vec![
        RgbTable {
          semantic_name: Some("sky".into()),
          divisions: 2,
          gamma_encoding: 1,
          color_primaries: 4,
          gamut_extension: 1,
          data: data.clone(),
        },
        RgbTable {
          semantic_name: None,
          divisions: 2,
          gamma_encoding: 0,
          color_primaries: 0,
          gamut_extension: 0,
          data,
        },
      ],
    }
  }

  #[test]
  fn illuminant_data_roundtrip() {
    for endian in ENDIANS {
      for data in [
        IlluminantData::Xy { x: 0.3127, y: 0.329 },
        IlluminantData::Spectral {
          min_lambda: 380.0,
          lambda_spacing: 10.0,
          values: vec![0.5, 1.0, 1.5],
        },
      ] {
        assert_eq!(decode_illuminant_data(&encode_illuminant_data(&data, endian), endian), Ok(data));
      }
    }
    assert!(decode_illuminant_data(&[0, 0, 0, 0], Endian::Little).is_err());
  }

  #[test]
  fn illuminant_data_big_endian() {
    // x = 1/2, y = 1/4
    let data = [0, 0, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 4];
    assert_eq!(decode_illuminant_data(&data, Endian::Big), Ok(IlluminantData::Xy { x: 0.5, y: 0.25 }));
  }

  #[test]
  fn dynamic_range_roundtrip() {
    let range = ProfileDynamicRange {
      version: 1,
      dynamic_range: 1,
      hint_max_output_value: 8.0,
    };
    for endian in ENDIANS {
      let data = encode_dynamic_range(&range, endian);
      assert_eq!(data.len(), 8);
      assert_eq!(decode_dynamic_range(&data, endian), Ok(range));
    }
  }

  #[test]
  fn rgb_tables_roundtrip() {
    for endian in ENDIANS {
      for data in [
        TableData::U8((0..24).collect()),
        TableData::U16((0..24).map(|v| v * 1000).collect()),
        TableData::F32((0..24).map(|v| v as f32 / 24.0).collect()),
      ] {
        let tables = rgb_tables(data);
        assert_eq!(decode_rgb_tables(&encode_rgb_tables(&tables, endian), endian), Ok(tables));
      }
    }
  }

  #[test]
  fn rgb_tables_samples_are_little_endian() {
    let tables = rgb_tables(TableData::U16(vec![0x0102; 24]));
    let data = encode_rgb_tables(&tables, Endian::Big);
    // Header in big-endian, first table sample in little-endian
    assert_eq!(data[0..4], [0, 0, 0, 2]);
    assert_eq!(data[18..20], [0x02, 0x01]);
  }

  #[test]
  fn rgb_tables_invalid() {
    let mut data = encode_rgb_tables(&rgb_tables(TableData::U8(vec![0; 24])), Endian::Little);
    data.truncate(data.len() - 1);
    assert!(decode_rgb_tables(&data, Endian::Little).is_err());
    assert!(decode_rgb_tables(&[0, 0, 0, 0, 0, 0, 0, 0], Endian::Little).is_err());
  }

  #[test]
  fn gain_table_map_roundtrip() {
    for endian in ENDIANS {
      for gains in [
        GainData::U8(vec![0, 64, 128, 255]),
        GainData::U16(vec![0, 1000, 30000, 65535]),
        GainData::F16(vec![0x3c00, 0x4000, 0x3800, 0x3c00]),
        GainData::F32(vec![1.0, 2.0, 0.5, 1.0]),
      ] {
        let bytes_per_value = match gains {
          GainData::U8(_) => 1,
          GainData::U16(_) | GainData::F16(_) => 2,
          GainData::F32(_) => 4,
        };
        let map = gain_table_map(gains);
        let data = encode_gain_table_map(&map, endian);
        assert_eq!(data.len(), 80 + map.gains.len() * bytes_per_value);
        assert_eq!(decode_gain_table_map(&data, endian), Ok(map));
      }
    }
  }

  #[test]
  fn gain_table_map_truncated() {
    let mut data = encode_gain_table_map(&gain_table_map(GainData::F32(vec![1.0; 4])), Endian::Little);
    data.pop();
    assert!(decode_gain_table_map(&data, Endian::Little).is_err());
  }

  /// Camera profile in big-endian byte order with ColorMatrix1, CalibrationIlluminant1 and ProfileName
  fn big_endian_camera_profile() -> Vec<u8> {
    let mut data = b"MM".to_vec();
    data.extend_from_slice(&CAMERA_PROFILE_MAGIC.to_be_bytes());
    data.extend_from_slice(&8_u32.to_be_bytes());
    let entries = 3_u16;
    // Values follow the IFD, offsets are relative to the profile start
    let values = 8 + 2 + entries as u32 * 12 + 4;
    data.extend_from_slice(&entries.to_be_bytes());
    let mut entry = |tag: DngTag, typ: u16, count: u32, value: [u8; 4]| {
      data.extend_from_slice(&(tag as u16).to_be_bytes());
      data.extend_from_slice(&typ.to_be_bytes());
      data.extend_from_slice(&count.to_be_bytes());
      data.extend_from_slice(&value);
    };
    entry(DngTag::ColorMatrix1, 10, 9, values.to_be_bytes());
    entry(DngTag::CalibrationIlluminant1, 3, 1, [0, 21, 0, 0]);
    entry(DngTag::ProfileName, 2, 5, (values + 72).to_be_bytes());
    data.extend_from_slice(&0_u32.to_be_bytes());
    for v in [2, 0, 0, 0, 1, 0, 0, 0, -1_i32] {
      data.extend_from_slice(&v.to_be_bytes());
      data.extend_from_slice(&2_i32.to_be_bytes());
    }
    data.extend_from_slice(b"Test\0");
    data
  }

  #[test]
  fn big_endian_extra_camera_profile() -> std::result::Result<(), String> {
    // The profile starts at offset 6 of the file
    let mut file = vec![0xAA; 6];
    file.extend(big_endian_camera_profile());
    let profile = read_extra_camera_profile(&mut Cursor::new(&file), 6)?;
    assert_eq!(profile.name.as_deref(), Some("Test"));
    assert_eq!(profile.calibration_illuminant[0], Some(Illuminant::D65));
    assert_eq!(profile.color_matrix[0], Some(vec![1.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, -0.5]));
    assert!(read_extra_camera_profile(&mut Cursor::new(&file), 0).is_err());
    Ok(())
  }

  #[test]
  fn extra_camera_profile_roundtrip() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut profile = CameraProfile {
      name: Some("Extra".into()),
      ..Default::default()
    };
    profile.calibration_illuminant[0] = Some(Illuminant::D50);
    profile.color_matrix[0] = Some(vec![0.5; 9]);
    let mut file = vec![0; 2];
    file.extend(extra_camera_profile_data(&profile)?);
    assert_eq!(read_extra_camera_profile(&mut Cursor::new(&file), 2)?, profile);
    Ok(())
  }

  #[test]
  fn rationals_are_rounded() {
    assert_eq!(srationals(&[0.6515, -0.0927])[0].n, 6515);
    assert_eq!(srationals(&[0.6515, -0.0927])[1].n, -927);
    assert_eq!(rationals(&[0.29999999])[0].n, 3000);
  }
}
