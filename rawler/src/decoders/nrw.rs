use std::f32::NAN;

use crate::analyze::FormatDump;
use crate::bits::BEu16;
use crate::bits::LEu32;
use crate::exif::Exif;
use crate::formats::tiff::ifd::OffsetMode;
use crate::formats::tiff::reader::TiffReader;
use crate::formats::tiff::GenericTiffReader;
use crate::formats::tiff::IFD;
use crate::packed::decode_12be;
use crate::packed::decode_12be_interlaced_unaligned;
use crate::packed::decode_12be_msb32;
use crate::packed::decode_16be;
use crate::tags::ExifTag;
use crate::tags::TiffCommonTag;
use crate::RawFile;
use crate::RawImage;
use crate::RawLoader;
use crate::RawlerError;
use crate::Result;

use super::ok_image;
use super::Camera;
use super::Decoder;
use super::RawDecodeParams;
use super::RawMetadata;
#[derive(Debug, Clone)]
pub struct NrwDecoder<'a> {
  #[allow(unused)]
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
  camera: Camera,
  makernote: IFD,
}

impl<'a> NrwDecoder<'a> {
  pub fn new(file: &mut RawFile, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<NrwDecoder<'a>> {
    let camera = rawloader.check_supported(tiff.root_ifd())?;

    let makernote = if let Some(exif) = tiff.find_first_ifd_with_tag(ExifTag::MakerNotes) {
      exif.parse_makernote(file.inner(), OffsetMode::Absolute, &[])?
    } else {
      log::warn!("NRW makernote not found");
      None
    }
    .ok_or("File has not makernotes")?;

    Ok(NrwDecoder {
      tiff,
      rawloader,
      camera,
      makernote,
    })
  }
}

impl<'a> Decoder for NrwDecoder<'a> {
  fn raw_image(&self, file: &mut RawFile, _params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let data = self.tiff.find_ifds_with_tag(TiffCommonTag::CFAPattern);
    let raw = data
      .iter()
      .find(|&&ifd| ifd.get_entry(TiffCommonTag::ImageWidth).unwrap().force_u32(0) > 1000)
      .unwrap();
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let offset = fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_usize(0);
    let src = file.subview_until_eof(offset as u64).unwrap();

    let image = if self.camera.find_hint("coolpixsplit") {
      decode_12be_interlaced_unaligned(&src, width, height, dummy)
    } else if self.camera.find_hint("msb32") {
      decode_12be_msb32(&src, width, height, dummy)
    } else if self.camera.find_hint("unpacked") {
      decode_16be(&src, width, height, dummy)
    } else {
      decode_12be(&src, width, height, dummy)
    };

    let wb = self.get_wb(&self.camera)?;
    let cpp = 1;
    ok_image(self.camera.clone(), width, height, cpp, wb, image.into_inner())
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

impl<'a> NrwDecoder<'a> {
  fn get_wb(&self, cam: &Camera) -> Result<[f32; 4]> {
    if cam.find_hint("nowb") {
      Ok([NAN, NAN, NAN, NAN])
    } else if let Some(levels) = self.makernote.get_entry(TiffCommonTag::NefWB0) {
      Ok([levels.force_f32(0), 1.0, levels.force_f32(1), NAN])
    } else if let Some(levels) = self.makernote.get_entry(TiffCommonTag::NrwWB) {
      let data = levels.get_data();
      if data[0..3] == b"NRW"[..] {
        let offset = if data[4..8] == b"0100"[..] { 1556 } else { 56 };
        Ok([
          (LEu32(data, offset) << 2) as f32,
          (LEu32(data, offset + 4) + LEu32(data, offset + 8)) as f32,
          (LEu32(data, offset + 12) << 2) as f32,
          NAN,
        ])
      } else {
        Ok([BEu16(data, 1248) as f32, 256.0, BEu16(data, 1250) as f32, NAN])
      }
    } else {
      Err(RawlerError::General("NRW: Don't know how to fetch WB".to_string()))
    }
  }
}
