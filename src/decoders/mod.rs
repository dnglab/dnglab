use std::collections::HashMap;
use std::io::Read;
use std::fs::File;
use std::error::Error;
use std::panic;

macro_rules! fetch_tag {
  ($tiff:expr, $tag:expr) => (
    try!(
      $tiff.find_entry($tag).ok_or(
        format!("Couldn't find tag {}",stringify!($tag)).to_string()
      )
    );
  );
}

macro_rules! fetch_ifd {
  ($tiff:expr, $tag:expr) => (
    try!(
      $tiff.find_first_ifd($tag).ok_or(
        format!("Couldn't find ifd with tag {}",stringify!($tag)).to_string()
      )
    );
  );
}

extern crate toml;
mod image;
mod basics;
mod ljpeg;
mod cfa;
mod tiff;
mod ciff;
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
mod dcr;
mod dng;
mod pef;
mod crw;
mod nkd;
mod mos;
mod iiq;
mod tfr;
mod nef;
use self::tiff::*;
use self::ciff::*;
pub use self::image::*;

pub static CAMERAS_TOML: &'static str = include_str!("../../data/cameras/all.toml");

pub trait Decoder {
  fn image(&self) -> Result<Image, String>;
}

#[derive(Debug, Clone)]
pub struct Buffer {
  buf: Vec<u8>,
  size: usize,
}

impl Buffer {
  pub fn new(reader: &mut Read) -> Result<Buffer, String> {
    let mut buffer = Vec::new();
    if let Err(err) = reader.read_to_end(&mut buffer) {
      return Err(format!("IOError: {}", err).to_string())
    }
    let size = buffer.len();
    buffer.extend([0;16].iter().cloned());
    Ok(Buffer {
      buf: buffer,
      size: size,
    })
  }
}

#[derive(Debug, Clone)]
pub struct Camera {
  pub make: String,
  pub model: String,
  pub mode: String,
  pub clean_make: String,
  pub clean_model: String,
  pub filesize: usize,
  pub raw_width: usize,
  pub raw_height: usize,
  whitelevels: [u16;4],
  blacklevels: [u16;4],
  blackareah: (usize, usize),
  blackareav: (usize, usize),
  xyz_to_cam: [[f32;3];4],
  cfa: cfa::CFA,
  crops: [usize;4],
  bps: usize,
  wb_offset: usize,
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
        "clean_make" => {self.clean_make = val.as_str().unwrap().to_string().clone();},
        "clean_model" => {self.clean_model = val.as_str().unwrap().to_string().clone();},
        "whitepoint" => {let white = val.as_integer().unwrap() as u16; self.whitelevels = [white, white, white, white];},
        "blackpoint" => {let black = val.as_integer().unwrap() as u16; self.blacklevels = [black, black, black, black];},
        "blackareah" => {
          let vals = val.as_slice().unwrap();
          self.blackareah = (vals[0].as_integer().unwrap() as usize,
                             vals[1].as_integer().unwrap() as usize);
        },
        "blackareav" => {
          let vals = val.as_slice().unwrap();
          self.blackareav = (vals[0].as_integer().unwrap() as usize,
                             vals[1].as_integer().unwrap() as usize);
        },
        "color_matrix" => {
          let matrix = val.as_slice().unwrap();
          for (i, val) in matrix.into_iter().enumerate() {
            self.xyz_to_cam[i/3][i%3] = val.as_integer().unwrap() as f32;
          }
        },
        "crops" => {
          let crop_vals = val.as_slice().unwrap();
          for (i, val) in crop_vals.into_iter().enumerate() {
            self.crops[i] = val.as_integer().unwrap() as usize;
          }
        },
        "color_pattern" => {self.cfa = cfa::CFA::new(&val.as_str().unwrap().to_string());},
        "bps" => {self.bps = val.as_integer().unwrap() as usize;},
        "wb_offset" => {self.wb_offset = val.as_integer().unwrap() as usize;},
        "filesize" => {self.filesize = val.as_integer().unwrap() as usize;},
        "raw_width" => {self.raw_width = val.as_integer().unwrap() as usize;},
        "raw_height" => {self.raw_height = val.as_integer().unwrap() as usize;},
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
      clean_make: "".to_string(),
      clean_model: "".to_string(),
      filesize: 0,
      raw_width: 0,
      raw_height: 0,
      whitelevels: [0;4],
      blacklevels: [0;4],
      blackareah: (0,0),
      blackareav: (0,0),
      xyz_to_cam : [[0.0;3];4],
      cfa: cfa::CFA::new(""),
      crops: [0,0,0,0],
      bps: 0,
      wb_offset: 0,
      hints: Vec::new(),
    }
  }
}

pub fn ok_image(camera: &Camera, width: usize, height: usize, wb_coeffs: [f32;4], image: Vec<u16>) -> Result<Image,String> {
  Ok(Image::new(camera, width, height, wb_coeffs, image))
}

pub fn ok_image_with_blacklevels(camera: &Camera, width: usize, height: usize, wb_coeffs: [f32;4], blacks: [u16;4], image: Vec<u16>) -> Result<Image,String> {
  let mut img = Image::new(camera, width, height, wb_coeffs, image);
  img.blacklevels = blacks;
  Ok(img)
}

