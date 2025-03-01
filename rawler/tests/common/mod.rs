// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

//#[cfg(feature = "samplecheck")]
use md5::Digest;
use rawler::dng::convert::ConvertParams;
use rawler::dng::convert::convert_raw_file;
use rawler::{
  analyze::{AnalyzerResult, analyze_metadata, extract_raw_pixels},
  decoders::RawDecodeParams,
};
use std::{
  convert::TryInto,
  path::{Path, PathBuf},
};

macro_rules! camera_file_check {
  ($make:expr, $model:expr, $test:ident, $file:expr) => {
    #[allow(non_snake_case)]
    #[test]
    fn $test() -> std::result::Result<(), Box<dyn std::error::Error>> {
      //crate::init_test_logger();
      crate::common::check_camera_raw_file_conversion($make, $model, $file)
    }
  };
}

pub(crate) use camera_file_check;

pub(crate) fn rawdb_path() -> PathBuf {
  PathBuf::from(std::env::var("RAWLER_RAWDB").expect("RAWLER_RAWDB variable must be set in order to run RAW test!"))
}

pub(crate) fn rawdb_file(path: impl AsRef<Path>) -> PathBuf {
  let rawdb = rawdb_path();
  rawdb.join(path)
}

pub(crate) fn check_md5_equal(digest: [u8; 16], expected: &str) {
  assert_eq!(hex::encode(digest), expected);
}

/// Generic function to check camera raw files against
/// pre-generated stats and pixel files.
pub(crate) fn check_camera_raw_file_conversion(make: &str, model: &str, sample: &str) -> std::result::Result<(), Box<dyn std::error::Error>> {
  let rawdb = PathBuf::from(std::env::var("RAWLER_RAWDB").expect("RAWLER_RAWDB variable must be set in order to run RAW test!"));

  let mut camera_rawdb = rawdb.clone();
  camera_rawdb.push("cameras");

  let mut testfiles = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
  testfiles.push("data/testdata");

  let base_path = testfiles.join("cameras").join(make).join(model);

  let raw_file = camera_rawdb.join(make).join(model).join(sample);
  let filename = raw_file.file_name().map(|name| name.to_os_string()).expect("Filename must by OS string compatible");
  let mut orig_analyze_file = filename.clone();
  let mut orig_digest_file = filename.clone();
  orig_analyze_file.push(".analyze.yaml");
  orig_digest_file.push(".digest.txt");
  let stats_file = base_path.join(sample).with_file_name(orig_analyze_file);
  let digest_file = base_path.join(sample).with_file_name(orig_digest_file);

  //let pixel_file = base_path.join(&sample).with_extension("pixel");

  //eprintln!("{:?}", stats_file);

  assert!(raw_file.exists(), "Raw file {:?} not found", raw_file);
  assert!(stats_file.exists(), "Stats file {:?} not found", stats_file);

  // Validate stats file
  let new_stats = analyze_metadata(PathBuf::from(&raw_file)).unwrap();
  let old_stats = std::fs::read_to_string(&stats_file)?;

  let old_stats: AnalyzerResult = serde_yaml::from_str(&old_stats)?;

  assert_eq!(old_stats, new_stats);

  // Validate pixel data
  let old_digest_str = std::fs::read_to_string(digest_file)?;
  let old_digest = Digest(TryInto::<[u8; 16]>::try_into(hex::decode(old_digest_str.trim()).expect("Malformed MD5 digest")).expect("Must be [u8; 16]"));
  let (_, _, _cpp, buf) = extract_raw_pixels(&raw_file, &RawDecodeParams::default()).unwrap();
  let v: Vec<u8> = buf.iter().flat_map(|p| p.to_le_bytes()).collect();
  let new_digest = md5::compute(v);
  assert_eq!(old_digest, new_digest, "Old and new raw pixel digest not match!");

  // Convert to DNG with default params
  let params = ConvertParams {
    embedded: false,
    apply_scaling: false,
    ..Default::default()
  };
  let mut dng = std::io::Cursor::new(Vec::new());
  convert_raw_file(&raw_file, &mut dng, &params)?;

  Ok(())
}
