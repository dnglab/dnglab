// SPDX-License-Identifier: LGPL-2.1
// Copyright 2025 Daniel Vogelbacher <daniel@chaospixel.com>

use crate::imgop::{
  matrix::{multiply, multiply_row1},
  xyz::*,
};

#[allow(clippy::excessive_precision)]
pub const BRADFORD_ADAPTION: [[f32; 3]; 3] = [
  [0.8951000, 0.2664000, -0.1614000],
  [-0.7502000, 1.7135000, 0.0367000],
  [0.0389000, -0.0685000, 1.0296000],
];

#[allow(clippy::excessive_precision)]
pub const BRADFORD_ADAPTION_INVERSE: [[f32; 3]; 3] = [
  [0.9869929, -0.1470543, 0.1599627],
  [0.4323053, 0.5183603, 0.0492912],
  [-0.0085287, 0.0400428, 0.9684867],
];

fn whitepoint_to_lms(whitepoint: &[f32; 3], adaption: &[[f32; 3]; 3]) -> [f32; 3] {
  multiply_row1(adaption, whitepoint)
}

#[allow(non_snake_case)]
fn illuminant_to_XYZ_tristimulus(illuminant: &Illuminant) -> [f32; 3] {
  match illuminant {
    Illuminant::Unknown => todo!(),
    Illuminant::Daylight => {
      // There is no official CIE XYZ tristimulus white point for "Daylight" illuminants.
      // We use D65 as an approximation
      CIE_1931_TRISTIMULUS_D65
    }
    Illuminant::Fluorescent => todo!(),
    Illuminant::Tungsten => todo!(),
    Illuminant::Flash => {
      // There is no official CIE XYZ tristimulus white point for "Flash" illuminants.
      // We use D55 as an approximation assuming flash CCT ≈ 5500 K
      CIE_1931_TRISTIMULUS_D55
    }
    Illuminant::FineWeather => todo!(),
    Illuminant::CloudyWeather => todo!(),
    Illuminant::Shade => todo!(),
    Illuminant::DaylightFluorescent => todo!(),
    Illuminant::DaylightWhiteFluorescent => todo!(),
    Illuminant::CoolWhiteFluorescent => todo!(),
    Illuminant::WhiteFluorescent => todo!(),
    Illuminant::A => CIE_1931_TRISTIMULUS_A,
    Illuminant::B => CIE_1931_TRISTIMULUS_B,
    Illuminant::C => CIE_1931_TRISTIMULUS_C,
    Illuminant::D55 => CIE_1931_TRISTIMULUS_D55,
    Illuminant::D65 => CIE_1931_TRISTIMULUS_D65,
    Illuminant::D75 => CIE_1931_TRISTIMULUS_D75,
    Illuminant::D50 => CIE_1931_TRISTIMULUS_D50,
    Illuminant::IsoStudioTungsten => todo!(),
  }
}

// See http://www.brucelindbloom.com/index.html?Eqn_RGB_XYZ_Matrix.html
pub fn bradford_adaption_matrix(src_illu: &Illuminant, dst_illu: &Illuminant) -> [[f32; 3]; 3] {
  let tristimulus_src = illuminant_to_XYZ_tristimulus(src_illu);
  let tristimulus_dst = illuminant_to_XYZ_tristimulus(dst_illu);

  let lms_src = whitepoint_to_lms(&tristimulus_src, &BRADFORD_ADAPTION);
  let lms_dst = whitepoint_to_lms(&tristimulus_dst, &BRADFORD_ADAPTION);

  let diag = [
    [lms_dst[0] / lms_src[0], 0.0, 0.0], //
    [0.0, lms_dst[1] / lms_src[1], 0.0], //
    [0.0, 0.0, lms_dst[2] / lms_src[2]], //
  ];

  multiply(&multiply(&BRADFORD_ADAPTION_INVERSE, &diag), &BRADFORD_ADAPTION)
}

pub fn adapt_bradford(src_illu: &Illuminant, dst_illu: &Illuminant, src_matrix: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
  let adaption = bradford_adaption_matrix(src_illu, dst_illu);
  multiply(src_matrix, &adaption)
}

#[cfg(test)]
mod tests {
  use approx::assert_relative_eq;

  use crate::imgop::matrix::transform_2d;

  use super::*;

  #[test]
  fn adaption_test() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let m = bradford_adaption_matrix(&Illuminant::A, &Illuminant::B);
    eprintln!("Matrix: {:?}", m);
    let expected = [
      [0.89051634_f32, -0.08291367, 0.2680946],
      [-0.09715235, 1.0754263, 0.08794621],
      [0.053896993, -0.0908558, 2.4838552],
    ];

    assert_relative_eq!(transform_2d(&m).as_slice(), transform_2d(&expected).as_slice(), epsilon = f32::EPSILON);

    Ok(())
  }
}
