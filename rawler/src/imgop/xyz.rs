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
#[allow(clippy::excessive_precision)]
pub const SRGB_TO_XYZ_D65: [[f32; 3]; 3] = [
  [0.4124564, 0.3575761, 0.1804375],
  [0.2126729, 0.7151522, 0.0721750],
  [0.0193339, 0.1191920, 0.9503041],
];

#[allow(clippy::excessive_precision)]
pub const XYZ_TO_ADOBERGB_D65: [[f32; 3]; 3] = [
  [2.0413690, -0.5649464, -0.3446944],
  [-0.9692660, 1.8760108, 0.0415560],
  [0.0134474, -0.1183897, 1.0154096],
];

#[allow(clippy::excessive_precision)]
pub const XYZ_TO_ADOBERGB_D50: [[f32; 3]; 3] = [
  [1.9624274, -0.6105343, -0.3413404],
  [-0.9787684, 1.9161415, 0.0334540],
  [0.0286869, -0.1406752, 1.3487655],
];

#[allow(clippy::excessive_precision)]
pub const XYZ_TO_SRGB_D50: [[f32; 3]; 3] = [
  [3.1338561, -1.6168667, -0.4906146],
  [-0.9787684, 1.9161415, 0.0334540],
  [0.0719453, -0.2289914, 1.4052427],
];

#[allow(clippy::excessive_precision)]
pub const XYZ_TO_SRGB_D65: [[f32; 3]; 3] = [
  [3.2404542, -1.5371385, -0.4985314],
  [-0.9692660, 1.8760108, 0.0415560],
  [0.0556434, -0.2040259, 1.0572252],
];

#[allow(clippy::excessive_precision)]
pub const XYZ_TO_PROFOTORGB_D50: [[f32; 3]; 3] = [
  [1.3459433, -0.2556075, -0.0511118],
  [-0.5445989, 1.5081673, 0.0205351],
  [0.0000000, 0.0000000, 1.2118128],
];

pub const CIE_1931_TRISTIMULUS_A: [f32; 3] = [1.09850, 1.00000, 0.35585]; // X, Y, Z

pub const CIE_1931_TRISTIMULUS_B: [f32; 3] = [0.99072, 1.00000, 0.85223]; // X, Y, Z

pub const CIE_1931_TRISTIMULUS_C: [f32; 3] = [0.98074, 1.00000, 1.18232]; // X, Y, Z

pub const CIE_1931_TRISTIMULUS_D50: [f32; 3] = [0.96422, 1.00000, 0.82521]; // X, Y, Z

pub const CIE_1931_TRISTIMULUS_D55: [f32; 3] = [0.95682, 1.00000, 0.92149]; // X, Y, Z

pub const CIE_1931_TRISTIMULUS_D65: [f32; 3] = [0.95047, 1.00000, 1.08883]; // X, Y, Z

pub const CIE_1931_TRISTIMULUS_D75: [f32; 3] = [0.94972, 1.00000, 1.22638]; // X, Y, Z

pub const CIE_1931_TRISTIMULUS_E: [f32; 3] = [1.00000, 1.00000, 1.00000]; // X, Y, Z

pub const CIE_1931_TRISTIMULUS_F2: [f32; 3] = [0.99186, 1.00000, 0.67393]; // X, Y, Z

pub const CIE_1931_TRISTIMULUS_F7: [f32; 3] = [0.95041, 1.00000, 1.08747]; // X, Y, Z

pub const CIE_1931_TRISTIMULUS_F11: [f32; 3] = [1.00962, 1.00000, 0.64350]; // X, Y, Z

/// incandescent / tungsten
pub const CIE_1931_WHITE_POINT_A: (f32, f32) = (0.44757, 0.40745);
/// obsolete, direct sunlight at noon
pub const CIE_1931_WHITE_POINT_B: (f32, f32) = (0.34842, 0.35161);
/// obsolete, average / North sky daylight
pub const CIE_1931_WHITE_POINT_C: (f32, f32) = (0.31006, 0.31616);
/// horizon light, ICC profile PCS
pub const CIE_1931_WHITE_POINT_D50: (f32, f32) = (0.34567, 0.35850);
/// mid-morning / mid-afternoon daylight
pub const CIE_1931_WHITE_POINT_D55: (f32, f32) = (0.33242, 0.34743);
/// noon daylight: television, sRGB color space
pub const CIE_1931_WHITE_POINT_D65: (f32, f32) = (0.31271, 0.32902);
/// North sky daylight
pub const CIE_1931_WHITE_POINT_D75: (f32, f32) = (0.29902, 0.31485);
/// high-efficiency blue phosphor monitors, BT.2035
pub const CIE_1931_WHITE_POINT_D93: (f32, f32) = (0.28315, 0.29711);
/// equal energy
pub const CIE_1931_WHITE_POINT_E: (f32, f32) = (0.33333, 0.33333);

#[allow(non_snake_case)]
pub fn xyY_to_XYZ(x: f32, y: f32, Y: f32) -> [f32; 3] {
  if y.is_normal() && y.is_sign_positive() {
    [x * Y / y, Y, (1.0 - x - y) * Y / y]
  } else {
    panic!("xy_to_XYZ(): 'y' argument must be greater than zero");
  }
}

#[allow(non_snake_case)]
pub fn xy_to_XYZ(x: f32, y: f32) -> [f32; 3] {
  const Y: f32 = 1.0;
  xyY_to_XYZ(x, y, Y)
}

/// Convert a given xy whitepoint to white balance coefficents,
/// adapted to
pub fn xy_whitepoint_to_wb_coeff(x: f32, y: f32, colormatrix: &[[f32; 3]; 3]) -> [f32; 3] {
  let mut result = [0.0, 0.0, 0.0];
  if y > 0.0 {
    let as_shot_white = xy_to_XYZ(x, y);
    for i in 0..3 {
      let c = colormatrix[i][0] * as_shot_white[0] + colormatrix[i][1] * as_shot_white[1] + colormatrix[i][2] * as_shot_white[2];
      if c > 0.0 {
        result[i] = 1.0 / c;
      }
    }
  }
  result
}
