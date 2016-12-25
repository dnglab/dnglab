use std::collections::HashMap;
use std::io::Read;
use std::fs::File;
use std::error::Error;
use std::panic;
use std::fmt;
use std::clone;

macro_rules! fetch_tag {
  ($tiff:expr, $tag:expr) => (try!($tiff.find_entry($tag).ok_or(format!("Couldn't find tag {}",stringify!($tag)).to_string())););
}

macro_rules! fetch_ifd {
  ($tiff:expr, $tag:expr) => (try!($tiff.find_first_ifd($tag).ok_or(format!("Couldn't find ifd with tag {}",stringify!($tag)).to_string())););
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
mod kdc;
mod dcs;
mod rw2;
mod raf;
use self::tiff::*;

pub static CAMERAS_TOML: &'static str = include_str!("../../data/cameras/all.toml");

pub trait Decoder {
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

pub struct CFA {
  patname: String,
  pattern: [[usize;48];48],
}

impl CFA {
  pub fn new(patname: &str) -> CFA {
    let filters = match patname {
      "BGGR" => 0x16161616,
      "GRBG" => 0x61616161,
      "GBRG" => 0x49494949,
      "RGGB" => 0x94949494,
      "ERBG" => 0x63636363,
      _ => 0,
    };

    let mut pattern: [[usize;48];48] = [[0;48];48];
    for row in 0..48 {
      for col in 0..48 {
        pattern[row][col] = filters >> (((row << 1 & 14) + (col & 1) ) << 1) & 3;
      }
    }
    CFA {
      patname: patname.to_string(),
      pattern: pattern,
    }
  }

  pub fn color_at(&self, row: usize, col: usize) -> usize {
    self.pattern[(row+48) % 48][(col+48) % 48]
  }
}

impl fmt::Debug for CFA {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    write!(f, "CFA {{ name:{} }}", self.patname)
  }
}

impl clone::Clone for CFA {
  fn clone(&self) -> CFA {
    let mut cpattern: [[usize;48];48] = [[0;48];48];
    for row in 0..48 {
      for col in 0..48 {
        cpattern[row][col] = self.pattern[row][col];
      }
    }
    CFA {
      patname: self.patname.clone(),
      pattern: cpattern,
    }
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
  pub cfa: CFA,
  pub crops: [i64;4],
}

#[derive(Debug, Clone)]
pub struct Camera {
  pub make: String,
  pub model: String,
  pub mode: String,
  pub canonical_make: String,
  pub canonical_model: String,
  whitelevels: [i64;4],
  blacklevels: [i64;4],
  color_matrix: [i64;12],
  cfa: CFA,
  crops: [i64;4],
  bps: u32,
  hints: Vec<String>,
}

impl Camera {
  pub fn find_hint(&self, hint: &str) -> bool {
    self.hints.contains(&(hint.to_string()))
  }

  pub fn update_from_toml(&mut self, ct: &toml::Table) {
    for (name, val) in ct {
      match name.as_ref() {
        "make" => {self.make = val.as_str().unwrap().to_string().clone();},
        "model" => {self.model = val.as_str().unwrap().to_string().clone();},
        "mode" => {self.mode = val.as_str().unwrap().to_string().clone();},
        "canonical_name" => {self.canonical_make = val.as_str().unwrap().to_string().clone();},
        "canonical_model" => {self.canonical_model = val.as_str().unwrap().to_string().clone();},
        "whitepoint" => {let white = val.as_integer().unwrap(); self.whitelevels = [white, white, white, white];},
        "blackpoint" => {let black = val.as_integer().unwrap(); self.blacklevels = [black, black, black, black];},
        "color_matrix" => {
          let matrix = val.as_slice().unwrap();
          for (i, val) in matrix.into_iter().enumerate() {
            self.color_matrix[i] = val.as_integer().unwrap();
          }
        },
        "crops" => {
          let crop_vals = val.as_slice().unwrap();
          for (i, val) in crop_vals.into_iter().enumerate() {
            self.crops[i] = val.as_integer().unwrap();
          }
        },
        "color_pattern" => {self.cfa = CFA::new(&val.as_str().unwrap().to_string());},
        "bps" => {self.bps = val.as_integer().unwrap() as u32;},
        "hints" => {
          self.hints = Vec::new();
          for hint in val.as_slice().unwrap() {
            self.hints.push(hint.as_str().unwrap().to_string());
          }
        },
        _ => {},
      }
    }
  }

  pub fn new() -> Camera {
    Camera {
      make: "".to_string(),
      model: "".to_string(),
      mode: "".to_string(),
      canonical_make: "".to_string(),
      canonical_model: "".to_string(),
      whitelevels: [0;4],
      blacklevels: [0;4],
      color_matrix : [0;12],
      cfa: CFA::new(""),
      crops: [0,0,0,0],
      bps: 0,
      hints: Vec::new(),
    }
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
    cfa: camera.cfa.clone(),
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
  pub cameras: HashMap<(String,String,String),Camera>,
}

impl RawLoader {
  pub fn new() -> RawLoader {
    let mut parser = toml::Parser::new(&CAMERAS_TOML);
    let toml = match parser.parse() {
      Some(val) => val,
      None => panic!(format!("Error parsing all.toml: {:?}", parser.errors)),
    };
    let mut cams = Vec::new();
    for camera in toml.get("cameras").unwrap().as_slice().unwrap() {
      let ct = camera.as_table().unwrap();
      let mut cam = Camera::new();
      cam.update_from_toml(ct);
      let basecam = cam.clone();
      cams.push(cam);

      match ct.get("modes") {
        Some(val) => {
          for mode in val.as_slice().unwrap() {
            let cmt = mode.as_table().unwrap();
            let mut mcam = basecam.clone();
            mcam.update_from_toml(cmt);
            cams.push(mcam);
          }
        },
        None => {},
      }
    }

    let mut map = HashMap::new();
    for cam in cams {
      map.insert((cam.make.clone(),cam.model.clone(),cam.mode.clone()), cam);
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

    let tiff = try!(TiffIFD::new_file(buffer));

    macro_rules! use_decoder {
        ($dec:ty, $buf:ident, $tiff:ident, $rawdec:ident) => (Ok(Box::new(<$dec>::new($buf, $tiff, $rawdec)) as Box<Decoder>));
    }

    match fetch_tag!(tiff, Tag::Make).get_str().to_string().as_ref() {
      "SONY"                    => use_decoder!(arw::ArwDecoder, buffer, tiff, self),
      "Mamiya-OP Co.,Ltd."      => use_decoder!(mef::MefDecoder, buffer, tiff, self),
      "OLYMPUS IMAGING CORP."   => use_decoder!(orf::OrfDecoder, buffer, tiff, self),
      "OLYMPUS CORPORATION"     => use_decoder!(orf::OrfDecoder, buffer, tiff, self),
      "OLYMPUS OPTICAL CO.,LTD" => use_decoder!(orf::OrfDecoder, buffer, tiff, self),
      "SAMSUNG"                 => use_decoder!(srw::SrwDecoder, buffer, tiff, self),
      "SEIKO EPSON CORP."       => use_decoder!(erf::ErfDecoder, buffer, tiff, self),
      "EASTMAN KODAK COMPANY"   => use_decoder!(kdc::KdcDecoder, buffer, tiff, self),
      "KODAK"                   => use_decoder!(dcs::DcsDecoder, buffer, tiff, self),
      "Panasonic"               => use_decoder!(rw2::Rw2Decoder, buffer, tiff, self),
      "FUJIFILM"                => use_decoder!(raf::RafDecoder, buffer, tiff, self),
      make => Err(format!("Couldn't find a decoder for make \"{}\"", make).to_string()),
    }
  }

  pub fn check_supported_with_mode<'a>(&'a self, tiff: &'a TiffIFD, mode: &str) -> Result<&Camera, String> {
    let make = fetch_tag!(tiff, Tag::Make).get_str();
    let model = fetch_tag!(tiff, Tag::Model).get_str();

    match self.cameras.get(&(make.to_string(),model.to_string(),mode.to_string())) {
      Some(cam) => Ok(cam),
      None => Err(format!("Couldn't find camera \"{}\" \"{}\" mode \"{}\"", make, model, mode)),
    }
  }

  pub fn check_supported<'a>(&'a self, tiff: &'a TiffIFD) -> Result<&Camera, String> {
    self.check_supported_with_mode(tiff, "")
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
