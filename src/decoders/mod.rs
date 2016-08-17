mod mrw;

pub trait Decoder {
  fn make(&self) -> String;
  fn model(&self) -> String;
}

pub fn get_decoder<'a>(buffer: &'a [u8]) -> Box<Decoder+'a> {
  let dec = Box::new(mrw::MrwDecoder::new(buffer));
  dec as Box<Decoder>
}
