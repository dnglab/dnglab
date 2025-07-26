use log::warn;

use crate::RawlerError;
use crate::analyze::FormatDump;
use crate::rawsource::RawSource;

use crate::RawImage;
use crate::RawLoader;
use crate::Result;
use crate::bits::BEu16;
use crate::exif::Exif;
use crate::formats::tiff::GenericTiffReader;
use crate::formats::tiff::IFD;
use crate::formats::tiff::ifd::OffsetMode;
use crate::formats::tiff::reader::TiffReader;
use crate::packed::decode_12be_wcontrol;
use crate::tags::ExifTag;
use crate::tags::TiffCommonTag;

use super::BlackLevel;
use super::CFAConfig;
use super::Camera;
use super::Decoder;
use super::FormatHint;
use super::RawDecodeParams;
use super::RawMetadata;
use super::RawPhotometricInterpretation;

#[derive(Debug, Clone)]
pub struct ErfDecoder<'a> {
  #[allow(unused)]
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
  makernote: IFD,
  camera: Camera,
}

impl<'a> ErfDecoder<'a> {
  pub fn new(file: &RawSource, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<ErfDecoder<'a>> {
    let camera = rawloader.check_supported(tiff.root_ifd())?;

    let makernote = if let Some(exif) = tiff.find_first_ifd_with_tag(ExifTag::MakerNotes) {
      exif.parse_makernote(&mut file.reader(), OffsetMode::Absolute, &[])?
    } else {
      warn!("ERF makernote not found");
      None
    }
    .ok_or("File has not makernotes")?;

    Ok(ErfDecoder {
      tiff,
      rawloader,
      camera,
      makernote,
    })
  }
}

impl<'a> Decoder for ErfDecoder<'a> {
  fn raw_image(&self, file: &RawSource, _params: &RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let raw = self
      .tiff
      .find_first_ifd_with_tag(TiffCommonTag::CFAPattern)
      .ok_or_else(|| RawlerError::DecoderFailed(format!("Failed to find a IFD with CFAPattern tag")))?;
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let offset = fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_usize(0);
    let src = file.subview_until_eof(offset as u64)?;

    let image = decode_12be_wcontrol(src, width, height, dummy);
    let cpp = 1;

    let blacklevel = self.get_blacklevel(cpp);
    let photometric = RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&self.camera));
    let img = RawImage::new(self.camera.clone(), image, cpp, self.get_wb()?, photometric, blacklevel, None, dummy);
    Ok(img)
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
    FormatHint::ERF
  }
}

impl<'a> ErfDecoder<'a> {
  fn get_wb(&self) -> Result<[f32; 4]> {
    let levels = fetch_tiff_tag!(self.makernote, TiffCommonTag::EpsonWB);
    if levels.count() != 256 {
      Err(RawlerError::DecoderFailed("ERF: Levels count is off".to_string()))
    } else {
      let r = BEu16(levels.get_data(), 48) as f32;
      let b = BEu16(levels.get_data(), 50) as f32;
      Ok([r * 508.0 * 1.078 / 65536.0, 1.0, b * 382.0 * 1.173 / 65536.0, f32::NAN])
    }
  }

  fn get_blacklevel(&self, cpp: usize) -> Option<BlackLevel> {
    if let Some(levels) = self.makernote.get_entry(0x0401) {
      let levels = [levels.force_u16(0), levels.force_u16(1), levels.force_u16(2), levels.force_u16(3)];
      return Some(BlackLevel::new(&levels, self.camera.cfa.width, self.camera.cfa.height, cpp));
    }
    None
  }
}
