use std::collections::HashMap;
extern crate toml;
mod basics;
mod tiff;
mod mrw;

pub struct Camera<'a> {
  pub make: &'a str,
  pub model: &'a str,
}

pub trait Decoder {
  fn identify(&self) -> Camera;
  fn image(&self) -> Image;
}

pub struct Image {
  pub width: u32,
  pub height: u32,
  pub wb_coeffs: [f32;4],
  pub data: Box<[u16]>,
}

#[derive(Debug)]
pub struct CameraMetadata {
  pub make: String,
  pub model: String,
}

#[derive(Debug)]
pub struct RawLoader {
  pub cameras: HashMap<(String,String),CameraMetadata>,
}

impl RawLoader {
  pub fn new() -> RawLoader{
    let mut map = HashMap::new();

    let toml = r#"
        [camera]
        make = "SomeMake"
        model = "SomeModel"
    "#;

    let camvalue = toml::Parser::new(toml).parse().unwrap();
    let cameradata = camvalue.get("camera").unwrap().as_table().unwrap();
    let make = cameradata.get("make").unwrap().as_str().unwrap().to_string();
    let model = cameradata.get("model").unwrap().as_str().unwrap().to_string();
    let cammeta = CameraMetadata{make: make.clone(), model: model.clone()};

    map.insert((make,model), cammeta);

    RawLoader{
      cameras: map,
    }
  }

  pub fn get_decoder<'b>(&'b self, buffer: &'b [u8]) -> Option<Box<Decoder+'b>> {
    if mrw::is_mrw(buffer) {
      let dec = Box::new(mrw::MrwDecoder::new(buffer));
      return Some(dec as Box<Decoder>);
    }
    None
  }
}
