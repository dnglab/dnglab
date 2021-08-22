// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

/// Illuminants for XYZ
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

// Constant matrix for converting sRGB to XYZ(D65):
// http://www.brucelindbloom.com/Eqn_RGB_XYZ_Matrix.html
pub const SRGB_TO_XYZ_D65: [[f32; 3]; 3] = [
  [0.4124564, 0.3575761, 0.1804375],
  [0.2126729, 0.7151522, 0.0721750],
  [0.0193339, 0.1191920, 0.9503041],
];
