mod basics;
mod mrw;

pub trait Decoder {
  fn make(&self) -> String;
  fn model(&self) -> String;
  fn image(&self) -> Image;
}

pub struct Image {
  pub width: u32,
  pub height: u32,
  pub wb_coeffs: [f32;4],
  pub data: Box<[u16]>,
}

pub fn get_decoder<'a>(buffer: &'a [u8]) -> Option<Box<Decoder+'a>> {
  if mrw::is_mrw(buffer) {
    let dec = Box::new(mrw::MrwDecoder::new(buffer));
    return Some(dec as Box<Decoder>);
  }
  None
}
