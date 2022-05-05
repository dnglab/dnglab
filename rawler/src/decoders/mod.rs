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

use crate::analyze::FormatDump;
use crate::exif::Exif;
use crate::formats::ciff;
use crate::formats::tiff::reader::TiffReader;
use crate::formats::tiff::GenericTiffReader;
use crate::formats::tiff::IFD;
use crate::lens::LensDescription;
use crate::pixarray::PixU16;
use crate::tags::DngTag;
use crate::RawFile;
use crate::Result;

macro_rules! fetch_ciff_tag {
  ($tiff:expr, $tag:expr) => {
    $tiff.find_entry($tag).ok_or(format!("Couldn't find tag {}", stringify!($tag)).to_string())?
  };
}

macro_rules! fetch_tiff_tag {
  ($ifd:expr, $tag:expr) => {
    $ifd
      .get_entry($tag)
      .map(|entry| &entry.value)
      .ok_or(format!("Couldn't find tag {}", stringify!($tag)))?
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

pub mod ari;
pub mod arw;
mod camera;
pub mod cr2;
pub mod cr3;
pub mod crw;
pub mod dcr;
pub mod dcs;
pub mod dng;
pub mod erf;
pub mod iiq;
pub mod kdc;
pub mod mef;
pub mod mos;
pub mod mrw;
pub mod nef;
pub mod nkd;
pub mod nrw;
pub mod orf;
pub mod pef;
pub mod raf;
pub mod rw2;
pub mod srw;
pub mod tfr;
mod unwrapped;
pub mod x3f;

pub use camera::Camera;

use crate::alloc_image;
use crate::formats::bmff::Bmff;
use crate::tags::ExifTag;
use crate::tags::TiffCommonTag;
use crate::RawlerError;

pub use super::rawimage::*;

pub static CAMERAS_TOML: &str = include_str!(concat!(env!("OUT_DIR"), "/cameras.toml"));
pub static SAMPLE: &str = "\nPlease submit samples at https://raw.pixls.us/";
pub static BUG: &str = "\nPlease file a bug with a sample file at https://github.com/dnglab/dnglab/issues";

const SUPPORTED_FILES_EXT: [&str; 25] = [
  "ARI", "ARW", "CR2", "CR3", "CRM", "CRW", "DCR", "DCS", "DNG", "ERF", "IIQ", "KDC", "MEF", "MOS", "MRW", "NEF", "NRW", "ORF", "PEF", "RAF", "RW2", "SRW",
  "3FR", "FFF", "X3F",
];

/// Get list of supported file extensions. All names
/// are upper-case.
pub fn supported_extensions() -> &'static [&'static str] {
  &SUPPORTED_FILES_EXT[..]
}

pub trait Readable: std::io::Read + std::io::Seek {}

pub type ReadableBoxed = Box<dyn Readable>;

#[derive(Default, Clone, Debug)]
pub struct RawDecodeParams {
  pub image_index: usize,
}

#[derive(Default, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RawMetadata {
  pub exif: Exif,
  pub model: String,
  pub make: String,
  pub lens: Option<LensDescription>,
  pub unique_image_id: Option<u128>,
  pub rating: Option<u32>,
}

impl RawMetadata {
  pub(crate) fn new(camera: &Camera, exif: Exif) -> Self {
    Self {
      exif,
      model: camera.clean_model.clone(),
      make: camera.clean_make.clone(),
      unique_image_id: None,
      lens: None,
      rating: None,
    }
  }

  pub(crate) fn new_with_lens(camera: &Camera, mut exif: Exif, lens: Option<LensDescription>) -> Self {
    if let Some(lens) = &lens {
      exif.extend_from_lens(lens);
    }
    Self {
      exif,
      model: camera.clean_model.clone(),
      make: camera.clean_make.clone(),
      unique_image_id: None,
      lens,
      rating: None,
    }
  }
}

pub trait Decoder: Send {
  fn raw_image(&self, file: &mut RawFile, params: RawDecodeParams, dummy: bool) -> Result<RawImage>;

  fn raw_image_count(&self) -> Result<usize> {
    Ok(1)
  }

  /// Gives the metadata for a Raw. This is not the original data but
  /// a generalized set of metadata attributes.
  fn raw_metadata(&self, file: &mut RawFile, params: RawDecodeParams) -> Result<RawMetadata>;

  fn xpacket(&self, _file: &mut RawFile, _params: RawDecodeParams) -> Result<Option<Vec<u8>>> {
    Ok(None)
  }

  // TODO: extend with decode params for image index
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

