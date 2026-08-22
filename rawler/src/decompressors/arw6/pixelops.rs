// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Nikolay Amiantov <ab@fmap.me>

//! Misc pixel operations.

use super::{CODEC_BIT_DEPTH, Plane};

// The decoder works in a compressed domain; we need to map each compressed
// code back to a linear sensor value through an inverse curve. The curve is a
// fixed codec constant, shipped next to this module as delin_curve.csv (one
// value per 12-bit code); it was extracted by running samples through an
// official decoder. It is roughly logarithmic with a linear toe, but the
// exact construction is unknown, so we look values up in the table itself.
const DELIN_CURVE: [u16; 1 << CODEC_BIT_DEPTH] = parse_delin_curve(include_str!("delin_curve.csv"));

/// Parse the one-value-per-line delin_curve.csv at compile time.
const fn parse_delin_curve(csv: &str) -> [u16; 1 << CODEC_BIT_DEPTH] {
  let bytes = csv.as_bytes();
  let mut table = [0u16; 1 << CODEC_BIT_DEPTH];
  let mut entry = 0;
  let mut value = 0u32;
  let mut have_digit = false;
  let mut i = 0;
  while i < bytes.len() {
    match bytes[i] {
      b @ b'0'..=b'9' => {
        value = value * 10 + (b - b'0') as u32;
        assert!(value <= u16::MAX as u32, "delin_curve.csv: value does not fit u16");
        have_digit = true;
      }
      b'\n' => {
        assert!(have_digit, "delin_curve.csv: empty line");
        assert!(entry < table.len(), "delin_curve.csv: too many entries");
        table[entry] = value as u16;
        entry += 1;
        value = 0;
        have_digit = false;
      }
      _ => panic!("delin_curve.csv: unexpected character"),
    }
    i += 1;
  }
  assert!(!have_digit, "delin_curve.csv: missing final newline");
  assert!(entry == table.len(), "delin_curve.csv: too few entries");
  table
}

#[inline(always)]
pub(super) fn delinearize(code: u16) -> u16 {
  DELIN_CURVE[code as usize]
}

/// Mid-rise dequantizer. q == 0 is identity. Zero values are identity too.
#[inline(always)]
pub(super) fn dequant(value: i32, q: u8) -> i32 {
  if value == 0 || q == 0 {
    return value;
  }
  // (mag + 0.5) * 2^q => (2*mag+1) * 2^(q-1) - 0.5
  // The trailing `-0.5` is approximated as `-(mag & 1)`: rounded down on odd
  // magnitudes and up on even ones.
  let mag = value.unsigned_abs() as i64;
  let recon = (((mag << 1) + 1) << (q - 1)) - (mag & 1);
  (value.signum() as i64 * recon) as i32
}

/// Inverse horizontal DPCM in place over each row.
pub(super) fn predict_horizontal(plane: &mut Plane) {
  for r in 0..plane.h {
    let row = plane.row_mut(r);
    let mut acc = row[0];
    for value in row.iter_mut().skip(1) {
      acc += *value;
      *value = acc;
    }
  }
}
