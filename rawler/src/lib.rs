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

/// Evaluates `$body` if the cargo feature `$feature` is enabled. Otherwise returns
/// from the enclosing function with a [`RawlerError::FeatureDisabled`] error,
/// `$what` describes what needs the feature. `$body` is not compiled when the
/// feature is disabled, so it can reference gated items.
macro_rules! require_feature {
  ($feature:literal, $what:expr, $body:expr) => {{
    #[cfg(feature = $feature)]
    {
      $body
    }
    #[cfg(not(feature = $feature))]
    {
      return Err($crate::RawlerError::feature_disabled($feature, $what).into());
    }
  }};
}

/// [`RawlerError::DecoderFailed`] with a message of `format!` arguments
macro_rules! decoder_error {
  ($($arg:tt)*) => {
    $crate::RawlerError::decoder_failed(format!($($arg)*))
  };
}

/// [`RawlerError::InvalidInput`] with a message of `format!` arguments
macro_rules! input_error {
  ($($arg:tt)*) => {
    $crate::RawlerError::invalid_input(format!($($arg)*))
  };
}

/// [`RawlerError::General`] with a message of `format!` arguments
macro_rules! general_error {
  ($($arg:tt)*) => {
    $crate::RawlerError::general(format!($($arg)*))
  };
}

pub mod analyze;
pub mod bitarray;
pub mod bits;
pub mod buffer;
pub mod camera_profile;
pub mod cfa;
pub mod compressors;
pub mod decoders;
pub mod decompressors;
pub mod devtools;
pub mod dng;
pub(crate) mod envparams;
pub mod exif;
pub(crate) mod fileutils;
pub mod formats;
pub mod imgop;
pub mod lens;
pub mod ljpeg92;
pub mod mask;
pub mod pixarray;
pub mod pumps;
#[cfg(feature = "rawdb")]
pub mod rawdb;
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
use pumps::{BitPumpError, ByteStreamError};
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

/// Error that caused a [`RawlerError`], its context
pub type ErrorSource = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Error of rawler
///
/// The variant is the category of the error. [`Self::DecoderFailed`],
/// [`Self::InvalidInput`] and [`Self::General`] have a message and optionally
/// a context, the error that caused them, which is their
/// [`source()`](std::error::Error::source) and follows the message in their
/// display.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum RawlerError {
  /// The camera, or a mode of it, is not supported
  #[error("Error: {}, model '{}', make: '{}', mode: '{}'", what, model, make, mode)]
  Unsupported { what: String, model: String, make: String, mode: String },

  /// The raw data can't be decoded, e.g. because it is corrupt
  #[error("{}", with_context(message, context))]
  DecoderFailed {
    message: String,
    #[source]
    context: Option<ErrorSource>,
  },

  /// A cargo feature that the operation needs was disabled at build time
  #[error("Feature '{}' is required for {}, but it was disabled at build time", feature, what)]
  FeatureDisabled { feature: &'static str, what: String },

  /// Data or arguments an operation can't process, e.g. a CFA pattern a
  /// demosaic algorithm doesn't support, or an invalid camera profile
  #[error("{}", with_context(message, context))]
  InvalidInput {
    message: String,
    #[source]
    context: Option<ErrorSource>,
  },

  /// Any other error, e.g. a failing compression or encoding of an output file
  #[error("{}", with_context(message, context))]
  General {
    message: String,
    #[source]
    context: Option<ErrorSource>,
  },
}