  fn format_dump(&self) -> FormatDump;
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
  fn from_tiff(tiff: &IFD) -> Orientation {
    match tiff.get_entry(TiffCommonTag::Orientation) {
      Some(entry) => Orientation::from_u16(entry.force_usize(0) as u16),
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
#[derive(Debug, Clone, Default)]
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
      let mut camnames = vec![(cam.model.clone(), cam.clean_model.clone())];
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

    RawLoader { cameras: map, naked }
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

    if mrw::is_mrw(rawfile) {
      let dec = Box::new(mrw::MrwDecoder::new(rawfile, self)?);
      return Ok(dec as Box<dyn Decoder>);
    }

    if raf::is_raf(rawfile) {
      let dec = Box::new(raf::RafDecoder::new(rawfile, self)?);
      return Ok(dec as Box<dyn Decoder>);
    }

    if ari::is_ari(rawfile) {
      let dec = Box::new(ari::AriDecoder::new(rawfile, self)?);
      return Ok(dec as Box<dyn Decoder>);
    }

    if ciff::is_ciff(rawfile) {
      let dec = Box::new(crw::CrwDecoder::new(rawfile, self)?);
      return Ok(dec as Box<dyn Decoder>);
    }

    if x3f::is_x3f(rawfile) {
      let dec = Box::new(x3f::X3fDecoder::new(rawfile, self)?);
      return Ok(dec as Box<dyn Decoder>);
    }

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
    match GenericTiffReader::new(rawfile.inner(), 0, 0, None, &[]) {
      Ok(tiff) => {
        debug!("File is is TIFF file!");

        macro_rules! use_decoder {
          ($dec:ty, $buf:ident, $tiff:ident, $rawdec:ident) => {
            Ok(Box::new(<$dec>::new(rawfile, $tiff, $rawdec)?) as Box<dyn Decoder>)
          };
        }

        if tiff.has_entry(DngTag::DNGVersion) {
          return Ok(Box::new(dng::DngDecoder::new(rawfile, tiff, self)?));
        }

        // The DCS560C is really a CR2 camera so we just special case it here
        if let Some(model) = tiff.get_entry(TiffCommonTag::Model) {
          if model.get_string().ok() == Some(&String::from("DCS560C")) {
            return use_decoder!(cr2::Cr2Decoder, rawfile, tiff, self);
          }
        }

        if let Some(make) = tiff
          .get_entry(TiffCommonTag::Make)
          .and_then(|entry| entry.value.as_string().map(|s| s.as_str().trim_end()))
        {
          match make {
            "Canon" => return use_decoder!(cr2::Cr2Decoder, rawfile, tiff, self),
            "PENTAX" => return use_decoder!(pef::PefDecoder, rawfile, tiff, self),
            "PENTAX Corporation" => return use_decoder!(pef::PefDecoder, rawfile, tiff, self),
            "RICOH IMAGING COMPANY, LTD." => return use_decoder!(pef::PefDecoder, rawfile, tiff, self),
            "Phase One" => return use_decoder!(iiq::IiqDecoder, rawfile, tiff, self),
            "Phase One A/S" => return use_decoder!(iiq::IiqDecoder, rawfile, tiff, self),
            "Leaf" => return use_decoder!(iiq::IiqDecoder, rawfile, tiff, self),
            "Hasselblad" => return use_decoder!(tfr::TfrDecoder, rawfile, tiff, self),
            "SONY" => return use_decoder!(arw::ArwDecoder, rawfile, tiff, self),
            "Mamiya-OP Co.,Ltd." => return use_decoder!(mef::MefDecoder, rawfile, tiff, self),
            "OLYMPUS IMAGING CORP." => return use_decoder!(orf::OrfDecoder, rawfile, tiff, self),
            "OLYMPUS CORPORATION" => return use_decoder!(orf::OrfDecoder, rawfile, tiff, self),
            "OLYMPUS OPTICAL CO.,LTD" => return use_decoder!(orf::OrfDecoder, rawfile, tiff, self),
            "SAMSUNG" => return use_decoder!(srw::SrwDecoder, rawfile, tiff, self),
            "SEIKO EPSON CORP." => return use_decoder!(erf::ErfDecoder, rawfile, tiff, self),
            "EASTMAN KODAK COMPANY" => return use_decoder!(kdc::KdcDecoder, rawfile, tiff, self),
            "Eastman Kodak Company" => return use_decoder!(kdc::KdcDecoder, rawfile, tiff, self),
            "KODAK" => return use_decoder!(dcs::DcsDecoder, rawfile, tiff, self),
            "Kodak" => return use_decoder!(dcr::DcrDecoder, rawfile, tiff, self),
            "Panasonic" => return use_decoder!(rw2::Rw2Decoder, rawfile, tiff, self),
            "LEICA" => return use_decoder!(rw2::Rw2Decoder, rawfile, tiff, self),
            //"FUJIFILM" => return use_decoder!(raf::RafDecoder, rawfile, tiff, self),
            "NIKON" => return use_decoder!(nrw::NrwDecoder, rawfile, tiff, self),
            "NIKON CORPORATION" => return use_decoder!(nef::NefDecoder, rawfile, tiff, self),
            x => {
              return Err(RawlerError::Unsupported {
                what: format!("Couldn't find a decoder for make \"{}\"", x),
                make: make.to_string(),
                model: String::new(),
                mode: String::new(),
              });
            }
          }
        }

        if tiff.has_entry(TiffCommonTag::Software) {
          // Last ditch effort to identify Leaf cameras without Make and Model
          if fetch_tiff_tag!(tiff, TiffCommonTag::Software).as_string() == Some(&"Camera Library".to_string()) {
            return use_decoder!(mos::MosDecoder, rawfile, tiff, self);
          }
        }
      }
      Err(e) => {
        debug!("File is not a tiff file: {:?}", e);
      }
    }

    // If all else fails see if we match by filesize to one of those CHDK style files
    let data = rawfile.as_vec().unwrap();
    if let Some(cam) = self.naked.get(&data.len()) {
      return Ok(Box::new(nkd::NakedDecoder::new(cam.clone(), self)?));
    }

    Err(RawlerError::Unsupported {
      what: String::from("No decoder found"),
      model: "".to_string(),
      make: "".to_string(),
      mode: "".to_string(),
    })
  }

  /// Check support
  fn check_supported_with_everything<'a>(&'a self, make: &str, model: &str, mode: &str) -> Result<Camera> {
    match self.cameras.get(&(make.to_string(), model.to_string(), mode.to_string())) {
      Some(cam) => Ok(cam.clone()),
      None => Err(RawlerError::Unsupported {
        what: String::from("Unknown camera"),
        model: model.to_string(),
        make: make.to_string(),
        mode: mode.to_string(),
      }),
    }
  }

