use crate::analyze::FormatDump;
use crate::formats::tiff::reader::TiffReader;
use crate::formats::tiff::GenericTiffReader;
use crate::formats::tiff::IFD;
use crate::pixarray::PixU16;
use crate::RawFile;
use crate::Result;
use image::DynamicImage;
use log::debug;
use log::warn;
use rayon::iter::IndexedParallelIterator;
use rayon::iter::ParallelIterator;
use rayon::slice::ParallelSliceMut;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::hash::Hash;
use std::io::BufReader;
use std::io::SeekFrom;
use std::panic;
use std::panic::AssertUnwindSafe;
use std::path::Path;
use toml::Value;

macro_rules! fetch_tag {
  ($tiff:expr, $tag:expr) => {
    $tiff.find_entry($tag).ok_or(format!("Couldn't find tag {}", stringify!($tag)).to_string())?
  };
}

macro_rules! fetch_tag_new {
  ($ifd:expr, $tag:expr) => {
    $ifd
      .get_entry($tag)
      .map(|entry| &entry.value)
      .ok_or(format!("Couldn't find tag {}", stringify!($tag)))?
    //$tiff.find_entry($tag).ok_or(format!("Couldn't find tag {}", stringify!($tag)).to_string())?
  };
}

/*
macro_rules! fetch_ifd {
  ($tiff:expr, $tag:expr) => {
    $tiff
      .find_first_ifd($tag)
      .ok_or(format!("Couldn't find ifd with tag {}", stringify!($tag)).to_string())?
  };
}
 */

//mod ari;
//mod arw;
pub mod cr2;
pub mod cr3;
pub mod iiq;
pub mod pef;
/*
mod crw;
mod dcr;
mod dcs;
mod dng;
mod erf;

mod kdc;
mod mef;
mod mos;
mod mrw;
mod nef;
mod nkd;
mod nrw;
mod orf;
mod raf;
mod rw2;
mod srw;
mod tfr;
mod x3f;
mod unwrapped;
*/

mod camera;

pub use camera::Camera;

use crate::analyze::CaptureInfo;
use crate::formats::tiff_legacy::Rational;
use crate::formats::tiff_legacy::SRational;
use crate::RawlerError;
use crate::{formats::bmff::Bmff, formats::tiff::DirectoryWriter};

use crate::alloc_image;
use crate::formats::tiff_legacy::LegacyTiffIFD;
use crate::formats::tiff_legacy::LegacyTiffTag;
use crate::tags::ExifTag;
use crate::tags::LegacyTiffRootTag;

//use self::tiff::*;
pub use super::rawimage::*;

pub static CAMERAS_TOML: &'static str = include_str!(concat!(env!("OUT_DIR"), "/cameras.toml"));
pub static SAMPLE: &'static str = "\nPlease submit samples at https://raw.pixls.us/";
pub static BUG: &'static str = "\nPlease file a bug with a sample file at https://github.com/pedrocr/rawloader/issues/new";

pub trait Readable: std::io::Read + std::io::Seek {}

pub type ReadableBoxed = Box<dyn Readable>;

#[derive(Default, Clone, Debug)]
pub struct RawDecodeParams {
  pub image_index: usize,
}

pub trait Decoder: Send {
  fn raw_image(&mut self, file: &mut RawFile, params: RawDecodeParams, dummy: bool) -> Result<RawImage>;

  fn raw_image_count(&self) -> Result<usize> {
    Ok(1)
  }

  fn thumbnail_image(&self, _file: &mut RawFile) -> Result<Option<DynamicImage>> {
    warn!("Decoder has no thumbnail image support, fallback to preview image");
    Ok(None)
  }

  fn preview_image(&self, _file: &mut RawFile) -> Result<Option<DynamicImage>> {
    warn!("Decoder has no preview image support");
    Ok(None)
  }

