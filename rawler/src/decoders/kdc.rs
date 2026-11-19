use log::warn;

use crate::RawImage;
use crate::RawLoader;
use crate::RawSource;
use crate::RawlerError;
use crate::Result;
use crate::alloc_image_ok;
use crate::analyze::FormatDump;
use crate::decompressors::packed::decompress_12be;
use crate::exif::Exif;
use crate::formats::tiff::Entry;
use crate::formats::tiff::GenericTiffReader;
use crate::formats::tiff::IFD;
use crate::formats::tiff::Value;
use crate::formats::tiff::reader::TiffReader;
use crate::pixarray::PixU16;
use crate::tags::ExifTag;
use crate::tags::TiffCommonTag;

use super::CFAConfig;
use super::CameraLookup;
use super::Decoder;
use super::FormatHint;
use super::Orientation;
use super::RawDecodeParams;
use super::RawMetadata;
use super::RawPhotometricInterpretation;
use super::WellKnownIFD;
use super::WhiteLevel;
use super::image_bytes;
use super::makernotes::{MakernoteSection, VirtualMakernotes};
use super::ok_cfa_image;
use super::tiff_ifd;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct KdcDecoder<'a> {
  #[allow(unused)]
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
  kdc_ifd: Option<Arc<IFD>>,
  camera: CameraLookup,
}

impl<'a> KdcDecoder<'a> {
  pub fn new(file: &RawSource, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<KdcDecoder<'a>> {
    let camera = rawloader.lookup_camera(tiff.root_ifd())?;

    let root = tiff.root_ifd();
    let kdc_ifd = match root.get_entry(TiffCommonTag::KdcIFD) {
      Some(entry) => Some(IFD::new(&mut file.reader(), entry.force_u32(0), root.base, root.corr, root.endian, &[])?),
      None => None,
    };

    Ok(KdcDecoder {
      tiff,
      rawloader,
      camera,
      kdc_ifd: kdc_ifd.map(Arc::new),
    })
  }
}

impl<'a> Decoder for KdcDecoder<'a> {
  fn raw_image(&self, file: &RawSource, _params: &RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let camera = self.camera.supported()?;
    if camera.clean_model == "DC120" {
      let width = 848;
      let height = 976;
      let raw = self.get_raw_ifd()?;
      let off = fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_usize(0);
      let src = file.subview_until_eof(off as u64)?;
      let (image, white) = match fetch_tiff_tag!(raw, TiffCommonTag::Compression).force_usize(0) {
        1 => (Self::decode_dc120(src, width, height, dummy)?, 0xFF),
        7 => (Self::decode_dc120_jpeg(src, width, height, dummy)?, 0xFF << 1),
        c => {
          return Err(RawlerError::unsupported(
            camera,
            format!("KDC: DC120: Don't know how to handle compression type {}", c),
          ));
        }
      };
      let cpp = 1;
      let whitelevel = Some(WhiteLevel::new(vec![white; cpp]));
      let photometric = RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(camera));
      // The as shot white balance is not known
      let mut img = RawImage::new(camera.clone(), image, cpp, [f32::NAN; 4], photometric, None, whitelevel, dummy);
      img.orientation = Orientation::from_tiff(self.tiff.root_ifd());
      return Ok(img);
    }

