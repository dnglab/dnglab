use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
extern crate glob;
use self::glob::glob;

extern crate toml;
mod basics;
mod tiff;
mod mrw;

pub static CAMERAS_TOML: &'static str = include_str!("../../data/cameras/all.toml");

pub trait Decoder {
  fn identify(&self) -> Result<&Camera, String>;
  fn image(&self) -> Image;
}

pub struct Image {
  pub width: u32,
  pub height: u32,
  pub wb_coeffs: [f32;4],
  pub data: Box<[u16]>,
}

#[derive(Debug)]
pub struct Camera {
  pub make: String,
  pub model: String,
  pub canonical_make: String,
  pub canonical_model: String,
}

impl Camera {
}

#[derive(Debug)]
pub struct RawLoader {
  pub cameras: HashMap<(String,String),Camera>,
}

impl RawLoader {
  pub fn new() -> RawLoader {
    let mut map = HashMap::new();

    let mut parser = toml::Parser::new(&CAMERAS_TOML);
    let toml = match parser.parse() {
      Some(val) => val,
      None => panic!(format!("Error parsing all.toml: {:?}", parser.errors)),
    };
    let cameras = toml.get("cameras").unwrap().as_table().unwrap();
    for (_,c) in cameras {
      let ct = c.as_table().unwrap();
      let make = ct.get("make").unwrap().as_str().unwrap().to_string();
      let model = ct.get("model").unwrap().as_str().unwrap().to_string();
      let cam = Camera{
        make: make.clone(),
        model: model.clone(),
        canonical_make: make.clone(),
        canonical_model: model.clone()
      };
      map.insert((make.clone(),model.clone()), cam);
    }

    RawLoader{
      cameras: map,
    }
  }

  pub fn get_decoder<'b>(&'b self, buffer: &'b [u8]) -> Result<Box<Decoder+'b>, String> {
    if mrw::is_mrw(buffer) {
      let dec = Box::new(mrw::MrwDecoder::new(buffer, &self));
      return Ok(dec as Box<Decoder>);
    }
    Err("Couldn't find a decoder!".to_string())
  }

  pub fn check_supported<'a>(&'a self, make: &'a str, model: &'a str) -> Result<&Camera, String> {
    match self.cameras.get(&(make.to_string(),model.to_string())) {
      Some(cam) => Ok(cam),
      None => Err(format!("Couldn't find camera \"{}\" \"{}\"", make, model)),
    }
  }
}
