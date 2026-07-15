use approx::assert_abs_diff_eq;
use rawler::formats::tiff::Rational;
use rawler::imgop::Point;
use rawler::imgop::raw::correct_blacklevel_linear;
use rawler::rawimage::{BlackLevel, WhiteLevel};

#[test]
fn uses_row_column_sample_repeat_order() {
  let blacklevel = BlackLevel::new(&[10_u32, 20, 30, 11, 21, 31, 12, 22, 32, 13, 23, 33], 2, 2, 3);
  let whitelevel = WhiteLevel::new(vec![113, 223, 333]);
  let mut raw = vec![60.0, 120.0, 180.0, 61.0, 121.0, 181.0, 62.0, 122.0, 182.0, 63.0, 123.0, 183.0];

  correct_blacklevel_linear(&mut raw, 2, 2, 3, &blacklevel, &whitelevel, Point::zero()).unwrap();

  for value in raw {
    assert_abs_diff_eq!(value, 0.5, epsilon = 1e-6);
  }
}

#[test]
fn repeat_is_relative_to_active_area_origin() {
  let blacklevel = BlackLevel::new(&[10_u32, 20, 30, 40], 2, 2, 1);
  let whitelevel = WhiteLevel::new(vec![140]);
  let mut raw = vec![90.0, 80.0, 70.0, 60.0];

  correct_blacklevel_linear(&mut raw, 2, 2, 1, &blacklevel, &whitelevel, Point::new(1, 1)).unwrap();

  for value in raw {
    assert_abs_diff_eq!(value, 0.5, epsilon = 1e-6);
  }
}

#[test]
fn broadcasts_single_component_levels() {
  let blacklevel = BlackLevel::new(&[10_u32], 1, 1, 1);
  let whitelevel = WhiteLevel::new(vec![110]);
  let mut raw = vec![60.0, 60.0, 60.0];

  correct_blacklevel_linear(&mut raw, 1, 1, 3, &blacklevel, &whitelevel, Point::zero()).unwrap();

  assert_eq!(raw, vec![0.5, 0.5, 0.5]);
}

#[test]
fn rejects_inconsistent_metadata_without_mutating_pixels() {
  let blacklevel = BlackLevel {
    levels: vec![Rational::from(0_u32)],
    width: 2,
    height: 2,
    cpp: 3,
  };
  let whitelevel = WhiteLevel::new(vec![100, 100, 100]);
  let mut raw = vec![50.0; 12];
  let original = raw.clone();

  let error = correct_blacklevel_linear(&mut raw, 2, 2, 3, &blacklevel, &whitelevel, Point::zero()).unwrap_err();

  assert!(error.to_string().contains("Black level data length mismatch"));
  assert_eq!(raw, original);
}

#[test]
fn rejects_non_positive_normalization_range() {
  let blacklevel = BlackLevel::new(&[100_u32], 1, 1, 1);
  let whitelevel = WhiteLevel::new(vec![100]);
  let mut raw = vec![100.0];

  let error = correct_blacklevel_linear(&mut raw, 1, 1, 1, &blacklevel, &whitelevel, Point::zero()).unwrap_err();

  assert!(error.to_string().contains("Invalid black/white level range"));
  assert_eq!(raw, vec![100.0]);
}
