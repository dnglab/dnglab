use std::f32::NAN;

use crate::analyze::FormatDump;
use crate::bits::LookupTable;
use crate::exif::Exif;
use crate::formats::tiff::reader::TiffReader;
use crate::formats::tiff::GenericTiffReader;
use crate::packed::decode_8bit_wtable;
use crate::tags::TiffCommonTag;
use crate::OptBuffer;
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
pub struct DcsDecoder<'a> {
  #[allow(unused)]
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
  camera: Camera,
}

impl<'a> DcsDecoder<'a> {
  pub fn new(_file: &mut RawFile, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<DcsDecoder<'a>> {
    let camera = rawloader.check_supported(tiff.root_ifd())?;

    Ok(DcsDecoder { camera, tiff, rawloader })
  }
}

impl<'a> Decoder for DcsDecoder<'a> {
  fn raw_image(&self, file: &mut RawFile, _params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let raw = self.tiff.find_ifd_with_new_subfile_type(0).unwrap();
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let offset = fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_usize(0);
    let src: OptBuffer = file.subview_until_eof(offset as u64).unwrap().into(); // TODO add size and check all samples

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
    ok_image(self.camera.clone(), cpp, [NAN, NAN, NAN, NAN], image)
  }

  fn format_dump(&self) -> FormatDump {
    todo!()
  }

  fn raw_metadata(&self, _file: &mut RawFile, _params: RawDecodeParams) -> Result<RawMetadata> {
    let exif = Exif::new(self.tiff.root_ifd())?;
    let mdata = RawMetadata::new(&self.camera, exif);
    Ok(mdata)
  }
}
