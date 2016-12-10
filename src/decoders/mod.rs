use std::collections::HashMap;
use std::io::Read;
use std::fs::File;
use std::error::Error;
use std::panic;

macro_rules! fetch_tag {
  ($tiff:expr, $tag:expr, $message:expr) => (try!($tiff.find_entry($tag).ok_or($message.to_string())););
}

extern crate toml;
mod basics;
mod tiff;
mod mrw;
mod arw;
mod mef;
mod orf;
mod srw;
mod erf;
use self::basics::*;
use self::tiff::*;

pub static CAMERAS_TOML: &'static str = include_str!("../../data/cameras/all.toml");

pub trait Decoder {
  fn identify(&self) -> Result<&Camera, String>;
  fn image(&self) -> Result<Image, String>;
}

#[derive(Debug, Clone)]
pub struct Buffer {
  buf: Vec<u8>,
}

impl Buffer {
  pub fn new(reader: &mut Read) -> Result<Buffer, String> {
    let mut buffer = Vec::new();
    if let Err(err) = reader.read_to_end(&mut buffer) {
      return Err(format!("IOError: {}", err).to_string())
    }
    buffer.extend([0;16].iter().cloned());
    Ok(Buffer {
      buf: buffer,
    })
  }
}

#[derive(Debug, Clone)]
pub struct Image {
  pub make: String,
  pub model: String,
  pub canonical_make: String,
  pub canonical_model: String,
  pub width: usize,
  pub height: usize,
  pub wb_coeffs: [f32;4],
  pub data: Box<[u16]>,
  pub whitelevels: [i64;4],
  pub blacklevels: [i64;4],
  pub color_matrix: [i64;12],
  pub dcraw_filters: u32,
  pub crops: [i64;4],
}

#[derive(Debug, Clone)]
pub struct Camera {
  pub make: String,
  pub model: String,
  pub canonical_make: String,
  pub canonical_model: String,
  whitelevels: [i64;4],
  blacklevels: [i64;4],
  color_matrix: [i64;12],
  dcraw_filters: u32,
  crops: [i64;4],
  bps: u32,
  hints: Vec<String>,
}

impl Camera {
  pub fn find_hint(&self, hint: &str) -> bool {
    self.hints.contains(&(hint.to_string()))
  }
}

pub fn ok_image(camera: &Camera, width: u32, height: u32, wb_coeffs: [f32;4], image: Vec<u16>) -> Result<Image,String> {
  Ok(Image {
    make: camera.make.clone(),
    model: camera.model.clone(),
    canonical_make: camera.canonical_make.clone(),
    canonical_model: camera.canonical_model.clone(),
    width: width as usize,
    height: height as usize,
    wb_coeffs: wb_coeffs,
    data: image.into_boxed_slice(),
    blacklevels: camera.blacklevels,
    whitelevels: camera.whitelevels,
    color_matrix: camera.color_matrix,
    dcraw_filters: camera.dcraw_filters,
    crops: camera.crops,
  })
}

//pub fn ok_image_with_levels(camera: &Camera, width: u32, height: u32, wb_coeffs: [f32;4], whites: [i64;4], blacks: [i64;4], image: Vec<u16>) -> Result<Image,String> {
//  Ok(Image {
//    make: camera.make.clone(),
//    model: camera.model.clone(),
//    canonical_make: camera.canonical_make.clone(),
//    canonical_model: camera.canonical_model.clone(),
//    width: width,
//    height: height,
//    wb_coeffs: wb_coeffs,
//    data: image.into_boxed_slice(),
//    blacklevels: blacks,
//    whitelevels: whites,
//    color_matrix: camera.color_matrix,
//    dcraw_filters: camera.dcraw_filters,
//    crops: camera.crops,
//  })
//}

