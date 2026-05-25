// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

//#[cfg(feature = "rawdb")]
use md5::Digest;
use rawler::analyze::extract_full_pixels;
use rawler::analyze::extract_preview_pixels;
use rawler::analyze::extract_thumbnail_pixels;
#[cfg(feature = "rawdb")]
use rawler::devtools::rawdb::get_rawdb_cache;
use rawler::devtools::rawdb::rawdb_ensure_file;
use rawler::dng::convert::ConvertParams;
use rawler::dng::convert::convert_raw_file;
use rawler::{
  analyze::{AnalyzerResult, analyze_metadata, extract_raw_pixels},
  decoders::RawDecodeParams,
};
use std::{
  convert::TryInto,
  io::{Seek, SeekFrom, Write},
  path::{Path, PathBuf},
};
use zerocopy::IntoBytes;

/// A `Write + Seek` implementation that discards all data
/// while correctly tracking the stream position.
pub(crate) struct SeekableSink {
  pos: u64,
}

impl SeekableSink {
  pub(crate) fn new() -> Self {
    Self { pos: 0 }
  }
}

impl Write for SeekableSink {
  fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
    self.pos += buf.len() as u64;
    Ok(buf.len())
  }

  fn flush(&mut self) -> std::io::Result<()> {
    Ok(())
  }
}

impl Seek for SeekableSink {
  fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
    match pos {
      SeekFrom::Start(p) => self.pos = p,
      SeekFrom::End(_) => self.pos = u64::MAX,
      SeekFrom::Current(off) => {
        self.pos = (self.pos as i64).wrapping_add(off) as u64;
      }
    }
    Ok(self.pos)
  }
}

macro_rules! rawdb_test_file {
  ($make:expr, $model:expr, $test:ident, $file:expr) => {
    #[allow(non_snake_case)]
    #[test]
    fn $test() -> std::result::Result<(), Box<dyn std::error::Error>> {
      //crate::init_test_logger();
      crate::common::check_raw_file_conversion($make, $model, $file)
    }
  };
}

pub(crate) use rawdb_test_file;

pub(crate) fn check_md5_equal(digest: [u8; 16], expected: &str) {
  assert_eq!(hex::encode(digest), expected);
}

pub(crate) fn compare_digest(buf: &[u8], digest_file: impl AsRef<Path>) -> std::result::Result<(), Box<dyn std::error::Error>> {
  let old_digest_str = std::fs::read_to_string(digest_file.as_ref())?;
  let old_digest = Digest(TryInto::<[u8; 16]>::try_into(hex::decode(old_digest_str.trim()).expect("Malformed MD5 digest")).expect("Must be [u8; 16]"));
  let new_digest = md5::compute(buf);
  assert_eq!(old_digest, new_digest, "Old and new digest not match for file {}", digest_file.as_ref().display());
  Ok(())
}

/// Generic function to check camera raw files against
/// pre-generated stats and pixel files.
pub(crate) fn check_raw_file_conversion(make: &str, model: &str, sample: &str) -> std::result::Result<(), Box<dyn std::error::Error>> {
  let rawdb_cache = get_rawdb_cache();
  let testfiles = {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("data/testdata/rawdb");
    p
  };
  let base_path = testfiles.join(make).join(model);
  let raw_file = rawdb_ensure_file(&rawdb_cache, make, model, sample)?;
  let filename = raw_file.file_name().map(|name| name.to_os_string()).expect("Filename must by OS string compatible");
  let mut orig_analyze_file = filename.clone();
  let mut orig_raw_digest_file = filename.clone();
  let mut orig_full_digest_file = filename.clone();
  let mut orig_preview_digest_file = filename.clone();
  let mut orig_thumbnail_digest_file = filename.clone();
  orig_analyze_file.push(".analyze.yaml");
  orig_raw_digest_file.push(".digest.txt");
  orig_full_digest_file.push(".digest.full.txt");
  orig_preview_digest_file.push(".digest.preview.txt");
  orig_thumbnail_digest_file.push(".digest.thumbnail.txt");
  let stats_file = base_path.join(sample).with_file_name(orig_analyze_file);
  let digest_raw_file = base_path.join(sample).with_file_name(orig_raw_digest_file);
  let digest_full_file = base_path.join(sample).with_file_name(orig_full_digest_file);
  let digest_preview_file = base_path.join(sample).with_file_name(orig_preview_digest_file);
  let digest_thumbnail_file = base_path.join(sample).with_file_name(orig_thumbnail_digest_file);

  //let pixel_file = base_path.join(&sample).with_extension("pixel");

  //eprintln!("{:?}", stats_file);

  assert!(raw_file.exists(), "Raw file {:?} not found", raw_file);
  assert!(stats_file.exists(), "Stats file {:?} not found", stats_file);

  // Validate stats file
  let new_stats = analyze_metadata(PathBuf::from(&raw_file)).unwrap();
  let old_stats = std::fs::read_to_string(&stats_file)?;

  let old_stats: AnalyzerResult = serde_yaml::from_str(&old_stats)?;

  assert_eq!(old_stats, new_stats);

  // Validate raw pixel data
  {
    let image = extract_raw_pixels(&raw_file, &RawDecodeParams::default()).unwrap();
    let byte_buf = match &image.data {
      rawler::RawImageData::Integer(samples) => samples.as_slice().as_bytes(),
      rawler::RawImageData::Float(samples) => samples.as_slice().as_bytes(),
    };
    compare_digest(byte_buf, digest_raw_file)?;
  }

  // Validate full pixel data
  {
    let image = extract_full_pixels(&raw_file, &RawDecodeParams::default()).unwrap();
    compare_digest(image.as_bytes(), digest_full_file)?;
  }

  // Validate preview pixel data
  {
    let image = extract_preview_pixels(&raw_file, &RawDecodeParams::default()).unwrap();
    compare_digest(image.as_bytes(), digest_preview_file)?;
  }

  // Validate full pixel data
  {
    let image = extract_thumbnail_pixels(&raw_file, &RawDecodeParams::default()).unwrap();
    compare_digest(image.as_bytes(), digest_thumbnail_file)?;
  }

  // Convert to DNG with default params
  let params = ConvertParams {
    embedded: false,
    apply_scaling: false,
    ..Default::default()
  };
  let mut dng = SeekableSink::new();
  convert_raw_file(&raw_file, &mut dng, &params)?;

  Ok(())
}
