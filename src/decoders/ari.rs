use std::f32::NAN;

use crate::decoders::*;
use crate::decoders::basics::*;

pub fn is_ari(buf: &[u8]) -> bool {
  buf[0..4] == b"ARRI"[..]
}

#[derive(Debug, Clone)]
pub struct AriDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
}

impl<'a> AriDecoder<'a> {
  pub fn new(buf: &'a [u8], rawloader: &'a RawLoader) -> AriDecoder<'a> {
    AriDecoder {
      buffer: buf,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for AriDecoder<'a> {
  fn image(&self, dummy: bool) -> Result<RawImage,String> {
    let offset = LEu32(self.buffer, 8) as usize;
    let width = LEu32(self.buffer, 20) as usize;
    let height = LEu32(self.buffer, 24) as usize;
    let model = String::from_utf8_lossy(&self.buffer[668..]).split_terminator("\0").next().unwrap_or("").to_string();
    let camera = self.rawloader.check_supported_with_everything("ARRI", &model, "")?;
    let src = &self.buffer[offset..];

    let image = decode_12be_msb32(src, width, height, dummy);

    ok_image(camera, width, height, self.get_wb()?, image)
  }
}

impl<'a> AriDecoder<'a> {
  fn get_wb(&self) -> Result<[f32;4], String> {
    Ok([LEf32(self.buffer, 100), LEf32(self.buffer, 104), LEf32(self.buffer, 108), NAN])
  }
}
