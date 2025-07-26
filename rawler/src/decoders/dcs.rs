use crate::RawImage;
use crate::RawLoader;
use crate::RawlerError;
use crate::Result;
use crate::analyze::FormatDump;
use crate::bits::LookupTable;
use crate::exif::Exif;
use crate::formats::tiff::GenericTiffReader;
use crate::formats::tiff::reader::TiffReader;
use crate::packed::decode_8bit_wtable;
use crate::rawsource::RawSource;
use crate::tags::TiffCommonTag;

use super::Camera;
use super::Decoder;
use super::FormatHint;
use super::RawDecodeParams;
use super::RawMetadata;
use super::ok_cfa_image;

#[derive(Debug, Clone)]
pub struct DcsDecoder<'a> {
  #[allow(unused)]
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
  camera: Camera,
}

impl<'a> DcsDecoder<'a> {
  pub fn new(_file: &RawSource, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<DcsDecoder<'a>> {
    let camera = rawloader.check_supported(tiff.root_ifd())?;

    Ok(DcsDecoder { camera, tiff, rawloader })
  }
}

impl<'a> Decoder for DcsDecoder<'a> {
  fn raw_image(&self, file: &RawSource, _params: &RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let raw = self
      .tiff
      .find_ifd_with_new_subfile_type(0)
      .ok_or_else(|| RawlerError::DecoderFailed(format!("Failed to find IFD with subfile type 0")))?;
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let offset = fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_usize(0);
    let src = file.subview_until_eof_padded(offset as u64)?; // TODO add size and check all samples

    let linearization = fetch_tiff_tag!(self.tiff, TiffCommonTag::GrayResponse);
    let table = {
      let mut t: [u16; 256] = [0; 256];
      for i in 0..256 {
        t[i] = linearization.force_u32(i) as u16;
      }
      LookupTable::new(&t)
    };

    let image = decode_8bit_wtable(&src, &table, width, height, dummy);
    let cpp = 1;
    ok_cfa_image(self.camera.clone(), cpp, [f32::NAN, f32::NAN, f32::NAN, f32::NAN], image, dummy)
  }

  fn format_dump(&self) -> FormatDump {
    todo!()
  }

  fn raw_metadata(&self, _file: &RawSource, _params: &RawDecodeParams) -> Result<RawMetadata> {
    let exif = Exif::new(self.tiff.root_ifd())?;
    let mdata = RawMetadata::new(&self.camera, exif);
    Ok(mdata)
  }

  fn format_hint(&self) -> FormatHint {
    FormatHint::DCS
  }
}
