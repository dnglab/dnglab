use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
extern crate glob;
use self::glob::glob;

extern crate toml;
mod basics;
mod tiff;
mod mrw;

pub struct Camera<'a> {
  pub make: &'a str,
  pub model: &'a str,
}

pub trait Decoder {
  fn identify(&self) -> Result<Camera, String>;
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

impl CameraMetadata {
  pub fn from_toml(text: &str) -> CameraMetadata {
    let camvalue = toml::Parser::new(text).parse().unwrap();
    let cameradata = camvalue.get("camera").unwrap().as_table().unwrap();
    let make = cameradata.get("make").unwrap().as_str().unwrap().to_string();
    let model = cameradata.get("model").unwrap().as_str().unwrap().to_string();
    CameraMetadata{make: make, model: model}
  }
}

#[derive(Debug)]
pub struct RawLoader {
  pub cameras: HashMap<(String,String),CameraMetadata>,
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
          let cmd = CameraMetadata::from_toml(&toml);
          map.insert((cmd.make.clone(),cmd.model.clone()), cmd);
        }
        Err(e) => {}
      }
    }

    RawLoader{
      cameras: map,
    }
  }

  pub fn get_decoder<'b>(&'b self, buffer: &'b [u8]) -> Option<Box<Decoder+'b>> {
    if mrw::is_mrw(buffer) {
      let dec = Box::new(mrw::MrwDecoder::new(buffer, &self));
      return Some(dec as Box<Decoder>);
    }
    None
  }

  pub fn check_supported<'a>(&'a self, make: &'a str, model: &'a str) -> Result<Camera<'a>, String> {
    let cam_meta = match self.cameras.get(&(make.to_string(),model.to_string())) {
      Some(cam) => cam,
      None => return Err(format!("Couldn't find camera \"{}\" \"{}\"", make, model)),
    };

    Ok(Camera {
      make: make,
      model: model,
    })
  }
}
