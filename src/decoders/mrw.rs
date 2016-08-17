use decoders::Decoder;

pub struct MrwDecoder<'a> {
  buffer: &'a [u8],
}

impl<'a> MrwDecoder<'a> {
  pub fn new(buf: &[u8]) -> MrwDecoder {
    MrwDecoder { 
      buffer: buf,
    }
  }
}

impl<'a> Decoder for MrwDecoder<'a> {
  fn make(&self) -> String {
    "Minolta".to_string()
  }

  fn model(&self) -> String {
    "SomeModel".to_string()
  }
}
