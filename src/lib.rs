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
//! extern crate rawloader;
//!
//! fn main() {
//!   let args: Vec<_> = env::args().collect();
//!   if args.len() != 2 {
//!     println!("Usage: {} <file>", args[0]);
//!     std::process::exit(2);
//!   }
//!   let file = &args[1];
//!   let image = rawloader::decode_file(file).unwrap();
//!
//!   // Write out the image as a grayscale PPM
//!   let mut f = BufWriter::new(File::create(format!("{}.ppm",file)).unwrap());
//!   let preamble = format!("P6 {} {} {}\n", image.width, image.height, 65535).into_bytes();
//!   f.write_all(&preamble).unwrap();
//!   if let rawloader::RawImageData::Integer(data) = image.data {
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
  missing_docs,
  missing_debug_implementations,
  missing_copy_implementations,
  unsafe_code,
  unstable_features,
  unused_import_braces,
  unused_qualifications
)]

#[macro_use] extern crate enum_primitive;
extern crate num;
#[macro_use] extern crate lazy_static;
#[macro_use] extern crate serde_derive;
extern crate itertools;

mod decoders;
pub use decoders::RawImage;
pub use decoders::RawImageData;
pub use decoders::Orientation;
pub use decoders::cfa::CFA;
#[doc(hidden)] pub use decoders::Buffer;
#[doc(hidden)] pub use decoders::RawLoader;

lazy_static! {
  static ref LOADER: RawLoader = decoders::RawLoader::new();
}

use std::path::Path;
use std::error::Error;
use std::fmt;
use std::io::Read;

/// Error type for any reason for the decode to fail
#[derive(Debug)]
pub struct RawLoaderError {
  msg: String,
}

impl fmt::Display for RawLoaderError {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    write!(f, "RawLoaderError: \"{}\"", self.msg)
  }
}

impl Error for RawLoaderError {
  // Implement description so that older versions of rust still work
  fn description(&self) -> &str {
    "description() is deprecated; use Display"
  }
}

impl RawLoaderError {
  fn new(msg: String) -> Self {
    Self {
      msg,
    }
  }
}

/// Take a path to a raw file and return a decoded image or an error
///
/// # Example
/// ```rust,ignore
/// let image = match rawloader::decode_file("path/to/your/file.RAW") {
///   Ok(val) => val,
///   Err(e) => ... some appropriate action when the file is unreadable ...
/// };
/// ```
pub fn decode_file<P: AsRef<Path>>(path: P) -> Result<RawImage,RawLoaderError> {
  LOADER.decode_file(path.as_ref()).map_err(|err| RawLoaderError::new(err))
}

/// Take a readable source and return a decoded image or an error
///
/// # Example
/// ```rust,ignore
/// let mut file = match File::open(path).unwrap();
/// let image = match rawloader::decode(&mut file) {
///   Ok(val) => val,
///   Err(e) => ... some appropriate action when the file is unreadable ...
/// };
/// ```
pub fn decode(reader: &mut Read) -> Result<RawImage,RawLoaderError> {
  LOADER.decode(reader).map_err(|err| RawLoaderError::new(err))
}

// Used to force lazy_static initializations. Useful for fuzzing.
#[doc(hidden)]
pub fn force_initialization() {
  lazy_static::initialize(&LOADER);
  lazy_static::initialize(&decoders::CRW_HUFF_TABLES);
  lazy_static::initialize(&decoders::SNEF_CURVE);
}

// Used for fuzzing targets that just want to test the actual decoders instead of the full formats
// with all their TIFF and other crazyness
#[doc(hidden)]
pub fn decode_unwrapped(reader: &mut Read) -> Result<RawImageData,RawLoaderError> {
  LOADER.decode_unwrapped(reader).map_err(|err| RawLoaderError::new(err))
}
