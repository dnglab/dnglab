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
use lazy_static::lazy_static;

  pub mod bits;
  pub mod decoders;
  pub mod decompressors;
  pub mod formats;
  pub mod tags;
  pub mod packed;
  pub mod pumps;
  pub mod tiles;
  pub mod cfa;
  pub mod rawimage;
  pub mod ljpeg92;
  pub mod bitarray;
  pub mod tiff;
  pub mod devtools;
  pub mod dng;
  pub mod lens;
pub mod analyze;

  pub use rawimage::RawImage;
  pub use rawimage::RawImageData;
  pub use decoders::Orientation;
  pub use cfa::CFA;
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
  /// let image = match rawler::decode_file("path/to/your/file.RAW") {
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
  /// let image = match rawler::decode(&mut file) {
  ///   Ok(val) => val,
  ///   Err(e) => ... some appropriate action when the file is unreadable ...
  /// };
  /// ```
  pub fn decode(reader: &mut dyn Read) -> Result<RawImage,RawLoaderError> {
    LOADER.decode(reader, false).map_err(|err| RawLoaderError::new(err))
  }

  // Used to force lazy_static initializations. Useful for fuzzing.
  #[doc(hidden)]
  pub fn force_initialization() {
    lazy_static::initialize(&LOADER);
  }

  // Used for fuzzing targets that just want to test the actual decoders instead of the full formats
  // with all their TIFF and other crazyness
  #[doc(hidden)]
  pub fn decode_unwrapped(reader: &mut dyn Read) -> Result<RawImageData,RawLoaderError> {
    LOADER.decode_unwrapped(reader).map_err(|err| RawLoaderError::new(err))
  }

  // Used for fuzzing everything but the decoders themselves
  #[doc(hidden)]
  pub fn decode_dummy(reader: &mut dyn Read) -> Result<RawImage,RawLoaderError> {
    LOADER.decode(reader, true).map_err(|err| RawLoaderError::new(err))
  }


  pub fn get_decoder<'b>(buf: &'b Buffer) -> Result<Box<dyn Decoder + 'b>, RawLoaderError> {
    LOADER.get_decoder(buf).map_err(|err| RawLoaderError::new(err))
  }
