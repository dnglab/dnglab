use crate::common::check_md5_equal;
use crate::common::rawdb_file;
use rawler::{analyze::raw_pixels_digest, decoders::RawDecodeParams};

#[test]
fn dnglab_354_dng_mismatch_tile_dim_vs_ljpeg_sof_dim() -> std::result::Result<(), Box<dyn std::error::Error>> {
  let path = rawdb_file("issues/dnglab_354/dnglab_354.dng");
  let digest = raw_pixels_digest(path, RawDecodeParams::default())?;
  check_md5_equal(digest, "44e5e63b152719c0ff9eae9e25cdc275");
  Ok(())
}
