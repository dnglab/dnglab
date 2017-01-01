use decoders::*;
use decoders::basics::*;
use std::f32::NAN;

#[derive(Debug, Clone)]
pub struct NakedDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  camera: &'a Camera,
}

impl<'a> NakedDecoder<'a> {
  pub fn new(buf: &'a [u8], cam: &'a Camera, rawloader: &'a RawLoader) -> NakedDecoder<'a> {
    NakedDecoder {
      buffer: buf,
      camera: cam,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for NakedDecoder<'a> {
  fn image(&self) -> Result<Image,String> {
    let width = self.camera.raw_width;
    let height = self.camera.raw_height;
    let size = self.camera.filesize;
    let bits = size*8 / width / height;

    let image = match bits {
      10 => decode_10le_lsb16(self.buffer, width, height),
      12 => decode_12be_msb16(self.buffer, width, height),
      _  => return Err(format!("Naked: Don't know about {} bps images", bits).to_string()),
    };

    ok_image(self.camera, width, height, [NAN,NAN,NAN,NAN], image)
  }
}
