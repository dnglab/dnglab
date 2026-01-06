use std::io::Cursor;

use crate::common::check_md5_equal;
use crate::common::rawdb_file;
use rawler::dng::convert::ConvertParams;
use rawler::dng::convert::convert_raw_file;
use rawler::formats::jfif::Jfif;
use rawler::rawsource::RawSource;
use rawler::{analyze::raw_pixels_digest, decoders::RawDecodeParams};

#[test]
fn dnglab_354_dng_mismatch_tile_dim_vs_ljpeg_sof_dim() -> std::result::Result<(), Box<dyn std::error::Error>> {
  let path = rawdb_file("issues/dnglab_354/dnglab_354.dng");
  let digest = raw_pixels_digest(path, &RawDecodeParams::default())?;
  check_md5_equal(digest, "e5fcd3fd81a3f8e2d9709b92f3b8f546");
  Ok(())
}

#[test]
fn dnglab_366_monochrome_dng_support() -> std::result::Result<(), Box<dyn std::error::Error>> {
  let path = rawdb_file("issues/dnglab_366/dnglab_366.dng");
  let digest = raw_pixels_digest(&path, &RawDecodeParams::default())?;
  check_md5_equal(digest, "f3549fafda97fca90b9993c1278bcd90");
  let mut dng = Cursor::new(Vec::new());
  convert_raw_file(&path, &mut dng, &ConvertParams::default())?;
  Ok(())
}

#[test]
fn dnglab_376_canon_crx_craw_qstep_shl_bug() -> std::result::Result<(), Box<dyn std::error::Error>> {
  {
    let path = rawdb_file("issues/dnglab_376/Canon_EOS_R6M2_CRAW_ISO_25600.CR3");
    let digest = raw_pixels_digest(&path, &RawDecodeParams::default())?;
    check_md5_equal(digest, "66c9fcb6541c90bdfb06d876be5984ec");
    let mut dng = Cursor::new(Vec::new());
    convert_raw_file(&path, &mut dng, &ConvertParams::default())?;
  }
  {
    let path = rawdb_file("issues/dnglab_376/_MGC9382.CR3");
    let digest = raw_pixels_digest(&path, &RawDecodeParams::default())?;
    check_md5_equal(digest, "aef96546a58e5265fb2f7b9e7498cbd0");
    let mut dng = Cursor::new(Vec::new());
    convert_raw_file(&path, &mut dng, &ConvertParams::default())?;
  }
  Ok(())
}

#[test]
fn dnglab_386_catch_jpeg_exif_tiff_ifd_error() -> std::result::Result<(), Box<dyn std::error::Error>> {
  let path = rawdb_file("issues/dnglab_386/jpeg_ifd_error.jpg");
  let rawfile = RawSource::new(&path)?;
  let jfif = Jfif::new(&rawfile)?;
  assert!(jfif.exif_ifd().is_none());
  Ok(())
}

#[test]
fn dnglab_477_jpeg_quantization_table_with_zero_value() -> std::result::Result<(), Box<dyn std::error::Error>> {
  let image = image::open(rawdb_file("issues/dnglab_477/dnglab_477.jpg"))?;
  let _ = image.to_rgb8();
  Ok(())
}