/// `message`, followed by `context` if there is one
fn with_context(message: &str, context: &Option<ErrorSource>) -> String {
  match context {
    Some(context) => format!("{}: {}", message, context),
    None => message.to_string(),
  }
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

  /// [`Self::DecoderFailed`] with `message`, without a context
  pub fn decoder_failed(message: impl Into<String>) -> Self {
    Self::DecoderFailed {
      message: message.into(),
      context: None,
    }
  }

  /// [`Self::InvalidInput`] with `message`, without a context
  pub fn invalid_input(message: impl Into<String>) -> Self {
    Self::InvalidInput {
      message: message.into(),
      context: None,
    }
  }

  /// [`Self::General`] with `message`, without a context
  pub fn general(message: impl Into<String>) -> Self {
    Self::General {
      message: message.into(),
      context: None,
    }
  }

  /// [`Self::FeatureDisabled`]: `feature` is needed for `what`
  pub fn feature_disabled(feature: &'static str, what: impl Into<String>) -> Self {
    Self::FeatureDisabled { feature, what: what.into() }
  }

  /// [`Self::DecoderFailed`] for an I/O `error` in `context` on the file `path`
  pub fn with_io_error(context: impl AsRef<str>, path: impl AsRef<Path>, error: std::io::Error) -> Self {
    Self::decoder_failed(format!("I/O error in context '{}' on file {}", context.as_ref(), path.as_ref().display())).context(error)
  }

  /// The error with `context` as the error that caused it
  ///
  /// Sets the context of [`Self::DecoderFailed`], [`Self::InvalidInput`] and
  /// [`Self::General`]. The other variants describe the error itself, not a
  /// consequence of another one, they are returned unchanged.
  pub fn context(self, context: impl Into<ErrorSource>) -> Self {
    match self {
      Self::DecoderFailed { message, .. } => Self::DecoderFailed {
        message,
        context: Some(context.into()),
      },
      Self::InvalidInput { message, .. } => Self::InvalidInput {
        message,
        context: Some(context.into()),
      },
      Self::General { message, .. } => Self::General {
        message,
        context: Some(context.into()),
      },
      other => other,
    }
  }
}

impl From<std::io::Error> for RawlerError {
  fn from(err: std::io::Error) -> Self {
    Self::decoder_failed("I/O error").context(err)
  }
}

impl From<TiffError> for RawlerError {
  fn from(err: TiffError) -> Self {
    Self::decoder_failed("Failed to read TIFF data").context(err)
  }
}

impl From<JfifError> for RawlerError {
  fn from(err: JfifError) -> Self {
    Self::decoder_failed("Failed to read JFIF data").context(err)
  }
}

impl From<BitPumpError> for RawlerError {
  #[cold]
  fn from(err: BitPumpError) -> Self {
    Self::decoder_failed("Failed to read the raw data").context(err)
  }
}

