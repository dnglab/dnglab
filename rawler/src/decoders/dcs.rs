use crate::RawImage;
use crate::RawLoader;
use crate::RawlerError;
use crate::Result;
use crate::analyze::FormatDump;
use crate::bits::LookupTable;
use crate::decompressors::packed::decompress_8bit_wtable;
use crate::exif::Exif;
use crate::formats::tiff::GenericTiffReader;
use crate::formats::tiff::reader::TiffReader;
use crate::rawsource::RawSource;
use crate::tags::TiffCommonTag;

use super::CameraLookup;
use super::Decoder;
use super::FormatHint;
use super::Orientation;
use super::RawDecodeParams;
use super::RawMetadata;
use super::WellKnownIFD;
use super::image_bytes;
use super::makernotes;
use super::ok_cfa_image;
use super::tiff_ifd;
use crate::formats::tiff::IFD;
use crate::tags::ExifTag;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct DcsDecoder<'a> {
  #[allow(unused)]
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
  camera: CameraLookup,
}

impl<'a> DcsDecoder<'a> {
  pub fn new(_file: &RawSource, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<DcsDecoder<'a>> {
    let camera = rawloader.lookup_camera(tiff.root_ifd())?;

    Ok(DcsDecoder { camera, tiff, rawloader })
  }

  /// The IFD with the raw image data
  fn get_raw_ifd(&self) -> Result<&IFD> {
    self
      .tiff
      .find_ifd_with_new_subfile_type(0)
      .ok_or_else(|| decoder_error!("Failed to find IFD with subfile type 0"))
  }
}

impl<'a> Decoder for DcsDecoder<'a> {
  fn raw_image(&self, file: &RawSource, _params: &RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let camera = self.camera.supported()?;
    let raw = self.get_raw_ifd()?;
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let offset = fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_usize(0);
    image_bytes("DCS", width, height, 8)?;
    let src = file.subview_until_eof_padded(offset as u64)?; // TODO add size and check all samples

    let linearization = fetch_tiff_tag!(self.tiff, TiffCommonTag::GrayResponse);
    if linearization.count() < 256 {
      return Err(decoder_error!("DCS: GrayResponseCurve has {} values, 256 are needed", linearization.count()));
    }
    let table = {
      let mut t: [u16; 256] = [0; 256];
      for i in 0..256 {
        t[i] = linearization.force_u32(i) as u16;
      }
      LookupTable::new(&t)
    };

    let image = decompress_8bit_wtable(&src, &table, width, height, dummy).map_err(RawlerError::decoder_failed)?;
    let cpp = 1;
    let mut img = ok_cfa_image(camera.clone(), cpp, [f32::NAN, f32::NAN, f32::NAN, f32::NAN], image, dummy)?;
    img.orientation = Orientation::from_tiff(self.tiff.root_ifd());
    Ok(img)
  }

  fn check_supported(&self) -> Result<()> {
    self.camera.supported().map(|_| ())
  }

  fn format_dump(&self) -> FormatDump {
    todo!()
  }

  fn raw_metadata(&self, _file: &RawSource, _params: &RawDecodeParams) -> Result<RawMetadata> {
    let exif = Exif::new(self.tiff.root_ifd())?;
    let mdata = RawMetadata::new(self.camera.metadata(), exif);
    Ok(mdata)
  }

  fn ifd(&self, _file: &RawSource, _params: &RawDecodeParams, wk_ifd: WellKnownIFD) -> Result<Option<Arc<IFD>>> {
    Ok(match wk_ifd {
      WellKnownIFD::Root | WellKnownIFD::Exif | WellKnownIFD::ExifGps => tiff_ifd(self.tiff.root_ifd(), wk_ifd),
      WellKnownIFD::Raw => Some(Arc::new(self.get_raw_ifd()?.clone())),
      WellKnownIFD::VirtualMakernotes => makernotes::from_exif(self.format_hint(), self.tiff.find_first_ifd_with_tag(ExifTag::MakerNotes), None).map(Arc::new),
      _ => None,
    })
  }

  fn format_hint(&self) -> FormatHint {
    FormatHint::DCS
  }
}

#[cfg(test)]
mod tests {
  use std::io::Cursor;

  use super::*;
  use crate::formats::tiff::writer::TiffWriter;

  /// DCS460D file with an image of the given size and a gray response curve
  /// of `curve_len` values
  fn dcs_file(width: u32, height: u32, curve_len: usize) -> std::result::Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut buf = Cursor::new(Vec::new());
    let tiff = TiffWriter::new(&mut buf)?;
    let mut root = tiff.new_directory();
    root.add_tag(TiffCommonTag::Make, "KODAK");
    root.add_tag(TiffCommonTag::Model, "DCS460D         FILE VERSION 3");
    root.add_tag(TiffCommonTag::NewSubFileType, 0_u32);
    root.add_tag(TiffCommonTag::ImageWidth, width);
    root.add_tag(TiffCommonTag::ImageLength, height);
    root.add_tag(TiffCommonTag::StripOffsets, 8_u32);
    root.add_tag(TiffCommonTag::GrayResponse, &vec![0_u16; curve_len]);
    tiff.build(root)?;
    Ok(buf.into_inner())
  }

  #[test]
  fn invalid_size_or_curve_is_an_error() -> std::result::Result<(), Box<dyn std::error::Error>> {
    for (width, height, curve_len) in [(0, 4, 256), (4, 0, 256), (4, 4, 255)] {
      let source = RawSource::new_from_slice(&dcs_file(width, height, curve_len)?);
      let decoder = crate::get_decoder(&source)?;
      let result = decoder.raw_image(&source, &RawDecodeParams::default(), false);
      assert!(
        matches!(&result, Err(RawlerError::DecoderFailed { message, .. }) if message.starts_with("DCS: ")),
        "{}x{}, {} values: {:?}",
        width,
        height,
        curve_len,
        result.err()
      );
    }
    Ok(())
  }
}
