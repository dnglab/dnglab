//! Develop a sample of rawdb.dnglab.org to a JPEG image
//!
//! The sample is downloaded into the rawdb cache when it is used the first
//! time, see [`rawler::rawdb`] for the location of the cache. Run with:
//!
//! ```sh
//! cargo run -p rawler --release --example develop [-- <output.jpg>]
//! ```

use rawler::{decoders::RawDecodeParams, imgop::develop::RawDevelop, rawdb, rawsource::RawSource};
use std::{fs::File, io::BufWriter, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
  let output = std::env::args_os().nth(1).map_or_else(|| PathBuf::from("develop.jpg"), PathBuf::from);

  // Path of the sample in the cache, it is downloaded if missing
  let path = rawdb::sample("Canon", "EOS 5D Mark III", "raw_modes/Canon EOS 5D Mark III_RAW_ISO_200.CR2")?;
  println!("Sample: {}", path.display());

  // Decode the raw image and its metadata
  let source = RawSource::new(&path)?;
  let decoder = rawler::get_decoder(&source)?;
  let params = RawDecodeParams::default();
  let rawimage = decoder.raw_image(&source, &params, false)?;
  let metadata = decoder.raw_metadata(&source, &params)?;

  // Develop it to sRGB with the default processing steps
  RawDevelop::default().develop_jpeg(rawimage, &metadata, BufWriter::new(File::create(&output)?), 90)?;
  println!("Developed image: {}", output.display());
  Ok(())
}
