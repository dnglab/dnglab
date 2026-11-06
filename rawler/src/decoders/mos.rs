use log::warn;
use serde::Deserialize;
use serde::Serialize;

use crate::RawImage;
use crate::RawLoader;
use crate::RawlerError;
use crate::Result;
use crate::alloc_image_ok;
use crate::analyze::FormatDump;
use crate::decompressors::ljpeg::LjpegDecompressor;
use crate::decompressors::packed::decompress_16be;
use crate::decompressors::packed::decompress_16le;
use crate::exif::Exif;
use crate::formats::tiff::GenericTiffReader;
use crate::formats::tiff::reader::TiffReader;
use crate::pixarray::PixU16;
use crate::rawsource::RawSource;
use crate::tags::TiffCommonTag;

use super::Camera;
use super::CameraLookup;
use super::Decoder;
use super::FormatHint;
use super::RawDecodeParams;
use super::RawMetadata;
use super::ok_cfa_image;

#[derive(Debug, Clone)]
pub struct MosDecoder<'a> {
  #[allow(unused)]
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
  camera: CameraLookup,
}

impl<'a> MosDecoder<'a> {
  pub fn new(_file: &RawSource, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<MosDecoder<'a>> {
    let mut make = Self::xmp_tag(&tiff, "Make")?;
    if make == "Creo/Leaf" || make == "Creo Leaf" {
      make = "Leaf".to_string();
    }
    let model_full = Self::xmp_tag(&tiff, "Model")?;
    let model = model_full.split_terminator('(').next().expect("split always yields at least one element");
    let camera = rawloader.lookup_camera_with_everything(&make, model, "");

    Ok(MosDecoder { tiff, rawloader, camera })
  }
}

impl<'a> Decoder for MosDecoder<'a> {
  fn raw_image(&self, file: &RawSource, _params: &RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let camera = self.camera.supported()?;
    let raw = self
      .tiff
      .find_first_ifd_with_tag(TiffCommonTag::TileOffsets)
      .ok_or_else(|| RawlerError::DecoderFailed(format!("Failed to find a IFD with TileOffsets tag")))?;
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let offset = fetch_tiff_tag!(raw, TiffCommonTag::TileOffsets).force_usize(0);
    let compression = fetch_tiff_tag!(raw, TiffCommonTag::Compression).force_usize(0);
    let src = file.subview_until_eof(offset as u64)?;

    let image = match compression {
      1 => {
        if self.tiff.little_endian() {
          decompress_16le(src, width, height, dummy)?
        } else {
          decompress_16be(src, width, height, dummy)?
        }
      }
      7 | 99 => self.decode_compressed(camera, src, width, height, dummy)?,
      _ => return Err(RawlerError::DecoderFailed(format!("Unknown MOS compression: {}", compression))),
    };

    ok_cfa_image(camera, image)
  }

  fn raw_metadata(&self, file: &RawSource, _params: &RawDecodeParams) -> Result<RawMetadata> {
    let mut raw_md = RawMetadata::new(self.camera.metadata());

    let raw = self
      .tiff
      .find_first_ifd_with_tag(TiffCommonTag::TileOffsets)
      .ok_or_else(|| RawlerError::DecoderFailed(format!("Failed to find a IFD with TileOffsets tag")))?;
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);

    raw_md.exif = Exif::new(file, &self.tiff)?;
    raw_md.format_hints.push(FormatHint::RawImageSize(width, height));
    Ok(raw_md)
  }

  fn format_dump(&self) -> FormatDump {
    FormatDump::Mos(MosFormat { tiff: self.tiff.clone() })
  }
}

/// MOS format encapsulation for analyzer
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MosFormat {
  tiff: GenericTiffReader,
}

impl<'a> MosDecoder<'a> {
  fn xmp_tag(tiff: &GenericTiffReader, tag: &str) -> Result<String> {
    let xmp_bytes = fetch_tiff_tag!(tiff, TiffCommonTag::Xmp).get_data();
    let xmp = String::from_utf8_lossy(xmp_bytes);
    let error = format!("MOS: Couldn't find XMP tag {}", tag);
    let start = xmp.find(&format!("<tiff:{}>", tag)).ok_or_else(|| error.clone())?;
    let end = xmp.find(&format!("</tiff:{}>", tag)).ok_or(error)?;

    Ok(xmp[start + tag.len() + 7..end].to_string())
  }

  pub fn decode_compressed(&self, cam: &Camera, src: &[u8], width: usize, height: usize, dummy: bool) -> Result<PixU16> {
    let interleaved = self.camera.metadata().model == "Aptus 75";
    let img = alloc_image_ok!(dummy, width, height);
    let mut pump = LjpegDecompressor::new(src)?;
    pump.decode(width, height, img, interleaved, cam.whitepoint)?;

    Ok(img)
  }
}