impl From<ByteStreamError> for RawlerError {
  #[cold]
  fn from(err: ByteStreamError) -> Self {
    Self::decoder_failed("Failed to read the raw data").context(err)
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

/// Get a decoder for a raw file
///
/// This succeeds for cameras missing in the camera database, too, so metadata,
/// thumbnails and previews can be extracted. Decoding the raw image fails with
/// `RawlerError::Unsupported` for such cameras, see [`Decoder::check_supported()`].
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

#[cfg(test)]
mod tests {
  use std::error::Error;
  use std::io::ErrorKind;

  use super::*;

  #[test]
  fn macros_and_constructors_have_no_context() {
    let (name, planes) = ("RCD", 4);
    let decoder = decoder_error!("{}: strip {} is truncated", name, 3);
    assert!(matches!(&decoder, RawlerError::DecoderFailed { message, context: None } if message == "RCD: strip 3 is truncated"));
    let input = input_error!("{} planes are not supported", planes);
    assert!(matches!(&input, RawlerError::InvalidInput { message, context: None } if message == "4 planes are not supported"));
    let general = general_error!("Failed to write the {} file", "DNG");
    assert!(matches!(&general, RawlerError::General { message, context: None } if message == "Failed to write the DNG file"));
    assert!(matches!(RawlerError::decoder_failed("a"), RawlerError::DecoderFailed { message, context: None } if message == "a"));
    assert!(matches!(RawlerError::invalid_input("b"), RawlerError::InvalidInput { message, context: None } if message == "b"));
    assert!(matches!(RawlerError::general("c"), RawlerError::General { message, context: None } if message == "c"));
    assert!(matches!(
      RawlerError::feature_disabled("crx", "CR3"),
      RawlerError::FeatureDisabled { feature: "crx", what } if what == "CR3"
    ));
    for err in [decoder, input, general] {
      assert!(err.source().is_none(), "{:?}", err);
    }
  }

  #[test]
  fn context_is_the_source() {
    let err = RawlerError::general("Failed to render").context(RawlerError::invalid_input("No color matrix"));
    let source = err.source().and_then(|source| source.downcast_ref::<RawlerError>());
    assert!(
      matches!(source, Some(RawlerError::InvalidInput { message, .. }) if message == "No color matrix"),
      "{:?}",
      source
    );

    let io = RawlerError::decoder_failed("Failed to read").context(std::io::Error::new(ErrorKind::UnexpectedEof, "end of file"));
    assert_eq!(io.source().map(|source| source.to_string()).as_deref(), Some("end of file"));
    let text = RawlerError::invalid_input("Invalid profile").context("no ColorMatrix1");
    assert_eq!(text.source().map(|source| source.to_string()).as_deref(), Some("no ColorMatrix1"));
  }

  #[test]
  fn context_keeps_unsupported_and_feature_disabled() {
    let unsupported = RawlerError::Unsupported {
      what: "Unknown mode".into(),
      model: "Model".into(),
      make: "Make".into(),
      mode: "".into(),
    }
    .context("ignored");
    assert!(matches!(&unsupported, RawlerError::Unsupported { .. }));
    assert!(unsupported.source().is_none());
    let disabled = RawlerError::feature_disabled("crx", "CR3").context("ignored");
    assert!(matches!(&disabled, RawlerError::FeatureDisabled { .. }));
    assert!(disabled.source().is_none());
  }

  #[test]
  fn display_is_message_and_context() {
    assert_eq!(RawlerError::decoder_failed("Strip is truncated").to_string(), "Strip is truncated");
    assert_eq!(
      RawlerError::general("Failed to write").context("disk full").to_string(),
      "Failed to write: disk full"
    );
    let nested = RawlerError::general("outer").context(RawlerError::invalid_input("inner").context("message"));
    assert_eq!(nested.to_string(), "outer: inner: message");
    assert_eq!(
      RawlerError::feature_disabled("crx", "the CRX decompression").to_string(),
      "Feature 'crx' is required for the CRX decompression, but it was disabled at build time"
    );
  }

  #[test]
  fn from_errors_keeps_them_as_context() {
    let io = RawlerError::from(std::io::Error::new(ErrorKind::UnexpectedEof, "unexpected end of file"));
    assert!(matches!(&io, RawlerError::DecoderFailed { .. }));
    assert_eq!(io.to_string(), "I/O error: unexpected end of file");
    let kind = io.source().and_then(|source| source.downcast_ref::<std::io::Error>()).map(|err| err.kind());
    assert_eq!(kind, Some(ErrorKind::UnexpectedEof));

    let tiff = RawlerError::from(TiffError::General("no IFD".into()));
    assert!(matches!(&tiff, RawlerError::DecoderFailed { .. }));
    assert_eq!(tiff.to_string(), "Failed to read TIFF data: General error: no IFD");
    let source = tiff.source().and_then(|source| source.downcast_ref::<TiffError>());
    assert!(matches!(source, Some(TiffError::General(message)) if message == "no IFD"), "{:?}", source);
  }

  #[cfg(all(feature = "crx", not(feature = "inspector")))]
  #[test]
  fn require_feature_fails_for_disabled_features() {
    // Like a dummy decode of CR3 files, `dummy` doesn't need the feature
    fn enabled(dummy: bool) -> Result<u32> {
      Ok(if dummy {
        0
      } else {
        require_feature!("crx", "the test of an enabled feature", 7)
      })
    }
    fn disabled(dummy: bool) -> Result<u32> {
      Ok(if dummy {
        0
      } else {
        require_feature!("inspector", "the test of a disabled feature", 7)
      })
    }
    assert!(matches!(enabled(false), Ok(7)));
    assert!(matches!(disabled(true), Ok(0)));
    let err = disabled(false).err();
    assert!(
      matches!(&err, Some(RawlerError::FeatureDisabled { feature: "inspector", what }) if what == "the test of a disabled feature"),
      "{:?}",
      err
    );
  }
}
