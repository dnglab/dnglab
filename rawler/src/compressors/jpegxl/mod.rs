// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! JPEG XL encoder for DNG tiles.
//!
//! The encoder uses the libjxl of the system, which is loaded at runtime. Nothing is
//! required at build time; if libjxl is missing or too old, encoding fails with an
//! error. Set the environment variable `RAWLER_LIBJXL` to load libjxl from a
//! specific path.

pub mod ffi;

use std::ffi::{OsString, c_void};
use std::sync::OnceLock;

use libloading::Library;

use ffi::*;

/// Environment variable with the path of the libjxl shared library to load
pub const LIBJXL_ENV: &str = "RAWLER_LIBJXL";

/// Lowest supported libjxl version
pub const LIBJXL_MIN_VERSION: (u32, u32, u32) = (0, 11, 0);

#[cfg(target_os = "windows")]
const LIBJXL_CANDIDATES: &[&str] = &["jxl.dll", "libjxl.dll"];

#[cfg(target_os = "macos")]
const LIBJXL_CANDIDATES: &[&str] = &[
  "libjxl.0.12.dylib",
  "libjxl.0.11.dylib",
  "libjxl.dylib",
  "/opt/homebrew/lib/libjxl.0.12.dylib",
  "/opt/homebrew/lib/libjxl.0.11.dylib",
  "/opt/homebrew/lib/libjxl.dylib",
  "/usr/local/lib/libjxl.0.12.dylib",
  "/usr/local/lib/libjxl.0.11.dylib",
  "/usr/local/lib/libjxl.dylib",
];

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const LIBJXL_CANDIDATES: &[&str] = &["libjxl.so.0.12", "libjxl.so.0.11", "libjxl.so"];

/// Samples of a tile, interleaved per pixel
#[derive(Clone, Copy, Debug)]
pub enum TileSamples<'a> {
  /// Unsigned 16 bit integers
  U16(&'a [u16]),
  /// IEEE 754 half precision floats (bit patterns)
  F16(&'a [u16]),
}

/// Transfer function declared for lossy compressed data
///
/// Lossy compression uses the XYB colour space, so the declared transfer function
/// tells the encoder how the samples relate to light. Lossless data is stored as it is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LossyTransfer {
  /// Curve encoded samples, declared as sRGB transfer
  Srgb,
  /// Linear samples
  Linear,
}

/// Encoder settings for a tile
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileSettings {
  /// Butteraugli distance, 0.0 is lossless
  pub distance: f32,
  /// Effort (1 to 9)
  pub effort: u32,
  /// Decoding speed (1 to 4), only used for lossy compression
  pub decode_speed: u32,
  /// Transfer function declared for lossy compression
  pub lossy_transfer: LossyTransfer,
}

impl TileSettings {
  /// Check if the settings describe lossless compression
  pub fn is_lossless(&self) -> bool {
    self.distance == 0.0
  }
}

/// Functions of the loaded libjxl
struct LibJxl {
  /// Keeps the library loaded, the function pointers below point into it
  _library: Library,
  version: (u32, u32, u32),
  create: JxlEncoderCreateFn,
  destroy: JxlEncoderDestroyFn,
  get_error: JxlEncoderGetErrorFn,
  use_container: JxlEncoderUseContainerFn,
  init_basic_info: JxlEncoderInitBasicInfoFn,
  set_basic_info: JxlEncoderSetBasicInfoFn,
  color_encoding_set_to_srgb: JxlColorEncodingSetToSrgbFn,
  set_color_encoding: JxlEncoderSetColorEncodingFn,
  frame_settings_create: JxlEncoderFrameSettingsCreateFn,
  set_frame_lossless: JxlEncoderSetFrameLosslessFn,
  set_frame_distance: JxlEncoderSetFrameDistanceFn,
  frame_settings_set_option: JxlEncoderFrameSettingsSetOptionFn,
  add_image_frame: JxlEncoderAddImageFrameFn,
  close_input: JxlEncoderCloseInputFn,
  process_output: JxlEncoderProcessOutputFn,
}