    if camera.clean_model == "DC50" {
      let raw = self.get_raw_ifd()?;
      let width = camera.raw_width;
      let height = camera.raw_height;
      let off = fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_usize(0);
      let white = camera
        .whitelevel
        .as_ref()
        .and_then(|levels| levels.first().copied())
        .ok_or_else(|| decoder_error!("KDC DC50: no white level in the camera definition"))?;
      let cbpp = match raw.get_entry(ExifTag::CompressedBitsPerPixel) {
        Some(Entry {
          value: Value::Rational(data), ..
        }) if data.first().is_some_and(|bits| bits.n == 243) => 2,
        _ => 3,
      };
      let src = file.subview_until_eof_padded(off as u64)?;
      let image = crate::decompressors::radc::decompress(&src, width, height, cbpp, dummy)?;
      let cpp = 1;
      let whitelevel = Some(WhiteLevel::new(vec![white; cpp]));
      let photometric = RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(camera));
      let mut img = RawImage::new(camera.clone(), image, cpp, [1.0, 1.0, 1.0, f32::NAN], photometric, None, whitelevel, dummy);
      img.orientation = Orientation::from_tiff(self.tiff.root_ifd());
      return Ok(img);
    }

    let raw = self.get_raw_ifd()?;

    let width = fetch_tiff_tag!(raw, TiffCommonTag::KdcWidth).force_usize(0) + 80;
    let height = fetch_tiff_tag!(raw, TiffCommonTag::KdcLength).force_usize(0) + 70;
    let offset = fetch_tiff_tag!(raw, TiffCommonTag::KdcOffset);
    if offset.count() < 13 {
      return Err(RawlerError::decoder_failed("KDC Decoder: Couldn't find the KDC offset".to_string()));
    }
    let mut off = offset.force_usize(4) + offset.force_usize(12);

    // Offset hardcoding gotten from dcraw
    if camera.find_hint("easyshare_offset_hack") {
      off = if off < 0x15000 { 0x15000 } else { 0x17000 };
    }

    image_bytes("KDC", width, height, 12)?;
    let src = file.subview_until_eof(off as u64)?;
    let image = decompress_12be(src, width, height, dummy).map_err(RawlerError::decoder_failed)?;
    let cpp = 1;
    let mut img = ok_cfa_image(camera.clone(), cpp, self.get_wb()?, image, dummy)?;
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
      WellKnownIFD::Makernote => self.kdc_ifd.clone(),
      WellKnownIFD::VirtualMakernotes => {
        let mut container = VirtualMakernotes::new(self.format_hint());
        container.add_exif_makernote(self.tiff.find_first_ifd_with_tag(ExifTag::MakerNotes), None);
        if let (Some(entry), Some(kdc_ifd)) = (self.tiff.root_ifd().get_entry(TiffCommonTag::KdcIFD), &self.kdc_ifd) {
          container.add_pointer_ifd(MakernoteSection::KdcIFD, entry.force_u32(0), IFD::clone(kdc_ifd));
        }
        container.build().map(Arc::new)
      }
      _ => None,
    })
  }

  fn format_hint(&self) -> FormatHint {
    FormatHint::KDC
  }
}

impl<'a> KdcDecoder<'a> {
  /// The IFD with the raw image data: the one with CFAPattern for the DC120 and
  /// DC50, the one with KdcWidth for later models
  fn get_raw_ifd(&self) -> Result<&IFD> {
    match self.camera.metadata().clean_model.as_str() {
      model @ ("DC120" | "DC50") => self
        .tiff
        .find_first_ifd_with_tag(TiffCommonTag::CFAPattern)
        .ok_or_else(|| RawlerError::decoder_failed(format!("KDC {}: no IFD with CFAPattern", model))),
      _ => self
        .tiff
        .find_first_ifd_with_tag(TiffCommonTag::KdcWidth)
        .ok_or_else(|| decoder_error!("Failed to find a IFD with KdcWidth tag")),
    }
  }

  fn get_wb(&self) -> Result<[f32; 4]> {
    match self.kdc_ifd.as_ref() {
      Some(ifd) => Self::as_shot_wb(ifd),
      None => {
        warn!("KDC IFD not found, the white balance is unknown");
        Ok([f32::NAN, f32::NAN, f32::NAN, f32::NAN])
      }
    }
  }

  /// As shot white balance from the KDC IFD. The file has levels for each white
  /// balance mode, the mode of the shot selects them.
  fn as_shot_wb(kdc_ifd: &IFD) -> Result<[f32; 4]> {
    let mode = fetch_tiff_tag!(kdc_ifd, TiffCommonTag::KdcWBMode).force_u16(0);
    let levels = fetch_tiff_tag!(kdc_ifd, Self::wb_levels_tag(mode));
    if levels.count() != 3 {
      return Err(decoder_error!("KDC: Levels count is off: {}", levels.count()));
    }
    let levels = [0, 1, 2].map(|i| levels.force_f32(i));
    if !levels.iter().all(|level| level.is_finite() && *level > 0.0) {
      warn!("KDC: invalid white balance levels {:?}, the white balance is unknown", levels);
      return Ok([f32::NAN; 4]);
    }
    let [red, green, blue] = levels;
    Ok([red / green, 1.0, blue / green, f32::NAN])
  }

