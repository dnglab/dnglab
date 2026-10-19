// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Helpers for the DNG opcode tests

use crate::{
  RawImage, RawImageData, Result,
  decoders::{Camera, RawPhotometricInterpretation},
  rawimage::{BlackLevel, WhiteLevel},
};

use super::{
  OpcodeList,
  helpers::{put_f32, put_f64, put_u32},
};

pub(super) const V1_3: u32 = 0x01030000;

pub(super) fn opcode(id: u32, version: u32, flags: u32, params: &[u8]) -> Vec<u8> {
  let mut buf = Vec::new();
  for value in [id, version, flags, params.len() as u32] {
    put_u32(&mut buf, value);
  }
  buf.extend_from_slice(params);
  buf
}

pub(super) fn opcode_list(opcodes: &[Vec<u8>]) -> Vec<u8> {
  let mut buf = Vec::new();
  put_u32(&mut buf, opcodes.len() as u32);
  opcodes.iter().for_each(|op| buf.extend_from_slice(op));
  buf
}

/// MapPolynomial parameters, area is [top, left, bottom, right, plane, planes, row_pitch, col_pitch]
pub(super) fn map_polynomial(area: [u32; 8], coeffs: &[f64]) -> Vec<u8> {
  let mut buf = Vec::new();
  area.iter().for_each(|v| put_u32(&mut buf, *v));
  put_u32(&mut buf, coeffs.len() as u32 - 1);
  coeffs.iter().for_each(|c| put_f64(&mut buf, *c));
  buf
}

pub(super) fn float_image(width: usize, height: usize, cpp: usize, value: f32) -> RawImage {
  RawImage::new_with_data(
    Camera::default(),
    RawImageData::Float(vec![value; width * height * cpp]),
    width * cpp,
    height,
    cpp,
    [1.0, 1.0, 1.0, f32::NAN],
    RawPhotometricInterpretation::LinearRaw,
    Some(BlackLevel::zero(1, 1, cpp)),
    Some(WhiteLevel::new(vec![1; cpp])),
    false,
  )
}

pub(super) fn floats(rawimage: &RawImage) -> &[f32] {
  match &rawimage.data {
    RawImageData::Float(samples) => samples,
    RawImageData::Integer(_) => panic!("expected float data"),
  }
}

/// GainMap parameters, `points`, `spacing` and `origin` are (V, H), `gains` are [V][H][MapPlanes]
pub(super) fn gain_map(area: [u32; 8], points: (u32, u32), spacing: (f64, f64), origin: (f64, f64), map_planes: u32, gains: &[f32]) -> Vec<u8> {
  let mut buf = Vec::new();
  area.iter().for_each(|v| put_u32(&mut buf, *v));
  put_u32(&mut buf, points.0);
  put_u32(&mut buf, points.1);
  [spacing.0, spacing.1, origin.0, origin.1].iter().for_each(|v| put_f64(&mut buf, *v));
  put_u32(&mut buf, map_planes);
  gains.iter().for_each(|v| put_f32(&mut buf, *v));
  buf
}

/// WarpRectilinear parameters, each set is [kr0, kr1, kr2, kr3, kt0, kt1]
pub(super) fn warp_rectilinear(sets: &[[f64; 6]], center: (f64, f64)) -> Vec<u8> {
  let mut buf = Vec::new();
  put_u32(&mut buf, sets.len() as u32);
  sets.iter().flatten().for_each(|v| put_f64(&mut buf, *v));
  put_f64(&mut buf, center.0);
  put_f64(&mut buf, center.1);
  buf
}

pub(super) fn single_warp(sets: &[[f64; 6]], center: (f64, f64)) -> Result<OpcodeList> {
  OpcodeList::parse(&opcode_list(&[opcode(1, V1_3, 0, &warp_rectilinear(sets, center))]))
}

/// Image with value x * scale for each sample in column x
pub(super) fn ramp_image(width: usize, height: usize, cpp: usize, scale: f32) -> RawImage {
  let mut image = float_image(width, height, cpp, 0.0);
  image.data = RawImageData::Float((0..width * height * cpp).map(|i| ((i / cpp) % width) as f32 * scale).collect());
  image
}

pub(super) const V1_6: u32 = 0x01060000;

pub(super) const FULL_RANGE: [f64; 2] = [0.0, 1.0];

pub(super) const NO_TANGENTIAL: [f64; 2] = [0.0, 0.0];

/// WarpRectilinear2 parameters, each set is (kr0..kr14, [kt0, kt1], [min, max] valid radius)
pub(super) fn warp_rectilinear2(sets: &[([f64; 15], [f64; 2], [f64; 2])], center: (f64, f64), reciprocal: u32) -> Vec<u8> {
  let mut buf = Vec::new();
  put_u32(&mut buf, sets.len() as u32);
  for (radial, tangential, valid_radius) in sets {
    radial.iter().chain(tangential).chain(valid_radius).for_each(|v| put_f64(&mut buf, *v));
  }
  put_f64(&mut buf, center.0);
  put_f64(&mut buf, center.1);
  put_u32(&mut buf, reciprocal);
  buf
}

/// Radial coefficients starting with kr0, the remaining coefficients are zero
pub(super) fn radial(coeffs: &[f64]) -> [f64; 15] {
  let mut radial = [0.0; 15];
  radial[..coeffs.len()].copy_from_slice(coeffs);
  radial
}

pub(super) fn integers(rawimage: &RawImage) -> &[u16] {
  match &rawimage.data {
    RawImageData::Integer(samples) => samples,
    RawImageData::Float(_) => panic!("expected integer data"),
  }
}

pub(super) fn assert_close(value: f32, expected: f64) {
  assert!((value as f64 - expected).abs() < 1e-6, "{} != {}", value, expected);
}

pub(super) fn line_table(area: [u32; 8], values: &[f32]) -> Vec<u8> {
  let mut buf = Vec::new();
  area.iter().for_each(|v| put_u32(&mut buf, *v));
  put_u32(&mut buf, values.len() as u32);
  values.iter().for_each(|v| put_f32(&mut buf, *v));
  buf
}

pub(super) fn single_line_table(id: u32, area: [u32; 8], values: &[f32]) -> Result<OpcodeList> {
  OpcodeList::parse(&opcode_list(&[opcode(id, V1_3, 0, &line_table(area, values))]))
}