  fn check_supported_with_mode(&self, ifd0: &IFD, mode: &str) -> Result<Camera> {
    let make = fetch_tiff_tag!(ifd0, TiffCommonTag::Make).get_string()?.trim_end();
    let model = fetch_tiff_tag!(ifd0, TiffCommonTag::Model).get_string()?.trim_end();
    self.check_supported_with_everything(make, model, mode)
  }

  #[allow(dead_code)]
  fn check_supported(&self, ifd0: &IFD) -> Result<Camera> {
    self.check_supported_with_mode(ifd0, "")
  }

  fn decode_unsafe(&self, rawfile: &mut RawFile, params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let decoder = self.get_decoder(rawfile)?;
    decoder.raw_image(rawfile, params, dummy)
  }

  /// Decodes an input into a RawImage
  pub fn decode(&self, rawfile: &mut RawFile, params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    //let buffer = Buffer::new(reader)?;

    match panic::catch_unwind(AssertUnwindSafe(|| self.decode_unsafe(rawfile, params, dummy))) {
      Ok(val) => val,
      Err(_) => Err(RawlerError::General(format!("Caught a panic while decoding.{}", BUG))),
    }
  }

  /// Decodes a file into a RawImage
  pub fn decode_file(&self, path: &Path) -> Result<RawImage> {
    let file = match File::open(path) {
      Ok(val) => val,
      Err(e) => return Err(RawlerError::with_io_error("decode_file()", path, e)),
    };
    let mut buffered_file = RawFile::from(BufReader::new(file));
    self.decode(&mut buffered_file, RawDecodeParams::default(), false)
  }

  /// Decodes a file into a RawImage
  pub fn raw_image_count_file(&self, path: &Path) -> Result<usize> {
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
  pub fn decode_unwrapped(&self, rawfile: &mut RawFile) -> Result<RawImageData> {
    match panic::catch_unwind(AssertUnwindSafe(|| unwrapped::decode_unwrapped(rawfile))) {
      Ok(val) => val,
      Err(_) => Err(RawlerError::General(format!("Caught a panic while decoding.{}", BUG))),
    }
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
  PixU16::new_with(out, width, height)
}

pub fn decode_threaded<F>(width: usize, height: usize, dummy: bool, closure: &F) -> PixU16
where
  F: Fn(&mut [u16], usize) + Sync,
{
  let mut out: Vec<u16> = alloc_image!(width, height, dummy);
  out.par_chunks_mut(width).enumerate().for_each(|(row, line)| {
    closure(line, row);
  });
  PixU16::new_with(out, width, height)
}

pub fn decode_threaded_multiline<F>(width: usize, height: usize, lines: usize, dummy: bool, closure: &F) -> PixU16
where
  F: Fn(&mut [u16], usize) + Sync,
{
  let mut out: Vec<u16> = alloc_image!(width, height, dummy);
  out.par_chunks_mut(width * lines).enumerate().for_each(|(row, line)| {
    closure(line, row * lines);
  });
  PixU16::new_with(out, width, height)
}