  fn full_image(&self, _file: &mut RawFile) -> Result<Option<DynamicImage>> {
    warn!("Decoder has no full image support");
    Ok(None)
  }

  fn xpacket(&self, _file: &mut RawFile) -> Option<&Vec<u8>> {
    None
  }

  fn gps_metadata(&self, _file: &mut RawFile) -> Option<&Gps> {
    None
  }

  fn format_dump(&self) -> FormatDump;

  fn populate_capture_info(&mut self, _capture_info: &mut CaptureInfo) -> Result<()> {
    Ok(())
  }

  fn populate_dng_root(&mut self, _: &mut DirectoryWriter) -> Result<()> {
    Ok(())
  }

  fn populate_dng_exif(&mut self, _: &mut DirectoryWriter) -> Result<()> {
    Ok(())
  }

  fn decode_metadata(&mut self, _file: &mut RawFile) -> Result<()> {
    Ok(())
  }
}

/// Possible orientations of an image
///
/// Values are taken from the IFD tag Orientation (0x0112) in most cases but they can be
/// obtained from other metadata in the file.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[allow(missing_docs)]
pub enum Orientation {
  Normal,
  HorizontalFlip,
  Rotate180,
  VerticalFlip,
  Transpose,
  Rotate90,
  Transverse,
  Rotate270,
  Unknown,
}

impl Orientation {
  /// Convert a u16 from the IFD tag Orientation (0x0112) into its corresponding
  /// enum value
  pub fn from_u16(orientation: u16) -> Orientation {
    match orientation {
      1 => Orientation::Normal,
      2 => Orientation::HorizontalFlip,
      3 => Orientation::Rotate180,
      4 => Orientation::VerticalFlip,
      5 => Orientation::Transpose,
      6 => Orientation::Rotate90,
      7 => Orientation::Transverse,
      8 => Orientation::Rotate270,
      _ => Orientation::Unknown,
    }
  }

  /// Extract orienation from a TiffIFD. If the given TiffIFD has an invalid
  /// value or contains no orientation data `Orientation::Unknown` is returned
  fn _from_tiff(tiff: &LegacyTiffIFD) -> Orientation {
    match tiff.find_entry(LegacyTiffRootTag::Orientation) {
      Some(entry) => Orientation::from_u16(entry.get_usize(0) as u16),
      None => Orientation::Unknown,
    }
  }

  /// Convert orientation to an image flip operation tuple. The first field is
  /// if x and y coordinates should be swapped (transposed). The second and
  /// third field is horizontal and vertical flipping respectively. For a
  /// correct result, flipping must be done before transposing.
  pub fn to_flips(&self) -> (bool, bool, bool) {
    match *self {
      Orientation::Normal | Orientation::Unknown => (false, false, false),
      Orientation::VerticalFlip => (false, false, true),
      Orientation::HorizontalFlip => (false, true, false),
      Orientation::Rotate180 => (false, true, true),
      Orientation::Transpose => (true, false, false),
      Orientation::Rotate90 => (true, false, true),
      Orientation::Rotate270 => (true, true, false),
      Orientation::Transverse => (true, true, true),
    }
  }

  /// Does the opposite of to_flips()
  pub fn from_flips(flips: (bool, bool, bool)) -> Self {
    match flips {
      (false, false, false) => Orientation::Normal,
      (false, false, true) => Orientation::VerticalFlip,
      (false, true, false) => Orientation::HorizontalFlip,
      (false, true, true) => Orientation::Rotate180,
      (true, false, false) => Orientation::Transpose,
      (true, false, true) => Orientation::Rotate90,
      (true, true, false) => Orientation::Rotate270,
      (true, true, true) => Orientation::Transverse,
    }
  }

  /// Convert orientation to the Tiff Orientation value
  pub fn to_u16(&self) -> u16 {
    match *self {
      Orientation::Unknown => 0,
      Orientation::Normal => 1,
      Orientation::HorizontalFlip => 2,
      Orientation::Rotate180 => 3,
      Orientation::VerticalFlip => 4,
      Orientation::Transpose => 5,
      Orientation::Rotate90 => 6,
      Orientation::Transverse => 7,
      Orientation::Rotate270 => 8,
    }
  }
}

