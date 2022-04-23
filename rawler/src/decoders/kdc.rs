use std::f32::NAN;

use log::warn;

use crate::alloc_image;
use crate::analyze::FormatDump;
use crate::bits::BEu16;
use crate::exif::Exif;
use crate::formats::tiff::ifd::OffsetMode;
use crate::formats::tiff::reader::TiffReader;
use crate::formats::tiff::GenericTiffReader;
use crate::formats::tiff::IFD;
use crate::packed::decode_12be;
use crate::pixarray::PixU16;
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
pub struct KdcDecoder<'a> {
  #[allow(unused)]
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
  makernote: IFD,
  camera: Camera,
}

impl<'a> KdcDecoder<'a> {
  pub fn new(file: &mut RawFile, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<KdcDecoder<'a>> {
    let camera = rawloader.check_supported(tiff.root_ifd())?;

    let makernote = if let Some(exif) = tiff.find_first_ifd_with_tag(ExifTag::MakerNotes) {
      exif.parse_makernote(file.inner(), OffsetMode::Absolute, &[])?
    } else {
      warn!("KDC makernote not found");
      None
    }
    .ok_or("File has not makernotes")?;

    Ok(KdcDecoder {
      tiff,
      rawloader,
      camera,
      makernote,
    })
  }
}

impl<'a> Decoder for KdcDecoder<'a> {
  fn raw_image(&self, file: &mut RawFile, _params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    if self.camera.model == "Kodak DC120 ZOOM Digital Camera" {
      let width = 848;
      let height = 976;
      let raw = self.tiff.find_ifds_with_tag(TiffCommonTag::CFAPattern)[0];
      let off = fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_usize(0);
      let src = file.subview_until_eof(off as u64).unwrap();
      let image = match fetch_tiff_tag!(raw, TiffCommonTag::Compression).force_usize(0) {
        1 => Self::decode_dc120(&src, width, height, dummy),
        c => {
          return Err(RawlerError::unsupported(
            &self.camera,
            format!("KDC: DC120: Don't know how to handle compression type {}", c),
          ))
        }
      };
      let cpp = 1;
      return ok_image(self.camera.clone(), width, height, cpp, [NAN, NAN, NAN, NAN], image.into_inner());
    }

    let raw = self.tiff.find_first_ifd_with_tag(TiffCommonTag::KdcWidth).unwrap();

    let width = fetch_tiff_tag!(raw, TiffCommonTag::KdcWidth).force_usize(0) + 80;
    let height = fetch_tiff_tag!(raw, TiffCommonTag::KdcLength).force_usize(0) + 70;
    let offset = fetch_tiff_tag!(raw, TiffCommonTag::KdcOffset);
    if offset.count() < 13 {
      panic!("KDC Decoder: Couldn't find the KDC offset");
    }
    let mut off = offset.force_usize(4) + offset.force_usize(12);

    // Offset hardcoding gotten from dcraw
    if self.camera.find_hint("easyshare_offset_hack") {
      off = if off < 0x15000 { 0x15000 } else { 0x17000 };
    }

    let src = file.subview_until_eof(off as u64).unwrap();
    let image = decode_12be(&src, width, height, dummy);
    let cpp = 1;
    ok_image(self.camera.clone(), width, height, cpp, self.get_wb()?, image.into_inner())
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

impl<'a> KdcDecoder<'a> {
  fn get_wb(&self) -> Result<[f32; 4]> {
    match self.makernote.get_entry(TiffCommonTag::KdcWB) {
      Some(levels) => {
        if levels.count() != 3 {
          Err(RawlerError::General("KDC: Levels count is off".to_string()))
        } else {
          Ok([levels.force_f32(0), levels.force_f32(1), levels.force_f32(2), NAN])
        }
      }
      None => {
        let levels = fetch_tiff_tag!(self.makernote, TiffCommonTag::KodakWB);
        if levels.count() != 734 && levels.count() != 1502 {
          Err(RawlerError::General("KDC: Levels count is off".to_string()))
        } else {
          let r = BEu16(levels.get_data(), 148) as f32;
          let b = BEu16(levels.get_data(), 150) as f32;
          Ok([r / 256.0, 1.0, b / 256.0, NAN])
        }
      }
    }
  }

  pub(crate) fn decode_dc120(src: &[u8], width: usize, height: usize, dummy: bool) -> PixU16 {
    let mut out = alloc_image!(width, height, dummy);

    let mul: [usize; 4] = [162, 192, 187, 92];
    let add: [usize; 4] = [0, 636, 424, 212];
    for row in 0..height {
      let shift = row * mul[row & 3] + add[row & 3];
      for col in 0..width {
        out[row * width + col] = src[row * width + ((col + shift) % 848)] as u16;
      }
    }

    PixU16::new_with(out, width, height)
  }
}