/// Get a function of `library` by its name
///
/// # Safety
/// `T` must be the function pointer type of the symbol.
unsafe fn symbol<T: Copy>(library: &Library, name: &str) -> Result<T, String> {
  let symbol = unsafe { library.get::<T>(name.as_bytes()) }.map_err(|err| format!("libjxl has no function {}: {}", name, err))?;
  Ok(*symbol)
}

impl LibJxl {
  /// Load libjxl from `RAWLER_LIBJXL` or the default library names
  fn load() -> Result<Self, String> {
    let candidates: Vec<OsString> = match std::env::var_os(LIBJXL_ENV) {
      Some(path) => vec![path],
      None => LIBJXL_CANDIDATES.iter().map(OsString::from).collect(),
    };
    let mut errors = Vec::new();
    for candidate in &candidates {
      // SAFETY: loading libjxl runs no initialization code with preconditions
      match unsafe { Library::new(candidate) } {
        Ok(library) => return Self::from_library(library, candidate),
        Err(err) => errors.push(format!("{}: {}", candidate.to_string_lossy(), err)),
      }
    }
    Err(format!(
      "libjxl {}.{}.{} or later not found, install libjxl or set {} to its path (tried {})",
      LIBJXL_MIN_VERSION.0,
      LIBJXL_MIN_VERSION.1,
      LIBJXL_MIN_VERSION.2,
      LIBJXL_ENV,
      errors.join("; ")
    ))
  }

  fn from_library(library: Library, name: &OsString) -> Result<Self, String> {
    // SAFETY: the function pointer types match the libjxl API
    unsafe {
      let version_fn: JxlEncoderVersionFn = symbol(&library, "JxlEncoderVersion")?;
      let encoded = version_fn();
      let version = (encoded / 1_000_000, encoded / 1_000 % 1_000, encoded % 1_000);
      if version < LIBJXL_MIN_VERSION {
        return Err(format!(
          "libjxl {}.{}.{} ({}) is too old, version {}.{}.{} or later is required",
          version.0,
          version.1,
          version.2,
          name.to_string_lossy(),
          LIBJXL_MIN_VERSION.0,
          LIBJXL_MIN_VERSION.1,
          LIBJXL_MIN_VERSION.2
        ));
      }
      log::debug!("Loaded libjxl {}.{}.{} from {}", version.0, version.1, version.2, name.to_string_lossy());
      Ok(Self {
        version,
        create: symbol(&library, "JxlEncoderCreate")?,
        destroy: symbol(&library, "JxlEncoderDestroy")?,
        get_error: symbol(&library, "JxlEncoderGetError")?,
        use_container: symbol(&library, "JxlEncoderUseContainer")?,
        init_basic_info: symbol(&library, "JxlEncoderInitBasicInfo")?,
        set_basic_info: symbol(&library, "JxlEncoderSetBasicInfo")?,
        color_encoding_set_to_srgb: symbol(&library, "JxlColorEncodingSetToSRGB")?,
        set_color_encoding: symbol(&library, "JxlEncoderSetColorEncoding")?,
        frame_settings_create: symbol(&library, "JxlEncoderFrameSettingsCreate")?,
        set_frame_lossless: symbol(&library, "JxlEncoderSetFrameLossless")?,
        set_frame_distance: symbol(&library, "JxlEncoderSetFrameDistance")?,
        frame_settings_set_option: symbol(&library, "JxlEncoderFrameSettingsSetOption")?,
        add_image_frame: symbol(&library, "JxlEncoderAddImageFrame")?,
        close_input: symbol(&library, "JxlEncoderCloseInput")?,
        process_output: symbol(&library, "JxlEncoderProcessOutput")?,
        _library: library,
      })
    }
  }
}

static LIBJXL: OnceLock<Result<LibJxl, String>> = OnceLock::new();

/// Get the loaded libjxl, it is loaded on first use
fn libjxl() -> Result<&'static LibJxl, String> {
  LIBJXL.get_or_init(LibJxl::load).as_ref().map_err(Clone::clone)
}

