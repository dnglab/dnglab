use std::{
  fs::{metadata, File},
  io::{BufReader, Write},
  path::Path,
};

use byteorder::{BigEndian, WriteBytesExt};
use hex::FromHex;
use image::DynamicImage;
use itertools::Itertools;
use md5::Digest;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{
  buffer::Buffer,
  decoders::{cr2::Cr2Format, cr3::Cr3Format, dng::DngFormat, iiq::IiqFormat, nef::NefFormat, pef::PefFormat, tfr::TfrFormat, RawDecodeParams, RawMetadata},
  formats::tiff::Rational,
  formats::tiff::SRational,
  imgop::{raw::develop_raw_srgb, rescale_f32_to_u16, Dim2, Rect},
  rawimage::BlackLevel,
  RawFile, RawImage, RawImageData, RawlerError, Result,
};

#[derive(Debug, Clone, PartialEq)]
pub struct Md5Digest {
  digest: md5::Digest,
}

impl From<md5::Digest> for Md5Digest {
  fn from(digest: md5::Digest) -> Self {
    Self { digest }
  }
}

impl Serialize for Md5Digest {
  fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
  where
    S: Serializer,
  {
    let s = format!("{:x}", self.digest);
    serializer.serialize_str(&s)
  }
}

impl<'de> Deserialize<'de> for Md5Digest {
  fn deserialize<D>(deserializer: D) -> std::result::Result<Md5Digest, D::Error>
  where
    D: Deserializer<'de>,
  {
    use serde::de::Error;
    let s = String::deserialize(deserializer)?;
    if s.len() != 32 {
      Err(D::Error::custom(format!("Invalid digest value: {}", s)))
    } else {
      Ok(Md5Digest {
        digest: Digest(<[u8; 16]>::from_hex(s).map_err(D::Error::custom)?),
      })
    }
  }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMetadata {
  file_size: u64,
  file_name: String,
  digest: Option<Md5Digest>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzerMetadata {
  pub raw_params: RawParams,
  pub raw_metadata: RawMetadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::large_enum_variant)]
pub enum AnalyzerData {
  FileStructure(FormatDump),
  Metadata(AnalyzerMetadata),
  RawParams(RawParams),
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzerResult {
  pub file: FileMetadata,
  pub data: Option<AnalyzerData>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureInfo {
  pub make: String,
  pub model: String,
  pub exposure_time: Option<Rational>,
  pub shutter_speed: Option<SRational>,
  pub exposure_bias: Option<SRational>,
  pub lens_make: Option<String>,
  pub lens_model: Option<String>,
  pub lens_spec: Option<[Rational; 4]>,
}
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawParams {
  pub raw_width: usize,
  pub raw_height: usize,
  pub bit_depth: usize,
  pub crops: Option<Rect>,
  pub blacklevels: BlackLevel,
  pub whitelevels: Vec<u16>,
  pub wb_coeffs: (Option<f32>, Option<f32>, Option<f32>, Option<f32>),
}

impl From<&RawImage> for RawParams {
  fn from(rawimage: &RawImage) -> Self {
    Self {
      raw_width: rawimage.width,
      raw_height: rawimage.height,
      bit_depth: 16,
      crops: rawimage.crop_area,
      blacklevels: rawimage.blacklevel.clone(),
      whitelevels: rawimage.whitelevel.clone(),
      wb_coeffs: rawimage
        .wb_coeffs
        .iter()
        .map(|c| if c.is_nan() { None } else { Some(*c) })
        .collect_tuple()
        .unwrap(),
    }
  }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::large_enum_variant)]
pub enum FormatDump {
  Cr3(Cr3Format),
  Cr2(Cr2Format),
  Pef(PefFormat),
  Iiq(IiqFormat),
  Tfr(TfrFormat),
  Nef(NefFormat),
  Dng(DngFormat),
}

fn file_metadata<P: AsRef<Path>>(path: P, rawfile: &mut RawFile) -> Result<FileMetadata> {
  let fs_meta = metadata(&path).map_err(|e| RawlerError::with_io_error("read metadata", &path, e))?;
  let digest = rawfile
    .digest()
    .map_err(|e| RawlerError::with_io_error("Failed to calculate digest", &path, e))?;
  Ok(FileMetadata {
    file_name: path.as_ref().file_name().unwrap().to_string_lossy().to_string(),
    file_size: fs_meta.len(),
    digest: Some(digest.into()),
  })
}

pub fn analyze_metadata<P: AsRef<Path>>(path: P) -> Result<AnalyzerResult> {
  let input = BufReader::new(File::open(&path).map_err(|e| RawlerError::with_io_error("load into buffer", &path, e))?);
  let mut rawfile = RawFile::new(&path, input);
  let decoder = crate::get_decoder(&mut rawfile)?;
  let rawimage = decoder.raw_image(&mut rawfile, RawDecodeParams::default(), true)?;

  let mut result = AnalyzerResult {
    file: file_metadata(path, &mut rawfile)?,
    ..Default::default()
  };

  let md = decoder.raw_metadata(&mut rawfile, RawDecodeParams::default())?;
  result.data = Some(AnalyzerData::Metadata(AnalyzerMetadata {
    raw_params: RawParams::from(&rawimage),
    raw_metadata: md,
  }));
  Ok(result)
}

pub fn analyze_file_structure<P: AsRef<Path>>(path: P) -> Result<AnalyzerResult> {
  let input = BufReader::new(File::open(&path).map_err(|e| RawlerError::with_io_error("load buffer", &path, e))?);
  let mut rawfile = RawFile::new(&path, input);
  let decoder = crate::get_decoder(&mut rawfile)?;

  let result = AnalyzerResult {
    file: file_metadata(path, &mut rawfile)?,
    data: Some(AnalyzerData::FileStructure(decoder.format_dump())),
  };
  Ok(result)
}

pub fn extract_raw_pixels<P: AsRef<Path>>(path: P, params: RawDecodeParams) -> Result<(usize, usize, usize, Vec<u16>)> {
  let input = BufReader::new(File::open(&path).map_err(|e| RawlerError::with_io_error("load buffer", &path, e))?);
  let mut rawfile = RawFile::new(path, input);
  let decoder = crate::get_decoder(&mut rawfile)?;
  let rawimage = decoder.raw_image(&mut rawfile, params, false)?;
  match rawimage.data {
    RawImageData::Integer(buf) => Ok((rawimage.width, rawimage.height, rawimage.cpp, buf)),
    RawImageData::Float(_) => todo!(),
  }
}

pub fn raw_pixels_digest<P: AsRef<Path>>(path: P, params: RawDecodeParams) -> Result<[u8; 16]> {
  let (_, _, _, pixels) = extract_raw_pixels(path, params)?;
  let v: Vec<u8> = pixels.iter().flat_map(|p| p.to_le_bytes()).collect();
  Ok(md5::compute(&v).into())
}

pub fn extract_full_pixels<P: AsRef<Path>>(path: P, _params: RawDecodeParams) -> Result<DynamicImage> {
  let input = BufReader::new(File::open(&path).map_err(|e| RawlerError::with_io_error("load buffer", &path, e))?);
  let mut rawfile = RawFile::new(path, input);
  let decoder = crate::get_decoder(&mut rawfile)?;
  match decoder.full_image(&mut rawfile)? {
    Some(preview) => Ok(preview),
    None => Err("Unable to extract full image from RAW".into()),
  }
}

pub fn extract_preview_pixels<P: AsRef<Path>>(path: P, _params: RawDecodeParams) -> Result<DynamicImage> {
  let input = BufReader::new(File::open(&path).map_err(|e| RawlerError::with_io_error("load buffer", &path, e))?);
  let mut rawfile = RawFile::new(path, input);
  let decoder = crate::get_decoder(&mut rawfile)?;
  match decoder.preview_image(&mut rawfile)? {
    Some(preview) => Ok(preview),
    None => match decoder.full_image(&mut rawfile)? {
      Some(preview) => Ok(preview),
      None => Err("Unable to extract preview image from RAW".into()),
    },
  }
}

pub fn extract_thumbnail_pixels<P: AsRef<Path>>(path: P, _params: RawDecodeParams) -> Result<DynamicImage> {
  let input = BufReader::new(File::open(&path).map_err(|e| RawlerError::with_io_error("load buffer", &path, e))?);
  let mut rawfile = RawFile::new(path, input);
  let decoder = crate::get_decoder(&mut rawfile)?;
  match decoder.thumbnail_image(&mut rawfile)? {
    Some(thumbnail) => Ok(thumbnail),
    None => match decoder.preview_image(&mut rawfile)? {
      Some(thumbnail) => Ok(thumbnail),
      None => match decoder.full_image(&mut rawfile)? {
        Some(thumbnail) => Ok(thumbnail),
        None => Err("Unable to extract thumbnail image from RAW".into()),
      },
    },
  }
}

pub fn raw_to_srgb<P: AsRef<Path>>(path: P, params: RawDecodeParams) -> Result<(Vec<u16>, Dim2)> {
  let mut raw_file = BufReader::new(File::open(&path).map_err(|e| RawlerError::with_io_error("load buffer", &path, e))?);

  // Read whole raw file
  // TODO: Large input file bug, we need to test the raw file before open it
  let in_buffer = Buffer::new(&mut raw_file)?;

  let mut rawfile = in_buffer.into();

  // Get decoder or return
  let decoder = crate::get_decoder(&mut rawfile)?;
  //decoder.decode_metadata(&mut rawfile)?;
  let rawimage = decoder.raw_image(&mut rawfile, params, false)?;
  let params = rawimage.develop_params()?;
  let buf = match rawimage.data {
    RawImageData::Integer(buf) => buf,
    RawImageData::Float(_) => todo!(),
  };
  assert_eq!(rawimage.cpp, 1);
  let (srgbf, dim) = develop_raw_srgb(&buf, &params)?;
  let output = rescale_f32_to_u16(&srgbf, 0, u16::MAX);
  Ok((output, dim))
}

/// Dump raw pixel data as PGM
pub fn raw_as_pgm(width: usize, height: usize, buf: &[u16], writer: &mut dyn Write) -> std::io::Result<()> {
  let header = format!("P5 {} {} {}\n", width, height, 65535);
  writer.write_all(header.as_bytes())?;
  for px in buf {
    writer.write_u16::<BigEndian>(*px)?;
  }
  Ok(())
}

/// Dump  pixel data as PGM
pub fn rgb8_as_ppm8(width: usize, height: usize, buf: &[u8], writer: &mut dyn Write) -> std::io::Result<()> {
  let header = format!("P6 {} {} {}\n", width, height, u8::MAX);
  writer.write_all(header.as_bytes())?;
  writer.write_all(buf)?;
  Ok(())
}

/// Dump raw pixel data as PPM
pub fn raw_as_ppm16(width: usize, height: usize, buf: &[u16], writer: &mut dyn Write) -> std::io::Result<()> {
  let header = format!("P6 {} {} {}\n", width, height, 65535);
  writer.write_all(header.as_bytes())?;
  for px in buf {
    writer.write_u16::<BigEndian>(*px)?;
  }
  Ok(())
}