pub fn ok_image(camera: Camera, width: usize, height: usize, cpp: usize, wb_coeffs: [f32; 4], image: Vec<u16>) -> Result<RawImage> {
  Ok(RawImage::new(camera, width, height, cpp, wb_coeffs, image, false))
}

pub fn ok_image_with_blacklevels(
  camera: Camera,
  width: usize,
  height: usize,
  cpp: usize,
  wb_coeffs: [f32; 4],
  blacks: [u16; 4],
  image: Vec<u16>,
) -> Result<RawImage> {
  let mut img = RawImage::new(camera, width, height, cpp, wb_coeffs, image, false);
  img.blacklevels = blacks;
  Ok(img)
}

pub fn ok_image_with_black_white(
  camera: Camera,
  width: usize,
  height: usize,
  cpp: usize,
  wb_coeffs: [f32; 4],
  black: u16,
  white: u16,
  image: Vec<u16>,
) -> Result<RawImage> {
  let mut img = RawImage::new(camera, width, height, cpp, wb_coeffs, image, false);
  img.blacklevels = [black, black, black, black];
  img.whitelevels = [white, white, white, white];
  Ok(img)
}

/// The struct that holds all the info about the cameras and is able to decode a file
#[derive(Debug, Clone)]
pub struct RawLoader {
  cameras: HashMap<(String, String, String), Camera>,
  #[allow(dead_code)] // TODO: remove once naked cams supported again
  naked: HashMap<usize, Camera>,
}

impl RawLoader {
  /// Creates a new raw loader using the camera information included in the library
  pub fn new() -> RawLoader {
    let toml = match CAMERAS_TOML.parse::<Value>() {
      Ok(val) => val,
      Err(e) => panic!("{}", format!("Error parsing cameras.toml: {:?}", e)),
    };

    let mut cams = Vec::new();
    for camera in toml.get("cameras").unwrap().as_array().unwrap() {
      // Create a list of all the camera modes including the base one
      let mut cammodes = Vec::new();
      let ct = camera.as_table().unwrap();
      cammodes.push(ct);
      if let Some(val) = ct.get("modes") {
        for mode in val.as_array().unwrap() {
          cammodes.push(mode.as_table().unwrap());
        }
      }

      // Start with the basic camera
      let mut cam = Camera::new();
      cam.update_from_toml(cammodes[0]);
      // Create a list of alias names including the base one
      let mut camnames = Vec::new();
      camnames.push((cam.model.clone(), cam.clean_model.clone()));
      if let Some(val) = ct.get("model_aliases") {
        for alias in val.as_array().unwrap() {
          camnames.push((alias[0].as_str().unwrap().to_string().clone(), alias[1].as_str().unwrap().to_string().clone()));
        }
      }

      // For each combination of alias and mode (including the base ones) create Camera
      for (model, clean_model) in camnames {
        for ct in cammodes.clone() {
          let mut mcam = cam.clone();
          mcam.update_from_toml(ct);
          mcam.model = model.clone();
          mcam.clean_model = clean_model.clone();
          cams.push(mcam);
        }
      }
    }

    let mut map = HashMap::new();
    let mut naked = HashMap::new();
    for cam in cams {
      map.insert((cam.make.clone(), cam.model.clone(), cam.mode.clone()), cam.clone());
      if cam.filesize > 0 {
        naked.insert(cam.filesize, cam);
      }
    }

    RawLoader { cameras: map, naked: naked }
  }

  /// Get list of cameras
  pub fn get_cameras(&self) -> &HashMap<(String, String, String), Camera> {
    &self.cameras
  }

