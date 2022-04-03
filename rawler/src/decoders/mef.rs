use crate::analyze::FormatDump;
use std::f32::NAN;

use crate::exif::Exif;
use crate::formats::tiff::reader::TiffReader;
use crate::formats::tiff::GenericTiffReader;
use crate::packed::decode_12be;

use crate::tags::TiffCommonTag;
use crate::RawFile;
use crate::RawImage;
use crate::RawLoader;
use crate::Result;

use super::ok_image;
use super::Camera;
use super::Decoder;
use super::RawDecodeParams;
use super::RawMetadata;

#[derive(Debug, Clone)]
pub struct MefDecoder<'a> {
  #[allow(unused)]
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
  camera: Camera,
}

impl<'a> MefDecoder<'a> {
  pub fn new(_file: &mut RawFile, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<MefDecoder<'a>> {
    let camera = rawloader.check_supported(tiff.root_ifd())?;
    Ok(MefDecoder { tiff, rawloader, camera })
  }
}

impl<'a> Decoder for MefDecoder<'a> {
  fn raw_image(&self, file: &mut RawFile, _params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let raw = &self.tiff.find_first_ifd_with_tag(TiffCommonTag::CFAPattern).unwrap();
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let offset = fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_usize(0);
    let src = file.subview_until_eof(offset as u64).unwrap();

    let image = decode_12be(&src, width, height, dummy);
    let cpp = 1;
    ok_image(self.camera.clone(), width, height, cpp, [NAN, NAN, NAN, NAN], image.into_inner())
  }

  fn format_dump(&self) -> FormatDump {
    todo!()
  }

  fn raw_metadata(&self, _file: &mut RawFile, __params: RawDecodeParams) -> Result<RawMetadata> {
    let exif = Exif::new(self.tiff.root_ifd())?;
    let mdata = RawMetadata::new(&self.camera, exif);
    Ok(mdata)
  }
}
