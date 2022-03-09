// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::convert::TryFrom;

/// Illuminants for XYZ
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Illuminant {
  Unknown = 0,
  Daylight = 1,
  Fluorescent = 2,
  Tungsten = 3,
  Flash = 4,
  FineWeather = 9,
  CloudyWeather = 10,
  Shade = 11,
  DaylightFluorescent = 12,
  DaylightWhiteFluorescent = 13,
  CoolWhiteFluorescent = 14,
  WhiteFluorescent = 15,
  A = 17,
  B = 18,
  C = 19,
  D55 = 20,
  D65 = 21,
  D75 = 22,
  D50 = 23,
  IsoStudioTungsten = 24,
}

pub type FlatColorMatrix = Vec<f32>;

impl TryFrom<u16> for Illuminant {
  type Error = String;

  fn try_from(v: u16) -> Result<Self, Self::Error> {
    Ok(match v {
      0 => Self::Unknown,
      1 => Self::Daylight,
      2 => Self::Fluorescent,
      3 => Self::Tungsten,
      4 => Self::Flash,
      9 => Self::FineWeather,
      10 => Self::CloudyWeather,
      11 => Self::Shade,
      12 => Self::DaylightFluorescent,
      13 => Self::DaylightWhiteFluorescent,
      14 => Self::CoolWhiteFluorescent,
      15 => Self::WhiteFluorescent,
      17 => Self::A,
      18 => Self::B,
      19 => Self::C,
      20 => Self::D55,
      21 => Self::D65,
      22 => Self::D75,
      23 => Self::D50,
      24 => Self::IsoStudioTungsten,
      _ => {
        return Err(format!("Unknown illuminant value: {}", v));
      }
    })
  }
}

impl From<Illuminant> for u16 {
  fn from(value: Illuminant) -> Self {
    value as u16
  }
}

impl Illuminant {
  pub fn new_from_str(s: &str) -> Result<Self, String> {
    match s {
      "Unknown" => Ok(Self::Unknown),
      "Daylight" => Ok(Self::Daylight),
      "Fluorescent" => Ok(Self::Fluorescent),
      "Tungsten" => Ok(Self::Tungsten),
      "Flash" => Ok(Self::Flash),
      "FineWeather" => Ok(Self::FineWeather),
      "CloudyWeather" => Ok(Self::CloudyWeather),
      "Shade" => Ok(Self::Shade),
      "DaylightFluorescent" => Ok(Self::DaylightFluorescent),
      "DaylightWhiteFluorescent" => Ok(Self::DaylightWhiteFluorescent),
      "CoolWhiteFluorescent" => Ok(Self::CoolWhiteFluorescent),
      "WhiteFluorescent" => Ok(Self::WhiteFluorescent),
      "A" => Ok(Self::A),
      "B" => Ok(Self::B),
      "C" => Ok(Self::C),
      "D55" => Ok(Self::D55),
      "D65" => Ok(Self::D65),
      "D75" => Ok(Self::D75),
      "D50" => Ok(Self::D50),
      "IsoStudioTungsten" => Ok(Self::IsoStudioTungsten),
      _ => Err(format!("Unknown illuminant name: '{}'", s)),
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
