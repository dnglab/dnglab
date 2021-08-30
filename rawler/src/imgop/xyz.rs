use std::convert::TryFrom;

// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

/// Illuminants for XYZ
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Illuminant {
  A,
  B,
  C,
  D50,
  D55,
  D65,
  D75,
  E,
  F2,
  F7,
  F11,
}

pub type FlatColorMatrix = Vec<f32>;

impl TryFrom<usize> for Illuminant {
  type Error = String;

  fn try_from(_value: usize) -> Result<Self, Self::Error> {
    todo!()
  }
}

impl From<Illuminant> for u16 {
  fn from(value: Illuminant) -> Self {
    match value {
      Illuminant::A => todo!(),
      Illuminant::B => todo!(),
      Illuminant::C => todo!(),
      Illuminant::D50 => todo!(),
      Illuminant::D55 => todo!(),
      Illuminant::D65 => 21,
      Illuminant::D75 => todo!(),
      Illuminant::E => todo!(),
      Illuminant::F2 => todo!(),
      Illuminant::F7 => todo!(),
      Illuminant::F11 => todo!(),
    }
  }
}

impl TryFrom<&String> for Illuminant {
  type Error = String;

  fn try_from(value: &String) -> Result<Self, Self::Error> {
    if value == "A" {
      Ok(Self::A)
    } else if value == "B" {
      Ok(Self::B)
    } else if value == "C" {
      Ok(Self::C)
    } else if value == "D50" {
      Ok(Self::D50)
    } else if value == "D55" {
      Ok(Self::D55)
    } else if value == "D65" {
      Ok(Self::D65)
    } else if value == "D75" {
      Ok(Self::D75)
    } else if value == "E" {
      Ok(Self::E)
    } else if value == "F2" {
      Ok(Self::F2)
    } else if value == "F7" {
      Ok(Self::F7)
    } else if value == "F11" {
      Ok(Self::F11)
    } else {
      Err(String::from(format!("Invalid illuminant identifier: {}", value)))
    }
  }
}

// Constant matrix for converting sRGB to XYZ(D65):
// http://www.brucelindbloom.com/Eqn_RGB_XYZ_Matrix.html
pub const SRGB_TO_XYZ_D65: [[f32; 3]; 3] = [
  [0.4124564, 0.3575761, 0.1804375],
  [0.2126729, 0.7151522, 0.0721750],
  [0.0193339, 0.1191920, 0.9503041],
];
