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
use super::Orientation;
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
    let make = Self::xmp_tag(&tiff, "Make")?;
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
      .ok_or_else(|| decoder_error!("Failed to find a IFD with TileOffsets tag"))?;
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let offset = fetch_tiff_tag!(raw, TiffCommonTag::TileOffsets).force_usize(0);
    let compression = fetch_tiff_tag!(raw, TiffCommonTag::Compression).force_usize(0);
    let src = file.subview_until_eof(offset as u64)?;

    let image = match compression {
      1 => {
        if self.tiff.little_endian() {
          decompress_16le(src, width, height, dummy).map_err(RawlerError::decoder_failed)?
        } else {
          decompress_16be(src, width, height, dummy).map_err(RawlerError::decoder_failed)?
        }
      }
      7 | 99 => self.decode_compressed(camera, src, width, height, dummy)?,
      x => return Err(RawlerError::unsupported(camera, format!("MOS: unsupported compression {}", x))),
    };

    let cpp = 1;
    let mut img = ok_cfa_image(camera.clone(), cpp, self.get_wb()?, image, dummy)?;
    img.orientation = self.orientation();
    Ok(img)
  }

  fn check_supported(&self) -> Result<()> {
    self.camera.supported().map(|_| ())
  }

  fn format_dump(&self) -> FormatDump {
    todo!()
  }

  fn raw_metadata(&self, _file: &RawSource, _params: &RawDecodeParams) -> Result<RawMetadata> {
    let mut exif = Exif::new(self.tiff.root_ifd())?;
    exif.orientation = self.orientation().map(|orientation| orientation.to_u16());
    let mdata = RawMetadata::new(self.camera.metadata(), exif);
    Ok(mdata)
  }

  fn format_hint(&self) -> FormatHint {
    FormatHint::MOS
  }
}

impl<'a> MosDecoder<'a> {
  /// Text of a section of the Leaf metadata. The name field has 40 bytes, it
  /// is followed by 4 bytes with the section length and the NUL terminated text.
  fn leaf_section(meta: &[u8], name: &str) -> Option<String> {
    let pos = meta.windows(name.len()).position(|window| window == name.as_bytes())?;
    let data = meta.get(pos + 44..)?;
    let end = data.iter().position(|&x| x == 0)?;
    Some(String::from_utf8_lossy(&data[..end]).into_owned())
  }

  /// The Leaf metadata has the rotation in degrees, it is relative to the
  /// rotation of the raw data. Without it the TIFF tag is used.
  fn orientation(&self) -> Option<Orientation> {
    let leaf_rotation = || {
      let meta = self.tiff.get_entry(TiffCommonTag::LeafMetadata)?.get_data();
      let degrees = |name| Self::leaf_section(meta, name)?.trim().parse::<i32>().ok();
      let raw_data = degrees("CaptProf_raw_data_rotation");
      match degrees("ImgProf_rotation_angle") {
        Some(image) => Some(image - raw_data.unwrap_or(0)),
        None => raw_data,
      }
    };
    leaf_rotation()
      .and_then(Orientation::from_degrees)
      .or_else(|| Orientation::from_tiff(self.tiff.root_ifd()))
  }

  fn get_wb(&self) -> Result<[f32; 4]> {
    let meta = fetch_tiff_tag!(self.tiff, TiffCommonTag::LeafMetadata).get_data();
    if let Some(neutrals) = Self::leaf_section(meta, "NeutObj_neutrals") {
      let nums = neutrals
        .split_terminator('\n')
        .map(|x| x.parse::<f32>().unwrap_or(f32::NAN))
        .collect::<Vec<f32>>();
      if nums.len() == 4 {
        return Ok([nums[0] / nums[1], nums[0] / nums[2], nums[0] / nums[3], f32::NAN]);
      }
    }
    Ok([f32::NAN, f32::NAN, f32::NAN, f32::NAN])
  }

  fn xmp_tag(tiff: &GenericTiffReader, tag: &str) -> Result<String> {
    let xmp_bytes = fetch_tiff_tag!(tiff, TiffCommonTag::Xmp).get_data();
    let xmp = String::from_utf8_lossy(xmp_bytes);
    let error = format!("MOS: Couldn't find XMP tag {}", tag);
    let start = xmp.find(&format!("<tiff:{}>", tag)).ok_or_else(|| RawlerError::decoder_failed(error.clone()))?;
    let end = xmp.find(&format!("</tiff:{}>", tag)).ok_or_else(|| RawlerError::decoder_failed(error))?;

    Ok(xmp[start + tag.len() + 7..end].to_string())
  }

  pub fn decode_compressed(&self, cam: &Camera, src: &[u8], width: usize, height: usize, dummy: bool) -> Result<PixU16> {
    let interlaced = cam.find_hint("interlaced");
    Self::do_decode(src, interlaced, width, height, dummy)
  }

  pub(crate) fn do_decode(src: &[u8], interlaced: bool, width: usize, height: usize, dummy: bool) -> Result<PixU16> {
    if dummy {
      return Ok(PixU16::new_uninit(width, height));
    }

    let decompressor = LjpegDecompressor::new_full(src, true, true).map_err(RawlerError::decoder_failed)?;
    let ljpegout = decompressor.decode_leaf(width, height).map_err(RawlerError::decoder_failed)?;
    if interlaced {
      let mut out = alloc_image_ok!(width, height, dummy);
      for (row, line) in ljpegout.pixels().chunks_exact(width).enumerate() {
        let orow = if row & 1 == 1 { height - 1 - row / 2 } else { row / 2 };
        out[orow * width..(orow + 1) * width].copy_from_slice(line);
      }
      Ok(out)
    } else {
      Ok(ljpegout)
    }
  }
}