  /// Returns a decoder for a given buffer
  pub fn get_decoder<'b>(&'b self, rawfile: &mut RawFile) -> Result<Box<dyn Decoder + 'b>> {
    macro_rules! reset_file {
      ($file:ident) => {
        $file
          .inner()
          .seek(SeekFrom::Start(0))
          .map_err(|e| RawlerError::General(format!("I/O error while trying decoders: {:?}", e)))?
      };
    }

    //let buffer = rawfile.get_buf().unwrap();

    /*
    if mrw::is_mrw(buffer) {
      let dec = Box::new(mrw::MrwDecoder::new(buffer, &self));
      return Ok(dec as Box<dyn Decoder>);
    }

    if ciff::is_ciff(buffer) {
      let ciff = ciff::CiffIFD::new_file(buf)?;
      let dec = Box::new(crw::CrwDecoder::new(buffer, ciff, &self));
      return Ok(dec as Box<dyn Decoder>);
    }

    if ari::is_ari(buffer) {
      let dec = Box::new(ari::AriDecoder::new(buffer, &self));
      return Ok(dec as Box<dyn Decoder>);
    }

    if x3f::is_x3f(buffer) {
      let dec = Box::new(x3f::X3fDecoder::new(buf, &self));
      return Ok(dec as Box<dyn Decoder>);
    }
    */

    reset_file!(rawfile);
    match Bmff::new(rawfile.inner()) {
      Ok(bmff) => {
        if bmff.compatible_brand("crx ") {
          return Ok(Box::new(cr3::Cr3Decoder::new(rawfile, bmff, self)?));
        }
      }
      Err(e) => {
        debug!("It's not a BMFF file: {:?}", e);
      }
    }

    reset_file!(rawfile);
    if let Ok(tiff) = GenericTiffReader::new(rawfile.inner(), 0, 0, None, &[]) {
      //if tiff.has_entry(LegacyTiffRootTag::DNGVersion) {
      //  return Ok(Box::new(dng::DngDecoder::new(buffer, tiff, self)));
      //}

      macro_rules! use_decoder {
        ($dec:ty, $buf:ident, $tiff:ident, $rawdec:ident) => {
          Ok(Box::new(<$dec>::new(rawfile, $tiff, $rawdec)?) as Box<dyn Decoder>)
        };
      }

      match tiff
        .get_entry(LegacyTiffRootTag::Make)
        .and_then(|entry| entry.value.as_string().and_then(|s| Some(s.as_str().trim_end())))
      {
        Some("Canon") => return use_decoder!(cr2::Cr2Decoder, rawfile, tiff, self),
        Some("PENTAX") => return use_decoder!(pef::PefDecoder, rawfile, tiff, self),
        Some("PENTAX Corporation") => return use_decoder!(pef::PefDecoder, rawfile, tiff, self),
        Some("RICOH IMAGING COMPANY, LTD.") => return use_decoder!(pef::PefDecoder, rawfile, tiff, self),
        Some("Phase One") => return use_decoder!(iiq::IiqDecoder, rawfile, tiff, self),
        Some("Phase One A/S") => return use_decoder!(iiq::IiqDecoder, rawfile, tiff, self),
        Some("Leaf") => return use_decoder!(iiq::IiqDecoder, rawfile, tiff, self),

        Some(make) => {
          return Err(RawlerError::Unsupported(
            format!("Couldn't find a decoder for make \"{}\".{}", make, SAMPLE).to_string(),
          ))
        }

        None => {} /*
                   "SONY" => use_decoder!(arw::ArwDecoder, buffer, tiff, self),
                   "Mamiya-OP Co.,Ltd." => use_decoder!(mef::MefDecoder, buffer, tiff, self),
                   "OLYMPUS IMAGING CORP." => use_decoder!(orf::OrfDecoder, buffer, tiff, self),
                   "OLYMPUS CORPORATION" => use_decoder!(orf::OrfDecoder, buffer, tiff, self),
                   "OLYMPUS OPTICAL CO.,LTD" => use_decoder!(orf::OrfDecoder, buffer, tiff, self),
                   "SAMSUNG" => use_decoder!(srw::SrwDecoder, buffer, tiff, self),
                   "SEIKO EPSON CORP." => use_decoder!(erf::ErfDecoder, buffer, tiff, self),
                   "EASTMAN KODAK COMPANY" => use_decoder!(kdc::KdcDecoder, buffer, tiff, self),
                   "Eastman Kodak Company" => use_decoder!(kdc::KdcDecoder, buffer, tiff, self),
                   "KODAK" => use_decoder!(dcs::DcsDecoder, buffer, tiff, self),
                   "Kodak" => use_decoder!(dcr::DcrDecoder, buffer, tiff, self),
                   "Panasonic" => use_decoder!(rw2::Rw2Decoder, buffer, tiff, self),
                   "LEICA" => use_decoder!(rw2::Rw2Decoder, buffer, tiff, self),
                   "FUJIFILM" => use_decoder!(raf::RafDecoder, buffer, tiff, self),
                   "PENTAX Corporation" => use_decoder!(pef::PefDecoder, buffer, tiff, self),
                   "RICOH IMAGING COMPANY, LTD." => use_decoder!(pef::PefDecoder, buffer, tiff, self),
                   "PENTAX" => use_decoder!(pef::PefDecoder, buffer, tiff, self),
                   //"Leaf" => use_decoder!(iiq::IiqDecoder, buffer, tiff, self),
                   "Hasselblad" => use_decoder!(tfr::TfrDecoder, buffer, tiff, self),
                   "NIKON CORPORATION" => use_decoder!(nef::NefDecoder, buffer, tiff, self),
                   "NIKON" => use_decoder!(nrw::NrwDecoder, buffer, tiff, self),
                   */
                   //"Canon" => use_decoder!(cr2::Cr2Decoder, buffer, tiff, self),
                   /*
                   //"Phase One A/S" => use_decoder!(iiq::IiqDecoder, buffer, tiff, self),
                   */
      };

      if tiff.has_entry(LegacyTiffRootTag::Software) {
        // Last ditch effort to identify Leaf cameras without Make and Model
        //if fetch_tag!(tiff, LegacyTiffRootTag::Software).get_str() == "Camera Library" {
        //return Ok(Box::new(mos::MosDecoder::new(buffer, tiff, self)));
        //}
      }
    }

    /*
    if let Ok(tiff) = LegacyTiffIFD::new_file(buffer, &vec![]) {
      if tiff.has_entry(LegacyTiffRootTag::DNGVersion) {
        return Ok(Box::new(dng::DngDecoder::new(buffer, tiff, self)));
      }

      // The DCS560C is really a CR2 camera so we just special case it here
      if tiff.has_entry(LegacyTiffRootTag::Model) && fetch_tag!(tiff, LegacyTiffRootTag::Model).get_str() == "DCS560C" {
        //return Ok(Box::new(cr2::Cr2Decoder::new(buffer, tiff, self)));
      }

      if tiff.has_entry(LegacyTiffRootTag::Make) {
        macro_rules! use_decoder {
          ($dec:ty, $buf:ident, $tiff:ident, $rawdec:ident) => {
            Ok(Box::new(<$dec>::new($buf, $tiff, $rawdec)) as Box<dyn Decoder>)
          };
        }

        return match fetch_tag!(tiff, LegacyTiffRootTag::Make).get_str().to_string().as_ref() {
          "SONY" => use_decoder!(arw::ArwDecoder, buffer, tiff, self),
          "Mamiya-OP Co.,Ltd." => use_decoder!(mef::MefDecoder, buffer, tiff, self),
          "OLYMPUS IMAGING CORP." => use_decoder!(orf::OrfDecoder, buffer, tiff, self),
          "OLYMPUS CORPORATION" => use_decoder!(orf::OrfDecoder, buffer, tiff, self),
          "OLYMPUS OPTICAL CO.,LTD" => use_decoder!(orf::OrfDecoder, buffer, tiff, self),
          "SAMSUNG" => use_decoder!(srw::SrwDecoder, buffer, tiff, self),
          "SEIKO EPSON CORP." => use_decoder!(erf::ErfDecoder, buffer, tiff, self),
          "EASTMAN KODAK COMPANY" => use_decoder!(kdc::KdcDecoder, buffer, tiff, self),
          "Eastman Kodak Company" => use_decoder!(kdc::KdcDecoder, buffer, tiff, self),
          "KODAK" => use_decoder!(dcs::DcsDecoder, buffer, tiff, self),
          "Kodak" => use_decoder!(dcr::DcrDecoder, buffer, tiff, self),
          "Panasonic" => use_decoder!(rw2::Rw2Decoder, buffer, tiff, self),
          "LEICA" => use_decoder!(rw2::Rw2Decoder, buffer, tiff, self),
          "FUJIFILM" => use_decoder!(raf::RafDecoder, buffer, tiff, self),
          "PENTAX Corporation" => use_decoder!(pef::PefDecoder, buffer, tiff, self),
          "RICOH IMAGING COMPANY, LTD." => use_decoder!(pef::PefDecoder, buffer, tiff, self),
          "PENTAX" => use_decoder!(pef::PefDecoder, buffer, tiff, self),
          "Leaf" => use_decoder!(iiq::IiqDecoder, buffer, tiff, self),
          "Hasselblad" => use_decoder!(tfr::TfrDecoder, buffer, tiff, self),
          "NIKON CORPORATION" => use_decoder!(nef::NefDecoder, buffer, tiff, self),
          "NIKON" => use_decoder!(nrw::NrwDecoder, buffer, tiff, self),
          //"Canon" => use_decoder!(cr2::Cr2Decoder, buffer, tiff, self),
          "Phase One A/S" => use_decoder!(iiq::IiqDecoder, buffer, tiff, self),
          make => Err(RawlerError::Unsupported(
            format!("Couldn't find a decoder for make \"{}\".{}", make, SAMPLE).to_string(),
          )),
        };
      } else if tiff.has_entry(LegacyTiffRootTag::Software) {
        // Last ditch effort to identify Leaf cameras without Make and Model
        if fetch_tag!(tiff, LegacyTiffRootTag::Software).get_str() == "Camera Library" {
          return Ok(Box::new(mos::MosDecoder::new(buffer, tiff, self)));
        }
      }
    }

    // If all else fails see if we match by filesize to one of those CHDK style files
    if let Some(cam) = self.naked.get(&buf.size) {
      return Ok(Box::new(nkd::NakedDecoder::new(buffer, cam.clone(), self)));
    }
    */

    Err(RawlerError::Unsupported(
      format!("Couldn't find a decoder for this file.{}", SAMPLE).to_string(),
    ))
  }

  fn check_supported_with_everything<'a>(&'a self, make: &str, model: &str, mode: &str) -> Result<Camera> {
    match self.cameras.get(&(make.to_string(), model.to_string(), mode.to_string())) {
      Some(cam) => Ok(cam.clone()),
      None => Err(RawlerError::Unsupported(format!(
        "Couldn't find camera \"{}\" \"{}\" mode \"{}\".{}",
        make, model, mode, SAMPLE
      ))),
    }
  }

  fn _check_supported_with_mode_old<'a>(&'a self, tiff: &'a LegacyTiffIFD, mode: &str) -> Result<Camera> {
    let make = fetch_tag!(tiff, LegacyTiffRootTag::Make).get_str();
    let model = fetch_tag!(tiff, LegacyTiffRootTag::Model).get_str();

    // Get a default instance to modify
    let camera = self.check_supported_with_everything(make, model, mode)?;

    // Lookup the orientation of the image for later image rotation
    //camera.orientation = Orientation::from_tiff(tiff);

    Ok(camera)
  }

  fn _check_supported_old<'a>(&'a self, tiff: &'a LegacyTiffIFD) -> Result<Camera> {
    // TODO remove me
    self._check_supported_with_mode_old(tiff, "")
  }

  fn check_supported_with_mode(&self, ifd0: &IFD, mode: &str) -> Result<Camera> {
    let make = fetch_tag_new!(ifd0, LegacyTiffRootTag::Make).get_string()?.trim_end();
    let model = fetch_tag_new!(ifd0, LegacyTiffRootTag::Model).get_string()?.trim_end();
    self.check_supported_with_everything(make, model, mode)
  }

  #[allow(dead_code)]
  fn check_supported(&self, ifd0: &IFD) -> Result<Camera> {
    self.check_supported_with_mode(ifd0, "")
  }

  fn decode_unsafe<'b>(&'b self, rawfile: &mut RawFile, params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let mut decoder = self.get_decoder(rawfile)?;
    decoder.raw_image(rawfile, params, dummy)
  }

  /// Decodes an input into a RawImage
  pub fn decode<'b>(&'b self, rawfile: &mut RawFile, params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    //let buffer = Buffer::new(reader)?;

    match panic::catch_unwind(AssertUnwindSafe(|| self.decode_unsafe(rawfile, params, dummy))) {
      Ok(val) => val,
      Err(_) => Err(RawlerError::General(format!("Caught a panic while decoding.{}", BUG))),
    }
  }

  /// Decodes a file into a RawImage
  pub fn decode_file<'b>(&'b self, path: &Path) -> Result<RawImage> {
    let file = match File::open(path) {
      Ok(val) => val,
      Err(e) => return Err(RawlerError::with_io_error("decode_file()", path, e)),
    };
    let mut buffered_file = RawFile::from(BufReader::new(file));
    self.decode(&mut buffered_file, RawDecodeParams::default(), false)
  }

  /// Decodes a file into a RawImage
  pub fn raw_image_count_file<'b>(&'b self, path: &Path) -> Result<usize> {
    let file = match File::open(path) {
      Ok(val) => val,
      Err(e) => return Err(RawlerError::with_io_error("raw_image_count_file()", path, e)),
    };
    let buffered_file = BufReader::new(file);
    //let buffer = Buffer::new(&mut buffered_file)?;
    let decoder = self.get_decoder(&mut buffered_file.into())?;
    decoder.raw_image_count()
  }

  // Decodes an unwrapped input (just the image data with minimal metadata) into a RawImage
  // This is only useful for fuzzing really
  #[doc(hidden)]
  pub fn decode_unwrapped(&self, _rawfile: &mut RawFile) -> Result<RawImageData> {
    /*
    let buffer = Buffer::new(reader)?;

    match panic::catch_unwind(|| unwrapped::decode_unwrapped(&buffer)) {
      Ok(val) => val,
      Err(_) => Err(RawlerError::General(format!("Caught a panic while decoding.{}", BUG).to_string())),
    }
    */
    unimplemented!()
  }
}

