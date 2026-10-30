//! Write the linear camera RGB values of a Canon EOS 5D Mark III CR2 of
//! rawdb.dnglab.org to a 16-bit TIFF image
//!
//! The Bayer data is linearized: the black level is subtracted, the values are
//! scaled to the white level (0.0 .. 1.0), demosaiced and cropped to the
//! ActiveArea. They are neither white balanced nor mapped to a color space and
//! have no transfer function, so the image looks dark and green. Demosaicing
//! can produce values outside of 0.0 .. 1.0, the TIFF image clips them.
//!
//! ```sh
//! cargo run -p rawler --release --example linearize [-- <output.tif>]
//! ```

use rawler::{decoders::RawDecodeParams, imgop::develop::RawDevelop, rawdb, rawsource::RawSource};
use std::{fs::File, io::BufWriter, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
  let output = std::env::args_os().nth(1).map_or_else(|| PathBuf::from("linearize.tif"), PathBuf::from);
  let path = rawdb::sample("Canon", "EOS 5D Mark III", "raw_modes/Canon EOS 5D Mark III_RAW_ISO_200.CR2")?;
  println!("Sample: {}", path.display());

  let source = RawSource::new(&path)?;
  let decoder = rawler::get_decoder(&source)?;
  let params = RawDecodeParams::default();
  let rawimage = decoder.raw_image(&source, &params, false)?;
  let metadata = decoder.raw_metadata(&source, &params)?;
  println!("Raw data: {}x{}, {} sample per pixel", rawimage.width, rawimage.height, rawimage.cpp);

  let linear = rawimage.linearize()?;
  println!("Linear image: {}x{}, {} channels", linear.width, linear.height, linear.cpp);
  let samples = linear.data.as_f32();
  for (channel, name) in ["red", "green", "blue"].iter().enumerate() {
    let values = samples.iter().skip(channel).step_by(linear.cpp);
    let (min, max, sum) = values.fold((f32::INFINITY, f32::NEG_INFINITY, 0.0_f64), |(min, max, sum), v| {
      (min.min(*v), max.max(*v), sum + f64::from(*v))
    });
    let mean = sum / (samples.len() / linear.cpp) as f64;
    println!("  {:<5}  min {:.4}  mean {:.4}  max {:.4}", name, min, mean, max);
  }

  // Without processing steps, the linear values are written as they are
  RawDevelop::new_with(&[]).develop(linear, &metadata, BufWriter::new(File::create(&output)?))?;
  println!("Linear image: {}", output.display());
  Ok(())
}
