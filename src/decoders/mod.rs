use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
extern crate glob;
use self::glob::glob;

extern crate toml;
mod basics;
mod tiff;
mod mrw;

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
  pub fn from_toml(text: &str) -> Camera {
    let camvalue = toml::Parser::new(text).parse().unwrap();
    let cameradata = camvalue.get("camera").unwrap().as_table().unwrap();
    let make = cameradata.get("make").unwrap().as_str().unwrap().to_string();
    let model = cameradata.get("model").unwrap().as_str().unwrap().to_string();
    Camera{make: make.clone(), model: model.clone(), canonical_make: make.clone(), canonical_model: model.clone()}
  }
}

#[derive(Debug)]
pub struct RawLoader {
  pub cameras: HashMap<(String,String),Camera>,
}

impl RawLoader {
  pub fn new(path: &str) -> RawLoader {
    let mut map = HashMap::new();

    for entry in glob(&(path.to_string()+"/**/*.toml")).expect("Failed to read glob pattern") {
      match entry {
        Ok(path) => {
          let mut f = File::open(path).unwrap();
          let mut toml = String::new();
          f.read_to_string(&mut toml).unwrap();
          let cmd = Camera::from_toml(&toml);
          map.insert((cmd.make.clone(),cmd.model.clone()), cmd);
        }
        Err(err) => panic!(err),
      }
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