  /// Tag with the white balance levels of a white balance mode. Other modes than
  /// the known ones use the levels of the auto mode.
  fn wb_levels_tag(mode: u16) -> TiffCommonTag {
    match mode {
      1 => TiffCommonTag::KdcWBFluorescent,
      2 => TiffCommonTag::KdcWBTungsten,
      3 => TiffCommonTag::KdcWBDaylight,
      6 => TiffCommonTag::KdcWBShade,
      _ => TiffCommonTag::KdcWBAuto,
    }
  }

  pub(crate) fn decode_dc120(src: &[u8], width: usize, height: usize, dummy: bool) -> Result<PixU16> {
    let mut out = alloc_image_ok!(width, height, dummy);

    let mul: [usize; 4] = [162, 192, 187, 92];
    let add: [usize; 4] = [0, 636, 424, 212];
    for row in 0..height {
      let shift = row * mul[row & 3] + add[row & 3];
      for col in 0..width {
        let pixel = src
          .get(row * width + ((col + shift) % 848))
          .ok_or_else(|| decoder_error!("KDC DC120: the data ends in row {}", row))?;
        out[row * width + col] = *pixel as u16;
      }
    }
    Ok(out)
  }

  pub(crate) fn decode_dc120_jpeg(src: &[u8], width: usize, height: usize, dummy: bool) -> Result<PixU16> {
    let mut out = alloc_image_ok!(width, height, dummy);

    let swapped_src: Vec<u8> = src.as_chunks::<2>().0.iter().flat_map(|x| [x[1], x[0]]).collect();

    let img =
      image::load_from_memory_with_format(&swapped_src, image::ImageFormat::Jpeg).map_err(|err| decoder_error!("Failed to read JPEG image: {:?}", err))?;

    if width != img.width() as usize || height != img.height() as usize * 2 {
      return Err(decoder_error!(
        "KDC DC120: JPEG of {}x{} pixels for an image of {}x{} pixels",
        img.width(),
        img.height(),
        width,
        height
      ));
    }
    // The pixels are read as RGB, other JPEGs are converted
    let img = img.into_rgb8();
    let jpeg = img.as_raw();

    for irow in 0..img.height() as usize {
      let row = irow * 2;
      let iline = &jpeg[irow * width * 3..];
      for col in (0..width).step_by(2) {
        *out.at_mut(row + 0, col + 0) = (iline[col * 3 + 1] as u16) << 1;
        *out.at_mut(row + 1, col + 1) = (iline[(col + 1) * 3 + 1] as u16) << 1;
        *out.at_mut(row + 0, col + 1) = (iline[col * 3 + 0]) as u16 + (iline[(col + 1) * 3 + 0]) as u16;
        *out.at_mut(row + 1, col + 0) = (iline[col * 3 + 2]) as u16 + (iline[(col + 1) * 3 + 2]) as u16;
      }
    }

    Ok(out)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn dc120_with_short_data_is_an_error() {
    assert!(KdcDecoder::decode_dc120(&[0; 1000], 848, 976, false).is_err());
    assert!(KdcDecoder::decode_dc120(&vec![0; 848 * 976], 848, 976, false).is_ok());
  }

  #[test]
  fn dc120_jpeg_of_another_size_is_an_error() {
    let mut jpeg = std::io::Cursor::new(Vec::new());
    image::RgbImage::new(16, 8).write_to(&mut jpeg, image::ImageFormat::Jpeg).expect("JPEG");
    // The JPEG data has swapped bytes
    let swapped: Vec<u8> = jpeg.into_inner().chunks(2).flat_map(|x| x.iter().rev().copied().collect::<Vec<u8>>()).collect();
    assert!(KdcDecoder::decode_dc120_jpeg(&swapped, 16, 16, false).is_ok());
    assert!(KdcDecoder::decode_dc120_jpeg(&swapped, 16, 32, false).is_err());
  }
}
