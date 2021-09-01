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
    //missing_debug_implementations,
    //missing_copy_implementations,
    //unsafe_code,
    unstable_features,
    //unused_import_braces,
    //unused_qualifications
  )]

use decoders::Decoder;
use decoders::RawDecodeParams;
use lazy_static::lazy_static;

pub mod analyze;
pub mod bitarray;
pub mod bits;
pub mod cfa;
pub mod decoders;
pub mod decompressors;
pub mod devtools;
pub mod dng;
pub mod formats;
pub mod imgop;
pub mod lens;
pub mod ljpeg92;
pub mod packed;
pub mod pumps;
pub mod rawimage;
pub mod tags;
pub mod tiles;

pub use cfa::CFA;
#[doc(hidden)]
pub use decoders::Buffer;
pub use decoders::Orientation;
#[doc(hidden)]
pub use decoders::RawLoader;
pub use rawimage::RawImage;
pub use rawimage::RawImageData;
use formats::tiff::TiffError;

lazy_static! {
  static ref LOADER: RawLoader = decoders::RawLoader::new();
}

use std::io::Read;
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RawlerError {
  #[error("File is unsupported: {}", _0)]
  Unsupported(String),

  #[error("{}", _0)]
  General(String),
}

pub type Result<T> = std::result::Result<T, RawlerError>;

impl RawlerError {
  pub fn with_io_error(path: impl AsRef<Path>, error: std::io::Error) -> Self {
    Self::General(format!("I/O error on file: {:?}, {}", path.as_ref(), error.to_string()))
  }
}

impl From<&String> for RawlerError {
  fn from(str: &String) -> Self {
    Self::General(str.clone())
  }
}

impl From<String> for RawlerError {
  fn from(str: String) -> Self {
    Self::General(str)
  }
}


impl From<TiffError> for RawlerError {
  fn from(err: TiffError) -> Self {
    Self::General(err.to_string())
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
pub fn decode(reader: &mut dyn Read, params: RawDecodeParams) -> Result<RawImage> {
  LOADER.decode(reader, params, false)
}

// Used to force lazy_static initializations. Useful for fuzzing.
#[doc(hidden)]
pub fn force_initialization() {
  lazy_static::initialize(&LOADER);
}

// Used for fuzzing targets that just want to test the actual decoders instead of the full formats
// with all their TIFF and other crazyness
#[doc(hidden)]
pub fn decode_unwrapped(reader: &mut dyn Read) -> Result<RawImageData> {
  LOADER.decode_unwrapped(reader)
}

// Used for fuzzing everything but the decoders themselves
#[doc(hidden)]
pub fn decode_dummy(reader: &mut dyn Read) -> Result<RawImage> {
  LOADER.decode(reader, RawDecodeParams::default(), true)
}

pub fn get_decoder<'b>(buf: &'b Buffer) -> Result<Box<dyn Decoder + 'b>> {
  LOADER.get_decoder(buf)
}

pub fn raw_image_count_file<P: AsRef<Path>>(path: P) -> Result<usize> {
  LOADER.raw_image_count_file(path.as_ref())
}
