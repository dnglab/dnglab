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
  clippy::only_used_in_recursion,
  //clippy::seek_from_current, // TODO
  clippy::needless_lifetimes,
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
pub mod tags;
pub mod tiles;

#[doc(hidden)]
pub use buffer::Buffer;
pub use cfa::CFA;
pub use decoders::Orientation;
#[doc(hidden)]
pub use decoders::RawLoader;
use formats::tiff::TiffError;
use md5::Digest;
pub use rawimage::RawImage;
pub use rawimage::RawImageData;

lazy_static! {
  static ref LOADER: RawLoader = decoders::RawLoader::new();
}

use std::fs::File;
use std::io::BufReader;
use std::io::Cursor;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::path::Path;
use std::path::PathBuf;
use thiserror::Error;

pub(crate) const ISSUE_HINT: &str = "Please open an issue at https://github.com/dnglab/dnglab/issues and provide the RAW file.";

pub struct OptBuffer {
  buf: Vec<u8>,
  size: usize,
}

impl OptBuffer {
  pub fn new(buf: Vec<u8>) -> Self {
    let mut padded = buf;
    let size = padded.len();
    padded.extend([0; 16].iter().cloned()); // Pad for optimizations
    Self { buf: padded, size }
  }

  pub fn as_slice(&self) -> &[u8] {
    self.buf.as_slice()
  }

  pub fn len(&self) -> usize {
    self.size
  }

  pub fn is_empty(&self) -> bool {
    self.size == 0
  }
}

impl std::ops::Deref for OptBuffer {
  type Target = [u8];

  fn deref(&self) -> &Self::Target {
    self.as_slice()
  }
}

impl From<Vec<u8>> for OptBuffer {
  fn from(data: Vec<u8>) -> Self {
    Self::new(data)
  }
}

pub trait ReadTrait: Read + Seek {}

impl<T: Read + Seek> ReadTrait for T {}

pub struct RawFile {
  pub path: PathBuf,
  pub file: Box<dyn ReadTrait>,
  pub start_offset: u64,
}

impl RawFile {
  pub fn new<T>(path: impl AsRef<Path>, mut input: T) -> Self
  where
    T: ReadTrait + 'static,
  {
    let start_offset = input.stream_position().expect("Stream position failed");
    Self {
      path: PathBuf::from(path.as_ref()),
      file: Box::new(input),
      start_offset,
    }
  }

  pub fn seek_to_start(&mut self) -> std::io::Result<()> {
    self.file.seek(SeekFrom::Start(self.start_offset))?;
    Ok(())
  }

  /// Calculate digest for file
  pub fn digest(&mut self) -> std::io::Result<Digest> {
    Ok(md5::compute(self.as_vec()?))
  }

  pub fn with_box(mut file: Box<dyn ReadTrait>) -> Self {
    let start_offset = file.stream_position().expect("Stream position failed");
    Self {
      path: PathBuf::new(),
      file,
      start_offset,
    }
  }

  pub fn inner(&mut self) -> &mut Box<dyn ReadTrait> {
    &mut self.file
  }

  pub fn subview(&mut self, offset: u64, size: u64) -> std::io::Result<Vec<u8>> {
    let mut buf = vec![0; size as usize];
    self.file.seek(SeekFrom::Start(offset))?;
    self.file.read_exact(&mut buf)?;
    Ok(buf)
  }

  pub fn subview_until_eof(&mut self, offset: u64) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    self.file.seek(SeekFrom::Start(offset))?;
    self.file.read_to_end(&mut buf)?;
    Ok(buf)
  }

  pub fn as_vec(&mut self) -> std::io::Result<Vec<u8>> {
    let old_pos = self.file.stream_position()?;
    self.file.seek(SeekFrom::Start(self.start_offset))?;
    let mut buf = Vec::new();
    self.file.read_to_end(&mut buf)?;
    self.file.seek(SeekFrom::Start(old_pos))?;
    Ok(buf)
  }

  pub fn stream_len(&mut self) -> std::io::Result<u64> {
    let old_pos = self.file.stream_position()?;
    let len = self.file.seek(SeekFrom::End(0))?;

    // Avoid seeking a third time when we were already at the end of the
    // stream. The branch is usually way cheaper than a seek operation.
    if old_pos != len {
      self.file.seek(SeekFrom::Start(old_pos))?;
    }

    Ok(len)
  }
}

impl From<Buffer> for RawFile {
  fn from(buf: Buffer) -> Self {
    Self {
      path: PathBuf::new(),
      file: Box::new(Cursor::new(buf.buf)),
      start_offset: 0,
    }
  }
}

impl From<BufReader<File>> for RawFile {
  fn from(mut buf: BufReader<File>) -> Self {
    let start_offset = buf.stream_position().expect("Stream position failed");
    Self {
      path: PathBuf::new(), // TODO
      file: Box::new(buf),
      start_offset,
    }
  }
}

impl From<Cursor<Vec<u8>>> for RawFile {
  fn from(buf: Cursor<Vec<u8>>) -> Self {
    Self {
      path: PathBuf::new(),
      file: Box::new(buf),
      start_offset: 0,
    }
  }
}

#[derive(Error, Debug)]
pub enum RawlerError {
  #[error("File is unsupported: {}, model \"{}\", make: \"{}\", mode: \"{}\"", what, model, make, mode)]
  Unsupported { what: String, model: String, make: String, mode: String },

  #[error("{}", _0)]
  General(String),

  #[error("{}", _0)]
  DecoderFailed(String),

  #[error("{}", _0)]
  IOErr(String),
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
    Self::General(format!(
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
    Self::IOErr(format!("I/O Error: {}", err))
  }
}

impl From<&String> for RawlerError {
  fn from(str: &String) -> Self {
    Self::General(str.clone())
  }
}

impl From<&str> for RawlerError {
  fn from(str: &str) -> Self {
    Self::General(str.to_string())
  }
}

impl From<std::fmt::Arguments<'_>> for RawlerError {
  fn from(fmt: std::fmt::Arguments) -> Self {
    Self::General(fmt.to_string())
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

impl From<JfifError> for RawlerError {
  fn from(err: JfifError) -> Self {
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
pub fn decode(rawfile: &mut RawFile, params: RawDecodeParams) -> Result<RawImage> {
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
pub fn decode_unwrapped(rawfile: &mut RawFile) -> Result<RawImageData> {
  LOADER.decode_unwrapped(rawfile)
}

// Used for fuzzing everything but the decoders themselves
#[doc(hidden)]
pub fn decode_dummy(rawfile: &mut RawFile) -> Result<RawImage> {
  LOADER.decode(rawfile, RawDecodeParams::default(), true)
}

pub fn get_decoder(rawfile: &mut RawFile) -> Result<Box<dyn Decoder>> {
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
