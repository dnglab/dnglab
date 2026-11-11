// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! JPEG XL encoder for DNG tiles.
//!
//! The encoder uses the libjxl of the system, which is loaded at runtime. Nothing is
//! required at build time; if libjxl is missing or too old, encoding fails with an
//! error. Set the environment variable `RAWLER_LIBJXL` to load libjxl from a
//! specific path.

mod ffi;
mod jpegxl_lib;
mod types;

pub use types::*;
pub use jpegxl_lib::*;

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