pub fn decode_unthreaded<F>(width: usize, height: usize, dummy: bool, closure: &F) -> PixU16
where
  F: Fn(&mut [u16], usize) + Sync,
{
  let mut out: Vec<u16> = alloc_image!(width, height, dummy);
  out.chunks_mut(width).enumerate().for_each(|(row, line)| {
    closure(line, row);
  });
  PixU16::new(out, width, height)
}

pub fn decode_threaded<F>(width: usize, height: usize, dummy: bool, closure: &F) -> PixU16
where
  F: Fn(&mut [u16], usize) + Sync,
{
  let mut out: Vec<u16> = alloc_image!(width, height, dummy);
  out.par_chunks_mut(width).enumerate().for_each(|(row, line)| {
    closure(line, row);
  });
  PixU16::new(out, width, height)
}

pub fn decode_threaded_multiline<F>(width: usize, height: usize, lines: usize, dummy: bool, closure: &F) -> PixU16
where
  F: Fn(&mut [u16], usize) + Sync,
{
  let mut out: Vec<u16> = alloc_image!(width, height, dummy);
  out.par_chunks_mut(width * lines).enumerate().for_each(|(row, line)| {
    closure(line, row * lines);
  });
  PixU16::new(out, width, height)
}

// For rust <= 1.31 we just alias chunks_exact() and chunks_exact_mut() to the non-exact versions
// so we can use exact everywhere without spreading special cases across the code
#[cfg(needs_chunks_exact)]
mod chunks_exact {
  use std::slice;

