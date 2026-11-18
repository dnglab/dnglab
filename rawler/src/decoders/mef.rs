use crate::RawImage;
use crate::RawLoader;
use crate::RawlerError;
use crate::Result;
use crate::analyze::FormatDump;
use crate::decompressors::packed::decompress_12be;
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
use crate::formats::tiff::IFD;
use crate::tags::ExifTag;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub struct MefDecoder<'a> {
  #[allow(unused)]
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
  camera: CameraLookup,
}

impl<'a> MefDecoder<'a> {
  pub fn new(_file: &RawSource, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<MefDecoder<'a>> {
    let camera = rawloader.lookup_camera(tiff.root_ifd())?;
    Ok(MefDecoder { tiff, rawloader, camera })
  }
}

impl<'a> Decoder for MefDecoder<'a> {
  fn raw_image(&self, file: &RawSource, _params: &RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let camera = self.camera.supported()?;
    let raw = &self
      .tiff
      .find_first_ifd_with_tag(TiffCommonTag::CFAPattern)
      .ok_or_else(|| decoder_error!("Failed to find a IFD with CFAPattern tag"))?;
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let offset = fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_usize(0);
    image_bytes("MEF", width, height, 12)?;
    let src = file.subview_until_eof(offset as u64)?;

    let image = decompress_12be(src, width, height, dummy).map_err(RawlerError::decoder_failed)?;
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

  fn raw_metadata(&self, _file: &RawSource, __params: &RawDecodeParams) -> Result<RawMetadata> {
    let exif = Exif::new(self.tiff.root_ifd())?;
    let mdata = RawMetadata::new(self.camera.metadata(), exif);
    Ok(mdata)
  }

  fn ifd(&self, _file: &RawSource, _params: &RawDecodeParams, wk_ifd: WellKnownIFD) -> Result<Option<Rc<IFD>>> {
    Ok(match wk_ifd {
      WellKnownIFD::VirtualMakernotes => makernotes::from_exif(self.format_hint(), self.tiff.find_first_ifd_with_tag(ExifTag::MakerNotes), None).map(Rc::new),
      _ => None,
    })
  }

  fn format_hint(&self) -> FormatHint {
    FormatHint::MEF
  }
}
