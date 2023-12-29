use std::{
  ffi::OsStr,
  fs::File,
  io::{BufReader, Cursor, Read, Seek, Write},
  path::{Path, PathBuf},
  thread::JoinHandle,
};

use image::DynamicImage;

use crate::{
  decoders::{Decoder, RawDecodeParams},
  dng::{original::OriginalCompressed, writer::DngWriter, DNG_VERSION_V1_4, PREVIEW_JPEG_QUALITY},
  imgop::develop::RawDevelop,
  tags::{ExifTag, TiffCommonTag},
  RawFile, RawImage,
};

use super::{CropMode, DngCompression, DngPhotometricConversion};

/// Parameters for DNG conversion
#[derive(Clone, Debug)]
pub struct ConvertParams {
  pub embedded: bool,
  pub compression: DngCompression,
  pub photometric_conversion: DngPhotometricConversion,
  pub apply_scaling: bool,
  pub crop: CropMode,
  pub predictor: u8,
  pub preview: bool,
  pub thumbnail: bool,
  pub artist: Option<String>,
  pub software: String,
  pub index: usize,
}

impl Default for ConvertParams {
  fn default() -> Self {
    Self {
      embedded: true,
      compression: DngCompression::Lossless,
      photometric_conversion: DngPhotometricConversion::Original,
      apply_scaling: false,
      crop: CropMode::Best,
      predictor: 1,
      preview: true,
      thumbnail: true,
      artist: None,
      software: "DNGLab".into(),
      index: 0,
    }
  }
}

/// Convert a raw input file into DNG
///
/// We don't accept a DNG file path here, because we don't know
/// how to handle existing target files, buffering, etc.
/// This is up to the caller.
pub fn convert_raw_file<W: Write + Seek + Send>(raw: &Path, dng: &mut W, params: &ConvertParams) -> crate::Result<()> {
  let original_filename = raw.file_name().and_then(OsStr::to_str).unwrap_or_default();
  let raw_stream = BufReader::new(File::open(raw)?); // TODO: add path hint to error?
  let rawfile = RawFile::new(PathBuf::from(raw), raw_stream);

  let original_compress_thread = if params.embedded {
    let mut original_stream = BufReader::new(File::open(raw)?);
    Some(std::thread::spawn(move || OriginalCompressed::compress(&mut original_stream)))
  } else {
    None
  };

  internal_convert(rawfile, dng, original_filename, original_compress_thread, params)
}

/// Convert a raw input file into DNG
pub fn convert_raw_stream<W, R>(raw_stream: R, dng: &mut W, original_filename: impl AsRef<str>, params: &ConvertParams) -> crate::Result<()>
where
  W: Write + Seek + Send,
  R: Read + Seek + 'static,
{
  let mut rawfile = RawFile::new(PathBuf::from(original_filename.as_ref()), raw_stream);

  let original_compress_thread = if params.embedded {
    let mut original_stream = Cursor::new(rawfile.as_vec()?);
    Some(std::thread::spawn(move || OriginalCompressed::compress(&mut original_stream)))
  } else {
    None
  };

  internal_convert(rawfile, dng, original_filename, original_compress_thread, params)
}

fn internal_convert<W>(
  mut rawfile: RawFile,
  dng: &mut W,
  original_filename: impl AsRef<str>,
  original_compress_thread: Option<JoinHandle<Result<OriginalCompressed, std::io::Error>>>,
  params: &ConvertParams,
) -> crate::Result<()>
where
  W: Write + Seek + Send,
{
  let decoder = crate::get_decoder(&mut rawfile)?;
  let raw_params = RawDecodeParams { image_index: params.index };
  let mut rawimage = decoder.raw_image(&mut rawfile, raw_params.clone(), false)?;
  let metadata = decoder.raw_metadata(&mut rawfile, raw_params.clone())?;

  log::info!(
    "DNG conversion: '{}', make: {}, model: {}, raw-image-count: {}",
    original_filename.as_ref(),
    rawimage.clean_make,
    rawimage.clean_model,
    decoder.raw_image_count()?
  );

  if params.apply_scaling {
    rawimage.apply_scaling()?;
  }

  log::debug!("wb coeff: {:?}", rawimage.wb_coeffs);

  let mut dng = DngWriter::new(dng, DNG_VERSION_V1_4)?;
  // Write RAW image for subframe type 0
  let mut raw = dng.subframe(0);
  raw.raw_image(&rawimage, params.crop, params.compression, params.photometric_conversion, params.predictor)?;
  raw.finalize()?;
  // Write preview and thumbnail if requested
  if params.preview || params.thumbnail {
    match generate_preview(&mut rawfile, decoder.as_ref(), &rawimage) {
      Ok(image) => {
        if params.preview {
          let mut preview = dng.subframe(1);
          preview.preview(&image, PREVIEW_JPEG_QUALITY)?;
          preview.finalize()?;
        }
        if params.thumbnail {
          dng.thumbnail(&image)?;
        }
      }
      Err(err) => log::warn!("Failed to get review image, continue anyway: {:?}", err),
    }
  }
  // Write metadata
  dng.load_base_tags(&rawimage)?;
  dng.load_metadata(&metadata)?;

  if let Some(xpacket) = decoder.xpacket(&mut rawfile, RawDecodeParams::default())? {
    dng.xpacket(&xpacket)?;
  }

  if let Some(handle) = original_compress_thread {
    let original = handle
      .join()
      .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, format!("Failed to join compression thread: {:?}", err)))??;
    dng.original_file(&original, original_filename)?;
  }

  if let Some(artist) = &params.artist {
    dng.root_ifd_mut().add_tag(TiffCommonTag::Artist, artist);
  }
  dng.root_ifd_mut().add_tag(TiffCommonTag::Software, &params.software);

  dng
    .root_ifd_mut()
    .add_tag(ExifTag::ModifyDate, chrono::Local::now().format("%Y:%m:%d %H:%M:%S").to_string());

  dng.close()?;

  Ok(())
}

fn generate_preview(rawfile: &mut RawFile, decoder: &dyn Decoder, rawimage: &RawImage) -> crate::Result<DynamicImage> {
  match decoder.full_image(rawfile)? {
    Some(image) => Ok(image),
    None => {
      log::warn!("Preview image not found, try to generate sRGB from RAW");
      let dev = RawDevelop::default();
      let image = dev.develop_intermediate(rawimage)?;
      /*
      let params = rawimage.develop_params()?;
      let (srgbf, dim) = develop_raw_srgb(&rawimage.data, &params)?;
      let output = convert_from_f32_scaled_u16(&srgbf, 0, u16::MAX);
      let image = if srgbf.len() == dim.w * dim.h {
        DynamicImage::ImageLuma16(ImageBuffer::from_raw(dim.w as u32, dim.h as u32, output).expect("Invalid ImageBuffer size"))
      } else {
        DynamicImage::ImageRgb16(ImageBuffer::from_raw(dim.w as u32, dim.h as u32, output).expect("Invalid ImageBuffer size"))
      };
       */
      Ok(image.to_dynamic_image().unwrap())
    }
  }
}