  // Add a chunks_exact for &[u8] and Vec<u16>
  pub trait ChunksExact<T> {
    fn chunks_exact(&self, n: usize) -> slice::Chunks<T>;
  }
  impl<'a, T> ChunksExact<T> for &'a [T] {
    fn chunks_exact(&self, n: usize) -> slice::Chunks<T> {
      self.chunks(n)
    }
  }
  impl<T> ChunksExact<T> for Vec<T> {
    fn chunks_exact(&self, n: usize) -> slice::Chunks<T> {
      self.chunks(n)
    }
  }

  // Add a chunks_exact_mut for &mut[u16] mostly
  pub trait ChunksExactMut<'a, T> {
    fn chunks_exact_mut(self, n: usize) -> slice::ChunksMut<'a, T>;
  }
  impl<'a, T> ChunksExactMut<'a, T> for &'a mut [T] {
    fn chunks_exact_mut(self, n: usize) -> slice::ChunksMut<'a, T> {
      self.chunks_mut(n)
    }
  }
}

#[cfg(needs_chunks_exact)]
pub use self::chunks_exact::*;

pub trait ExifWrite {
  //fn write_tag_xx<T: TiffValue>(&mut self, tag: Tag, value: T) -> DngResult<()>;

  //fn write_entry(&mut self, entry: &TiffEntry) -> std::result::Result<(), String>;