/// Version of the system libjxl, loads libjxl if required
pub fn libjxl_version() -> Result<(u32, u32, u32), String> {
  libjxl().map(|lib| lib.version)
}

/// Encoder instance, destroyed on drop
struct Encoder {
  lib: &'static LibJxl,
  ptr: *mut JxlEncoder,
}

impl Encoder {
  fn new(lib: &'static LibJxl) -> Result<Self, String> {
    // SAFETY: a null memory manager selects the default allocator
    let ptr = unsafe { (lib.create)(std::ptr::null()) };
    if ptr.is_null() {
      return Err("JxlEncoderCreate failed".into());
    }
    Ok(Self { lib, ptr })
  }

  /// Turn a libjxl status into a result, `step` names the failed call
  fn check(&self, status: JxlEncoderStatus, step: &str) -> Result<(), String> {
    if status == JXL_ENC_SUCCESS {
      Ok(())
    } else {
      // SAFETY: the encoder is valid
      let error = unsafe { (self.lib.get_error)(self.ptr) };
      Err(format!("JPEG XL encoding failed in {}: {}", step, error_name(error)))
    }
  }
}

impl Drop for Encoder {
  fn drop(&mut self) {
    // SAFETY: the encoder is valid and not used afterwards, this also frees the frame settings
    unsafe { (self.lib.destroy)(self.ptr) }
  }
}

