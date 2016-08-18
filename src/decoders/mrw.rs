use decoders::Decoder;
use decoders::basics::*;

pub fn is_mrw(buf: &[u8]) -> bool {
  BEu32(buf,0) == 0x004D524D
}

pub struct MrwDecoder<'a> {
  buffer: &'a [u8],
  dataoff: u32,
}

impl<'a> MrwDecoder<'a> {
  pub fn new(buf: &[u8]) -> MrwDecoder {
    let off = BEu32(buf, 4) + 8;

    MrwDecoder { 
      buffer: buf,
      dataoff: off,
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