  fn write_tag_rational(&mut self, tag: LegacyTiffTag, value: Rational) -> std::result::Result<(), String>;
  fn write_tag_srational(&mut self, tag: LegacyTiffTag, value: SRational) -> std::result::Result<(), String>;

  fn write_tag_u16(&mut self, tag: LegacyTiffTag, value: u16) -> std::result::Result<(), String>;
  fn write_tag_u32(&mut self, tag: LegacyTiffTag, value: u32) -> std::result::Result<(), String>;
  fn write_tag_u8(&mut self, tag: LegacyTiffTag, value: u8) -> std::result::Result<(), String>;
  fn write_tag_u8_array(&mut self, tag: LegacyTiffTag, value: &[u8]) -> std::result::Result<(), String>;
  fn write_tag_u16_array(&mut self, tag: LegacyTiffTag, value: &[u16]) -> std::result::Result<(), String>;
  fn write_tag_u32_array(&mut self, tag: LegacyTiffTag, value: &[u32]) -> std::result::Result<(), String>;
  fn write_tag_str(&mut self, tag: LegacyTiffTag, value: &str) -> std::result::Result<(), String>;
}

#[derive(Debug, Clone, Default)]
pub struct Gps {
  pub version_id: Option<[u8; 4]>,
  pub latitude_ref: Option<String>,
  pub latitude: Option<[Rational; 3]>,
  pub longitude_ref: Option<String>,
  pub longitude: Option<[Rational; 3]>,
  pub altitude_ref: Option<u8>,
  pub altitude: Option<Rational>,
  pub time_stamp: Option<[Rational; 3]>,
  pub satellites: Option<String>,
  pub status: Option<String>,
  pub map_datum: Option<String>,
  pub date_stamp: Option<String>,
}