/// Encode a tile into a bare JPEG XL codestream
///
/// `samples` holds `width` x `height` pixels with `channels` (1 or 3) samples each.
/// Lossless data keeps its original colour space; lossy data is stored in XYB and
/// declared as gray or RGB (Rec. 2100 primaries, D65) with the transfer function of
/// [`TileSettings::lossy_transfer`].
pub fn encode_tile(samples: TileSamples<'_>, width: usize, height: usize, channels: usize, settings: &TileSettings) -> Result<Vec<u8>, String> {
  let (data, exponent_bits_per_sample, data_type) = match samples {
    TileSamples::U16(data) => (data, 0, JXL_TYPE_UINT16),
    TileSamples::F16(data) => (data, 5, JXL_TYPE_FLOAT16),
  };
  if channels != 1 && channels != 3 {
    return Err(format!("JPEG XL encoding supports 1 or 3 channels, got {}", channels));
  }
  if width == 0 || height == 0 || data.len() != width * height * channels {
    return Err(format!(
      "JPEG XL tile {}x{} with {} channels does not match {} samples",
      width,
      height,
      channels,
      data.len()
    ));
  }
  let lossless = settings.is_lossless();
  let lib = libjxl()?;
  let enc = Encoder::new(lib)?;

  // SAFETY: all pointers passed to libjxl are valid for the duration of the calls,
  // the frame settings are owned by the encoder.
  unsafe {
    enc.check((lib.use_container)(enc.ptr, JXL_FALSE), "JxlEncoderUseContainer")?;

    let mut info = JxlBasicInfo::default();
    (lib.init_basic_info)(&mut info);
    info.xsize = width as u32;
    info.ysize = height as u32;
    info.intrinsic_xsize = width as u32;
    info.intrinsic_ysize = height as u32;
    info.bits_per_sample = 16;
    info.exponent_bits_per_sample = exponent_bits_per_sample;
    info.num_color_channels = channels as u32;
    info.num_extra_channels = 0;
    info.alpha_bits = 0;
    info.uses_original_profile = if lossless { JXL_TRUE } else { JXL_FALSE };
    enc.check((lib.set_basic_info)(enc.ptr, &info), "JxlEncoderSetBasicInfo")?;

    let mut color = JxlColorEncoding::default();
    if lossless {
      (lib.color_encoding_set_to_srgb)(&mut color, if channels == 1 { JXL_TRUE } else { JXL_FALSE });
    } else {
      color.color_space = if channels == 1 { JXL_COLOR_SPACE_GRAY } else { JXL_COLOR_SPACE_RGB };
      color.white_point = JXL_WHITE_POINT_D65;
      color.primaries = JXL_PRIMARIES_2100;
      color.transfer_function = match settings.lossy_transfer {
        LossyTransfer::Srgb => JXL_TRANSFER_FUNCTION_SRGB,
        LossyTransfer::Linear => JXL_TRANSFER_FUNCTION_LINEAR,
      };
      color.rendering_intent = JXL_RENDERING_INTENT_RELATIVE;
    }
    enc.check((lib.set_color_encoding)(enc.ptr, &color), "JxlEncoderSetColorEncoding")?;

    let frame = (lib.frame_settings_create)(enc.ptr, std::ptr::null());
    if frame.is_null() {
      return Err("JPEG XL encoding failed in JxlEncoderFrameSettingsCreate".into());
    }
    let set_option = |option, value: i64, step| enc.check((lib.frame_settings_set_option)(frame, option, value), step);
    if lossless {
      enc.check((lib.set_frame_lossless)(frame, JXL_TRUE), "JxlEncoderSetFrameLossless")?;
    } else {
      enc.check((lib.set_frame_distance)(frame, settings.distance), "JxlEncoderSetFrameDistance")?;
      set_option(JXL_ENC_FRAME_SETTING_DECODING_SPEED, i64::from(settings.decode_speed), "decoding speed")?;
      set_option(JXL_ENC_FRAME_SETTING_PATCHES, 0, "patches")?;
      if settings.distance > 0.02 {
        // Reduce blockiness and artifacts along strong edges
        set_option(JXL_ENC_FRAME_SETTING_GABORISH, 1, "gaborish")?;
        set_option(JXL_ENC_FRAME_SETTING_EPF, 3, "edge preserving filter")?;
      }
    }
    set_option(JXL_ENC_FRAME_SETTING_EFFORT, i64::from(settings.effort), "effort")?;

    let format = JxlPixelFormat {
      num_channels: channels as u32,
      data_type,
      endianness: JXL_NATIVE_ENDIAN,
      align: 0,
    };
    enc.check(
      (lib.add_image_frame)(frame, &format, data.as_ptr().cast::<c_void>(), std::mem::size_of_val(data)),
      "JxlEncoderAddImageFrame",
    )?;
    (lib.close_input)(enc.ptr);

    let mut output = vec![0_u8; std::mem::size_of_val(data) / 2 + 4096];
    let mut written = 0;
    loop {
      let mut next = output.as_mut_ptr().add(written);
      let mut avail = output.len() - written;
      let status = (lib.process_output)(enc.ptr, &mut next, &mut avail);
      written = output.len() - avail;
      match status {
        JXL_ENC_SUCCESS => {
          output.truncate(written);
          return Ok(output);
        }
        JXL_ENC_NEED_MORE_OUTPUT => output.resize(output.len() * 2, 0),
        status => enc.check(status, "JxlEncoderProcessOutput")?,
      }
    }
  }
}

