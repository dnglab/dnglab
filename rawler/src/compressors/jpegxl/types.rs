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