#[derive(Debug, Clone)]
pub struct RawLoader {
  pub cameras: HashMap<(String,String,String),Camera>,
  pub naked: HashMap<usize,Camera>,
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
    let mut naked = HashMap::new();
    for cam in cams {
      map.insert((cam.make.clone(),cam.model.clone(),cam.mode.clone()), cam.clone());
      if cam.filesize > 0 {
        naked.insert(cam.filesize, cam);
      }
    }

    RawLoader{
      cameras: map,
      naked: naked,
    }
  }

  pub fn get_decoder<'b>(&'b self, buf: &'b Buffer) -> Result<Box<Decoder+'b>, String> {
    let buffer = &buf.buf;

    if mrw::is_mrw(buffer) {
      let dec = Box::new(mrw::MrwDecoder::new(buffer, &self));
      return Ok(dec as Box<Decoder>);
    }

    if ciff::is_ciff(buffer) {
      let ciff = try!(CiffIFD::new_file(buf));
      let dec = Box::new(crw::CrwDecoder::new(buffer, ciff, &self));
      return Ok(dec as Box<Decoder>);
    }

    if let Ok(tiff) = TiffIFD::new_file(buffer) {
      if tiff.has_entry(Tag::DNGVersion) {
        return Ok(Box::new(dng::DngDecoder::new(buffer, tiff, self)))
      }

      if tiff.has_entry(Tag::Make) {
        macro_rules! use_decoder {
            ($dec:ty, $buf:ident, $tiff:ident, $rawdec:ident) => (Ok(Box::new(<$dec>::new($buf, $tiff, $rawdec)) as Box<Decoder>));
        }

        return match fetch_tag!(tiff, Tag::Make).get_str().to_string().as_ref() {
          "SONY"                        => use_decoder!(arw::ArwDecoder, buffer, tiff, self),
          "Mamiya-OP Co.,Ltd."          => use_decoder!(mef::MefDecoder, buffer, tiff, self),
          "OLYMPUS IMAGING CORP."       => use_decoder!(orf::OrfDecoder, buffer, tiff, self),
          "OLYMPUS CORPORATION"         => use_decoder!(orf::OrfDecoder, buffer, tiff, self),
          "OLYMPUS OPTICAL CO.,LTD"     => use_decoder!(orf::OrfDecoder, buffer, tiff, self),
          "SAMSUNG"                     => use_decoder!(srw::SrwDecoder, buffer, tiff, self),
          "SEIKO EPSON CORP."           => use_decoder!(erf::ErfDecoder, buffer, tiff, self),
          "EASTMAN KODAK COMPANY"       => use_decoder!(kdc::KdcDecoder, buffer, tiff, self),
          "KODAK"                       => use_decoder!(dcs::DcsDecoder, buffer, tiff, self),
          "Kodak"                       => use_decoder!(dcr::DcrDecoder, buffer, tiff, self),
          "Panasonic"                   => use_decoder!(rw2::Rw2Decoder, buffer, tiff, self),
          "LEICA"                       => use_decoder!(rw2::Rw2Decoder, buffer, tiff, self),
          "FUJIFILM"                    => use_decoder!(raf::RafDecoder, buffer, tiff, self),
          "PENTAX Corporation"          => use_decoder!(pef::PefDecoder, buffer, tiff, self),
          "RICOH IMAGING COMPANY, LTD." => use_decoder!(pef::PefDecoder, buffer, tiff, self),
          "PENTAX"                      => use_decoder!(pef::PefDecoder, buffer, tiff, self),
          "Leaf"                        => use_decoder!(iiq::IiqDecoder, buffer, tiff, self),
          "Hasselblad"                  => use_decoder!(tfr::TfrDecoder, buffer, tiff, self),
          "NIKON CORPORATION"           => use_decoder!(nef::NefDecoder, buffer, tiff, self),
          make => Err(format!("Couldn't find a decoder for make \"{}\"", make).to_string()),
        };
      } else if tiff.has_entry(Tag::Software) {
        // Last ditch effort to identify Leaf cameras without Make and Model
        if fetch_tag!(tiff, Tag::Software).get_str() == "Camera Library" {
          return Ok(Box::new(mos::MosDecoder::new(buffer, tiff, self)))
        }
      }
    }

    // If all else fails see if we match by filesize to one of those CHDK style files
    if let Some(cam) = self.naked.get(&buf.size) {
      return Ok(Box::new(nkd::NakedDecoder::new(buffer, cam, self)))
    }

    Err("Couldn't find a decoder for this file".to_string())
  }

  pub fn check_supported_with_everything<'a>(&'a self, make: &str, model: &str, mode: &str) -> Result<&Camera, String> {
    match self.cameras.get(&(make.to_string(),model.to_string(),mode.to_string())) {
      Some(cam) => Ok(cam),
      None => Err(format!("Couldn't find camera \"{}\" \"{}\" mode \"{}\"", make, model, mode)),
    }
  }

  pub fn check_supported_with_mode<'a>(&'a self, tiff: &'a TiffIFD, mode: &str) -> Result<&Camera, String> {
    let make = fetch_tag!(tiff, Tag::Make).get_str();
    let model = fetch_tag!(tiff, Tag::Model).get_str();

    self.check_supported_with_everything(make, model, mode)
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