#[derive(Debug, Clone)]
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
      let canonical_make = ct.get("canonical_make").unwrap().as_str().unwrap().to_string();
      let canonical_model = ct.get("canonical_model").unwrap().as_str().unwrap().to_string();
      let white = ct.get("whitepoint").unwrap().as_integer().unwrap();
      let black = ct.get("blackpoint").unwrap().as_integer().unwrap();
      let matrix = ct.get("color_matrix").unwrap().as_slice().unwrap();
      let mut cmatrix: [i64;12] = [0,0,0,0,0,0,0,0,0,0,0,0];
      for (i, val) in matrix.into_iter().enumerate() {
        cmatrix[i] = val.as_integer().unwrap();
      }
      let crop_vals = ct.get("crops").unwrap().as_slice().unwrap();
      let mut crops: [i64;4] = [0,0,0,0];
      for (i, val) in crop_vals.into_iter().enumerate() {
        crops[i] = val.as_integer().unwrap();
      }
      let color_pattern = ct.get("color_pattern").unwrap().as_str().unwrap().to_string();
      let bps: u32 = match ct.get("bps") {
        Some(x) => x.as_integer().unwrap() as u32,
        None => 0,
      };
      let mut hints: Vec<String> = Vec::new();
      match ct.get("hints") {
        None => {},
        Some(x) => {
          for val in x.as_slice().unwrap() {
            hints.push(val.as_str().unwrap().to_string());
          }
        },
      }
      let cam = Camera{
        make: make.clone(),
        model: model.clone(),
        canonical_make: canonical_make.clone(),
        canonical_model: canonical_model.clone(),
        whitelevels: [white, white, white, white],
        blacklevels: [black, black, black, black],
        color_matrix : cmatrix,
        dcraw_filters: RawLoader::dcraw_filters(&color_pattern),
        crops: crops,
        bps: bps,
        hints: hints,
      };
      map.insert((make.clone(),model.clone()), cam);
    }

    RawLoader{
      cameras: map,
    }
  }

  pub fn get_decoder<'b>(&'b self, buf: &'b Buffer) -> Result<Box<Decoder+'b>, String> {
    let buffer = &buf.buf;

    if mrw::is_mrw(buffer) {
      let dec = Box::new(mrw::MrwDecoder::new(buffer, &self));
      return Ok(dec as Box<Decoder>);
    }

    let endian = match LEu16(&buffer, 0) {
      0x4949 => LITTLE_ENDIAN,
      0x4d4d => BIG_ENDIAN,
      x => {return Err(format!("Couldn't find decoder for marker 0x{:x}", x).to_string())},
    };

    macro_rules! use_decoder {
        ($dec:ty, $buf:ident, $tiff:ident, $rawdec:ident) => (Ok(Box::new(<$dec>::new($buf, $tiff, $rawdec)) as Box<Decoder>));
    }

    let tiff = TiffIFD::new_root(buffer, 4, 0, endian);
    let make: &str = &(try!(tiff.find_entry(Tag::Make).ok_or("Couldn't find Make".to_string())).get_str().to_string());

    match make {
      "SONY"                    => use_decoder!(arw::ArwDecoder, buffer, tiff, self),
      "Mamiya-OP Co.,Ltd."      => use_decoder!(mef::MefDecoder, buffer, tiff, self),
      "OLYMPUS IMAGING CORP."   => use_decoder!(orf::OrfDecoder, buffer, tiff, self),
      "OLYMPUS CORPORATION"     => use_decoder!(orf::OrfDecoder, buffer, tiff, self),
      "OLYMPUS OPTICAL CO.,LTD" => use_decoder!(orf::OrfDecoder, buffer, tiff, self),
      "SAMSUNG"                 => use_decoder!(srw::SrwDecoder, buffer, tiff, self),
      "SEIKO EPSON CORP."       => use_decoder!(erf::ErfDecoder, buffer, tiff, self),
      make => Err(format!("Couldn't find a decoder for make \"{}\"", make).to_string()),
    }
  }

  pub fn check_supported<'a>(&'a self, make: &'a str, model: &'a str) -> Result<&Camera, String> {
    match self.cameras.get(&(make.to_string(),model.to_string())) {
      Some(cam) => Ok(cam),
      None => Err(format!("Couldn't find camera \"{}\" \"{}\"", make, model)),
    }
  }

  fn dcraw_filters(pattern: &str) -> u32 {
    match pattern {
      "BGGR" => 0x16161616,
      "GRBG" => 0x61616161,
      "GBRG" => 0x49494949,
      "RGGB" => 0x94949494,
      "ERBG" => 0x63636363,
      _ => 0,
    }
  }

  pub fn decode(&self, reader: &mut Read) -> Result<Image, String> {
    let buffer = try!(Buffer::new(reader));
    let decoder = try!(self.get_decoder(&buffer));
    decoder.image()
  }

  pub fn decode_safe(&self, path: &str) -> Result<Image, String> {
    match panic::catch_unwind(|| {
      let mut f = match File::open(path) {
        Ok(val) => val,
        Err(e) => {return Err(e.description().to_string())},
      };
      self.decode(&mut f)
    }) {
      Ok(val) => val,
      Err(_) => Err("Caught a panic while decoding, please file a bug and attach a sample file".to_string()),
    }
  }
}
