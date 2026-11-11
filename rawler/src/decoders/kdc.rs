use log::warn;

use crate::RawImage;
use crate::RawLoader;
use crate::RawSource;
use crate::RawlerError;
use crate::Result;
use crate::alloc_image;
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
use super::WhiteLevel;
use super::ok_cfa_image;

#[derive(Debug, Clone)]
pub struct KdcDecoder<'a> {
  #[allow(unused)]
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
  kdc_ifd: Option<IFD>,
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
      kdc_ifd,
    })
  }
}

impl<'a> Decoder for KdcDecoder<'a> {
  fn raw_image(&self, file: &RawSource, _params: &RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let camera = self.camera.supported()?;
    if camera.clean_model == "DC120" {
      let width = 848;
      let height = 976;
      let raw = self
        .tiff
        .find_ifds_with_tag(TiffCommonTag::CFAPattern)
        .into_iter()
        .next()
        .ok_or_else(|| RawlerError::decoder_failed("KDC DC120: no IFD with CFAPattern".to_string()))?;
      let off = fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_usize(0);
      let src = file.subview_until_eof(off as u64)?;
      let (image, white) = match fetch_tiff_tag!(raw, TiffCommonTag::Compression).force_usize(0) {
        1 => (Self::decode_dc120(src, width, height, dummy), 0xFF),
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
      let raw = self
        .tiff
        .find_ifds_with_tag(TiffCommonTag::CFAPattern)
        .into_iter()
        .next()
        .ok_or_else(|| RawlerError::decoder_failed("KDC DC50: no IFD with CFAPattern".to_string()))?;
      let width = camera.raw_width;
      let height = camera.raw_height;
      let off = fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_usize(0);
      let white = camera.whitelevel.clone().expect("KDC needs a whitelevel in camera config")[0];
      let cbpp = match raw.get_entry(ExifTag::CompressedBitsPerPixel) {
        Some(Entry {
          value: Value::Rational(data), ..
        }) if data[0].n == 243 => 2,
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

    let raw = self
      .tiff
      .find_first_ifd_with_tag(TiffCommonTag::KdcWidth)
      .ok_or_else(|| decoder_error!("Failed to find a IFD with KdcWidth tag"))?;

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

  fn format_hint(&self) -> FormatHint {
    FormatHint::KDC
  }
}

impl<'a> KdcDecoder<'a> {
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
    let green = levels.force_f32(1);
    Ok([levels.force_f32(0) / green, 1.0, levels.force_f32(2) / green, f32::NAN])
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
    out
  }

  pub(crate) fn decode_dc120_jpeg(src: &[u8], width: usize, height: usize, dummy: bool) -> Result<PixU16> {
    let mut out = alloc_image_ok!(width, height, dummy);

    let swapped_src: Vec<u8> = src.chunks_exact(2).flat_map(|x| [x[1], x[0]]).collect();

    let img =
      image::load_from_memory_with_format(&swapped_src, image::ImageFormat::Jpeg).map_err(|err| decoder_error!("Failed to read JPEG image: {:?}", err))?;

    assert_eq!(width, img.width() as usize);
    assert_eq!(height, img.height() as usize * 2);
    let buf = img
      .as_flat_samples_u8()
      .ok_or_else(|| RawlerError::decoder_failed("KDC: failed to get u8 samples from JPEG"))?;
    let jpeg = buf.as_slice();

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
