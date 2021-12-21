use crate::{decoders::*, packed::*};
use std::f32::NAN;

#[derive(Debug, Clone)]
pub struct NakedDecoder<'a> {
  buffer: &'a [u8],
  #[allow(dead_code)]
  rawloader: &'a RawLoader,
  camera: Camera,
}

impl<'a> NakedDecoder<'a> {
  pub fn new(buf: &'a [u8], cam: Camera, rawloader: &'a RawLoader) -> NakedDecoder<'a> {
    NakedDecoder {
      buffer: buf,
      camera: cam,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for NakedDecoder<'a> {
  fn raw_image(&self, _params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let width = self.camera.raw_width;
    let height = self.camera.raw_height;
    let size = self.camera.filesize;
    let bits = size*8 / width / height;

    let image = if self.camera.find_hint("12le_16bitaligned") {
      decode_12le_16bitaligned(self.buffer, width, height, dummy)
    } else {
      match bits {
        10 => decode_10le_lsb16(self.buffer, width, height, dummy),
        12 => decode_12be_msb16(self.buffer, width, height, dummy),
        _  => return Err(RawlerError::Unsupported(format!("Naked: Don't know about {} bps images", bits).to_string())),
      }
    };

    ok_image(self.camera.clone(), width, height, [NAN,NAN,NAN,NAN], image)
  }
}
