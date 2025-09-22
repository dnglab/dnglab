//! Library to extract the raw data and some metadata from digital camera
//! images. Given an image in a supported format and camera you will be able to get
//! everything needed to process the image
//!
//! # Example
//! ```rust,no_run
//! use std::env;
//! use std::fs::File;
//! use std::io::prelude::*;
//! use std::io::BufWriter;
//!
//! fn main() {
//!   let args: Vec<_> = env::args().collect();
//!   if args.len() != 2 {
//!     println!("Usage: {} <file>", args[0]);
//!     std::process::exit(2);
//!   }
//!   let file = &args[1];
//!   let image = rawler::decode_file(file).unwrap();
//!
//!   // Write out the image as a grayscale PPM
//!   let mut f = BufWriter::new(File::create(format!("{}.ppm",file)).unwrap());
//!   let preamble = format!("P6 {} {} {}\n", image.width, image.height, 65535).into_bytes();
//!   f.write_all(&preamble).unwrap();
//!   if let rawler::RawImageData::Integer(data) = image.data {
//!     for pix in data {
//!       // Do an extremely crude "demosaic" by setting R=G=B
//!       let pixhigh = (pix>>8) as u8;
//!       let pixlow  = (pix&0x0f) as u8;
//!       f.write_all(&[pixhigh, pixlow, pixhigh, pixlow, pixhigh, pixlow]).unwrap()
//!     }
//!   } else {
//!     eprintln!("Don't know how to process non-integer raw files");
//!   }
//! }
//! ```

#![deny(
    //missing_docs,
    unstable_features,
    //unused_import_braces,
    //unused_qualifications
  )]
// Clippy configuration
#![allow(
  clippy::needless_doctest_main,
  clippy::identity_op, // we often use x + 0 to better document an algorithm
  clippy::too_many_arguments,
  clippy::bool_assert_comparison,
  clippy::upper_case_acronyms,
  clippy::eq_op,
  clippy::needless_range_loop,
  clippy::manual_range_patterns,
  clippy::unnecessary_cast,
  clippy::get_first,
  clippy::vec_init_then_push,
  clippy::only_used_in_recursion,
  //clippy::seek_from_current, // TODO
  clippy::needless_lifetimes,
  clippy::type_complexity,
  //clippy::cast_abs_to_unsigned,
  //clippy::needless_return,
  //clippy::derivable_impls,
  //clippy::useless_vec,
)]

use decoders::Camera;
use decoders::Decoder;
use decoders::RawDecodeParams;
use formats::jfif::JfifError;
use lazy_static::lazy_static;

pub mod analyze;
pub mod bitarray;
pub mod bits;
pub mod buffer;
pub mod cfa;
pub mod decoders;
pub mod decompressors;
pub mod devtools;
pub mod dng;
pub(crate) mod envparams;
pub mod exif;
pub mod formats;
pub mod imgop;
pub mod lens;
pub mod ljpeg92;
pub mod packed;
pub mod pixarray;
pub mod pumps;
pub mod rawimage;
pub mod rawsource;
pub mod tags;
pub mod tiles;

#[doc(hidden)]
pub use cfa::CFA;
pub use decoders::Orientation;
#[doc(hidden)]
pub use decoders::RawLoader;
use formats::tiff::TiffError;
pub use rawimage::RawImage;
pub use rawimage::RawImageData;
use rawsource::RawSource;

lazy_static! {
  static ref LOADER: RawLoader = decoders::RawLoader::new();
}

use std::io::Read;
use std::io::Seek;
use std::path::Path;
use thiserror::Error;

pub(crate) const ISSUE_HINT: &str = "Please open an issue at https://github.com/dnglab/dnglab/issues and provide this message (optionally the RAW file, if you can license it under CC0-license).";

pub trait ReadTrait: Read + Seek {}

impl<T: Read + Seek> ReadTrait for T {}

// Function wrappers for macros to support import pattern
#[doc(hidden)]
pub fn alloc_image_plain(width: usize, height: usize, dummy: bool) -> pixarray::PixU16 {
  alloc_image_plain!(width, height, dummy)
}

#[doc(hidden)]
pub fn alloc_image_f32_plain(width: usize, height: usize, dummy: bool) -> pixarray::PixF32 {
  alloc_image_f32_plain!(width, height, dummy)
}

#[doc(hidden)]
pub fn alloc_image_ok(width: usize, height: usize, dummy: bool) -> Result<pixarray::PixU16> {
  if dummy {
    Ok(pixarray::PixU16::new_uninit(width, height))
  } else {
    Ok(alloc_image_plain!(width, height, dummy))
  }
}