/// Check if libjxl can be used in tests
///
/// Tests that need libjxl are skipped if it can't be loaded, unless the
/// environment variable `RAWLER_REQUIRE_LIBJXL` is set.
#[cfg(test)]
pub(crate) fn libjxl_available_for_tests() -> bool {
  match libjxl_version() {
    Ok(_) => true,
    Err(err) if std::env::var_os("RAWLER_REQUIRE_LIBJXL").is_some() => panic!("libjxl is required: {}", err),
    Err(err) => {
      eprintln!("Skipping test, libjxl is not available: {}", err);
      false
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use jxl_oxide::JxlImage;

  const LOSSLESS: TileSettings = TileSettings {
    distance: 0.0,
    effort: 7,
    decode_speed: 4,
    lossy_transfer: LossyTransfer::Srgb,
  };

  /// Synthetic samples over the full u16 range
  fn ramp(len: usize) -> Vec<u16> {
    (0..len).map(|i| ((i * 7919) % 65536) as u16).collect()
  }

  /// Decode a codestream, returns the image and its samples as f32
  fn decode(data: &[u8]) -> (JxlImage, Vec<f32>) {
    let image = JxlImage::builder().read(data).expect("valid JPEG XL");
    let render = image.render_frame(0).expect("rendered frame");
    let mut stream = render.stream_no_alpha();
    let mut samples = vec![0.0_f32; stream.width() as usize * stream.height() as usize * stream.channels() as usize];
    stream.write_to_buffer(&mut samples);
    (image, samples)
  }

  #[test]
  fn lossless_integer_roundtrip() -> Result<(), String> {
    if !libjxl_available_for_tests() {
      return Ok(());
    }
    for channels in [1, 3] {
      let samples = ramp(40 * 30 * channels);
      let data = encode_tile(TileSamples::U16(&samples), 40, 30, channels, &LOSSLESS)?;
      // Bare codestream, or a container if libjxl requires codestream level 10
      assert!(data.starts_with(&[0xff, 0x0a]) || data.starts_with(b"\0\0\0\x0cJXL "), "JPEG XL signature");
      let (image, decoded) = decode(&data);
      assert_eq!((image.width(), image.height()), (40, 30));
      let header = image.image_header();
      assert_eq!(header.metadata.bit_depth.bits_per_sample(), 16);
      assert!(!header.metadata.xyb_encoded);
      let decoded: Vec<u16> = decoded.iter().map(|v| (v * u16::MAX as f32).round() as u16).collect();
      assert_eq!(decoded, samples, "channels {}", channels);
    }
    Ok(())
  }

  #[test]
  fn lossless_half_float_roundtrip() -> Result<(), String> {
    if !libjxl_available_for_tests() {
      return Ok(());
    }
    // jxl-oxide 0.12 decodes half float zeros and subnormals wrong (2^-15 too high),
    // so the test only uses normal values. libjxl decodes them correctly.
    let floats: Vec<f32> = (1..=32 * 16 * 3).map(|i| i as f32 / 300.0).collect();
    let half: Vec<u16> = floats.iter().map(|v| half::f16::from_f32(*v).to_bits()).collect();
    let data = encode_tile(TileSamples::F16(&half), 32, 16, 3, &LOSSLESS)?;
    let (image, decoded) = decode(&data);
    assert!(matches!(
      image.image_header().metadata.bit_depth,
      jxl_oxide::image::BitDepth::FloatSample { bits_per_sample: 16, .. }
    ));
    let expected: Vec<f32> = half.iter().map(|v| half::f16::from_bits(*v).to_f32()).collect();
    assert_eq!(decoded, expected);
    Ok(())
  }

  #[test]
  fn lossy_roundtrip() -> Result<(), String> {
    if !libjxl_available_for_tests() {
      return Ok(());
    }
    let settings = TileSettings { distance: 1.0, ..LOSSLESS };
    // Smooth gradient, lossy compression must be close
    let samples: Vec<u16> = (0..64 * 64).map(|i| ((i % 64) * 1000 + (i / 64) * 20) as u16).collect();
    for transfer in [LossyTransfer::Srgb, LossyTransfer::Linear] {
      let settings = TileSettings {
        lossy_transfer: transfer,
        ..settings
      };
      let data = encode_tile(TileSamples::U16(&samples), 64, 64, 1, &settings)?;
      let (image, decoded) = decode(&data);
      assert!(image.image_header().metadata.xyb_encoded);
      for (d, s) in decoded.iter().zip(&samples) {
        assert!(
          (d * 65535.0 - *s as f32).abs() < 1500.0,
          "{:?}: decoded {}, expected {}",
          transfer,
          d * 65535.0,
          s
        );
      }
    }
    Ok(())
  }

  #[test]
  fn invalid_input_is_rejected() {
    let samples = vec![0_u16; 16];
    assert!(encode_tile(TileSamples::U16(&samples), 4, 4, 2, &LOSSLESS).is_err());
    assert!(encode_tile(TileSamples::U16(&samples), 4, 3, 1, &LOSSLESS).is_err());
  }
}
