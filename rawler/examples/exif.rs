//! Print the camera, exposure time, ISO and aperture of a raw file
//!
//! Without an argument, it reads a Canon EOS 5D Mark III CR2 of rawdb.dnglab.org.
//!
//! ```sh
//! cargo run -p rawler --release --example exif [-- <raw file>]
//! ```

use rawler::{decoders::RawDecodeParams, formats::tiff::Rational, rawdb, rawsource::RawSource};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
  let path = match std::env::args_os().nth(1) {
    Some(path) => PathBuf::from(path),
    None => rawdb::sample("Canon", "EOS 5D Mark III", "raw_modes/Canon EOS 5D Mark III_RAW_ISO_200.CR2")?,
  };
  println!("File: {}", path.display());

  let source = RawSource::new(&path)?;
  let decoder = rawler::get_decoder(&source)?;
  let metadata = decoder.raw_metadata(&source, &RawDecodeParams::default())?;
  let exif = &metadata.exif;

  let unknown = || "unknown".to_string();
  let iso = exif.iso_speed_ratings.map(u32::from).or(exif.iso_speed).or(exif.recommended_exposure_index);
  println!("Camera:   {} {}", metadata.make, metadata.model);
  println!("Exposure: {}", exif.exposure_time.map_or_else(unknown, exposure_time));
  println!("ISO:      {}", iso.map_or_else(unknown, |iso| iso.to_string()));
  println!("Aperture: {}", exif.fnumber.map_or_else(unknown, |f| format!("f/{:.1}", f.as_f32())));
  Ok(())
}

/// Exposure time as fraction below one second, e.g. "1/125 s" or "2.5 s"
fn exposure_time(time: Rational) -> String {
  let seconds = time.as_f32();
  if seconds > 0.0 && seconds < 1.0 {
    format!("1/{:.0} s", 1.0 / seconds)
  } else {
    format!("{} s", seconds)
  }
}