#[derive(Error, Debug)]
pub enum RawlerError {
  #[error("Error: {}, model '{}', make: '{}', mode: '{}'", what, model, make, mode)]
  Unsupported { what: String, model: String, make: String, mode: String },

  #[error("Failed to decode image, possibly corrupt image. Origin error was: {}", _0)]
  DecoderFailed(String),
}

pub type Result<T> = std::result::Result<T, RawlerError>;

impl RawlerError {
  pub fn unsupported(camera: &Camera, what: impl AsRef<str>) -> Self {
    Self::Unsupported {
      what: what.as_ref().to_string(),
      model: camera.model.clone(),
      make: camera.make.clone(),
      mode: camera.mode.clone(),
    }
  }

  pub fn with_io_error(context: impl AsRef<str>, path: impl AsRef<Path>, error: std::io::Error) -> Self {
    Self::DecoderFailed(format!(
      "I/O error in context '{}', {} on file: {}",
      context.as_ref(),
      error,
      path.as_ref().display()
    ))
  }
}

impl From<std::io::Error> for RawlerError {
  fn from(err: std::io::Error) -> Self {
    log::error!("I/O error: {}", err.to_string());
    log::error!("Backtrace:\n{:?}", backtrace::Backtrace::new());
    Self::DecoderFailed(format!("I/O Error without context: {}", err))
  }
}

impl From<&String> for RawlerError {
  fn from(str: &String) -> Self {
    Self::DecoderFailed(str.clone())
  }
}

impl From<&str> for RawlerError {
  fn from(str: &str) -> Self {
    Self::DecoderFailed(str.to_string())
  }
}

impl From<std::fmt::Arguments<'_>> for RawlerError {
  fn from(fmt: std::fmt::Arguments) -> Self {
    Self::DecoderFailed(fmt.to_string())
  }
}

impl From<String> for RawlerError {
  fn from(str: String) -> Self {
    Self::DecoderFailed(str)
  }
}

impl From<TiffError> for RawlerError {
  fn from(err: TiffError) -> Self {
    Self::DecoderFailed(err.to_string())
  }
}

impl From<JfifError> for RawlerError {
  fn from(err: JfifError) -> Self {
    Self::DecoderFailed(err.to_string())
  }
}

/// Take a path to a raw file and return a decoded image or an error
///
/// # Example
/// ```rust,ignore
/// let image = match rawler::decode_file("path/to/your/file.RAW") {
///   Ok(val) => val,
///   Err(e) => ... some appropriate action when the file is unreadable ...
/// };
/// ```
pub fn decode_file<P: AsRef<Path>>(path: P) -> Result<RawImage> {
  LOADER.decode_file(path.as_ref())
}

/// Take a readable source and return a decoded image or an error
///
/// # Example
/// ```rust,ignore
/// let mut file = match File::open(path).unwrap();
/// let image = match rawler::decode(&mut file) {
///   Ok(val) => val,
///   Err(e) => ... some appropriate action when the file is unreadable ...
/// };
/// ```
pub fn decode(rawfile: &RawSource, params: &RawDecodeParams) -> Result<RawImage> {
  LOADER.decode(rawfile, params, false)
}

// Used to force lazy_static initializations. Useful for fuzzing.
#[doc(hidden)]
pub fn force_initialization() {
  lazy_static::initialize(&LOADER);
}

// Used for fuzzing targets that just want to test the actual decoders instead of the full formats
// with all their TIFF and other crazyness
#[doc(hidden)]
pub fn decode_unwrapped(rawfile: &RawSource) -> Result<RawImageData> {
  LOADER.decode_unwrapped(rawfile)
}

// Used for fuzzing everything but the decoders themselves
#[doc(hidden)]
pub fn decode_dummy(rawfile: &RawSource) -> Result<RawImage> {
  LOADER.decode(rawfile, &RawDecodeParams::default(), true)
}

pub fn get_decoder(rawfile: &RawSource) -> Result<Box<dyn Decoder>> {
  LOADER.get_decoder(rawfile)
}

pub fn raw_image_count_file<P: AsRef<Path>>(path: P) -> Result<usize> {
  LOADER.raw_image_count_file(path.as_ref())
}

pub fn global_loader() -> &'static RawLoader {
  &LOADER
}

#[cfg(test)]
pub(crate) fn init_test_logger() {
  let _ = env_logger::builder().is_test(true).filter_level(log::LevelFilter::Debug).try_init();
}
