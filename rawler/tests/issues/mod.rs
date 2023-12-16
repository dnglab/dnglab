use std::io::Cursor;

use crate::common::check_md5_equal;
use crate::common::rawdb_file;
use rawler::dng::convert::convert_raw_file;
use rawler::dng::convert::ConvertParams;
use rawler::{analyze::raw_pixels_digest, decoders::RawDecodeParams};

#[test]
fn dnglab_354_dng_mismatch_tile_dim_vs_ljpeg_sof_dim() -> std::result::Result<(), Box<dyn std::error::Error>> {
  let path = rawdb_file("issues/dnglab_354/dnglab_354.dng");
  let digest = raw_pixels_digest(path, RawDecodeParams::default())?;
  check_md5_equal(digest, "44e5e63b152719c0ff9eae9e25cdc275");
  Ok(())
}

#[test]
fn dnglab_366_monochrome_dng_support() -> std::result::Result<(), Box<dyn std::error::Error>> {
  let path = rawdb_file("issues/dnglab_366/dnglab_366.dng");
  let digest = raw_pixels_digest(&path, RawDecodeParams::default())?;
  check_md5_equal(digest, "f3549fafda97fca90b9993c1278bcd90");
  let mut dng = Cursor::new(Vec::new());
  convert_raw_file(&path, &mut dng, &ConvertParams::default())?;
  Ok(())
}
