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
//!   let image = rawloader::decode(file).unwrap();
//!
//!   // Write out the image as a grayscale PPM
//!   let mut f = BufWriter::new(File::create(format!("{}.ppm",file)).unwrap());
//!   let preamble = format!("P6 {} {} {}\n", image.width, image.height, 65535).into_bytes();
//!   f.write_all(&preamble).unwrap();
//!   for pix in image.data {
//!     // Do an extremely crude "demosaic" by setting R=G=B
//!     let pixhigh = (pix>>8) as u8;
//!     let pixlow  = (pix&0x0f) as u8;
//!     f.write_all(&[pixhigh, pixlow, pixhigh, pixlow, pixhigh, pixlow]).unwrap()
//!   }
//! }
//! ```
//!
//! To do the image decoding decode the image the same way but then do:
//!
//! ```rust,no_run
//! # use std::env;
//! # use std::fs::File;
//! # use std::io::prelude::*;
//! # use std::io::BufWriter;
//! # extern crate rawloader;
//! # fn main() {
//! #  let args: Vec<_> = env::args().collect();
//! #  if args.len() != 2 {
//! #    println!("Usage: {} <file>", args[0]);
//! #    std::process::exit(2);
//! #  }
//! #  let file = &args[1];
//! #  let image = rawloader::decode(file).unwrap();
//! // Decode to the largest image that fits in 1080p size. If the original image is
//! // smaller this will not scale up but otherwise you will get an image that is either
//! // 1920 pixels wide or 1080 pixels tall and maintains the image ratio.
//! let decoded = image.to_rgb(1920, 1080).unwrap();
//!
//! let mut f = BufWriter::new(File::create(format!("{}.ppm",file)).unwrap());
//! let preamble = format!("P6 {} {} {}\n", decoded.width, decoded.height, 255).into_bytes();
//! f.write_all(&preamble).unwrap();
//! for pix in decoded.data {
//!   let pixel = ((pix.max(0.0)*255.0).min(255.0)) as u8;
//!   f.write_all(&[pixel]).unwrap();
//! }
//! # }
//! ```
//!
//! This is useful as a reference output and if all you need is a thumbnail or a preview
//! it will be a decent output that is produced fast (200-300ms for a 500x500 thumbnail of
//! a 24MP image).

#![deny(
  missing_docs,
  missing_debug_implementations,
  missing_copy_implementations,
  unsafe_code,
  unstable_features,
  unused_import_braces,
)]

#[macro_use] extern crate enum_primitive;
extern crate num;

#[macro_use] extern crate lazy_static;

extern crate itertools;

#[doc(hidden)] pub mod decoders;
pub use decoders::RawImage;
pub use decoders::cfa::CFA;
pub use decoders::RGBImage;
#[doc(hidden)] pub mod imageops;

lazy_static! {
  static ref LOADER: decoders::RawLoader = decoders::RawLoader::new();
}

/// Take a path to a raw file and return a decoded image or an error
///
/// # Example
/// ```rust,ignore
/// let image = match rawloader::decode("path/to/your/file.RAW") {
///   Ok(val) => val,
///   Err(e) => ... some appropriate action when the file is unreadable ...
/// };
/// ```
pub fn decode(path: &str) -> Result<RawImage,String> {
  LOADER.decode_safe(path)
}
