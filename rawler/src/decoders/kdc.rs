use log::warn;

use crate::RawImage;
use crate::RawLoader;
use crate::RawSource;
use crate::RawlerError;
use crate::Result;
use crate::alloc_image;
use crate::alloc_image_ok;
use crate::analyze::FormatDump;
use crate::bits::BEu16;
use crate::exif::Exif;
use crate::formats::tiff::Entry;
use crate::formats::tiff::GenericTiffReader;
use crate::formats::tiff::IFD;
use crate::formats::tiff::Value;
use crate::formats::tiff::ifd::OffsetMode;
use crate::formats::tiff::reader::TiffReader;
use crate::packed::decode_12be;
use crate::pixarray::PixU16;
use crate::tags::ExifTag;
use crate::tags::TiffCommonTag;

use super::CFAConfig;
use super::Camera;
use super::Decoder;
use super::FormatHint;
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
  makernote: Option<IFD>,
  camera: Camera,
}

impl<'a> KdcDecoder<'a> {
  pub fn new(file: &RawSource, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<KdcDecoder<'a>> {
    let camera = rawloader.check_supported(tiff.root_ifd())?;

    let makernote = if let Some(exif) = tiff.find_first_ifd_with_tag(ExifTag::MakerNotes) {
      exif.parse_makernote(&mut file.reader(), OffsetMode::Absolute, &[])?
    } else {
      warn!("KDC makernote not found");
      None
    };

    Ok(KdcDecoder {
      tiff,
      rawloader,
      camera,
      makernote,
    })
  }
}

impl<'a> Decoder for KdcDecoder<'a> {
  fn raw_image(&self, file: &RawSource, _params: &RawDecodeParams, dummy: bool) -> Result<RawImage> {
    if self.camera.clean_model == "DC120" {
      let width = 848;
      let height = 976;
      let raw = self.tiff.find_ifds_with_tag(TiffCommonTag::CFAPattern)[0];
      let off = fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_usize(0);
      let mut white = self.camera.whitelevel.clone().expect("KDC needs a whitelevel in camera config")[0];
      let src = file.subview_until_eof(off as u64)?;
      let image = match fetch_tiff_tag!(raw, TiffCommonTag::Compression).force_usize(0) {
        1 => Self::decode_dc120(src, width, height, dummy),
        7 => {
          white = 0xFF << 1;
          Self::decode_dc120_jpeg(src, width, height, dummy)?
        }
        c => {
          return Err(RawlerError::unsupported(
            &self.camera,
            format!("KDC: DC120: Don't know how to handle compression type {}", c),
          ));
        }
      };
      let cpp = 1;
      let whitelevel = Some(WhiteLevel::new(vec![white; cpp]));
      let photometric = RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&self.camera));
      let img = RawImage::new(self.camera.clone(), image, cpp, [1.0, 1.0, 1.0, f32::NAN], photometric, None, whitelevel, dummy);
      return Ok(img);
    }

    if self.camera.clean_model == "DC50" {
      let raw = self.tiff.find_ifds_with_tag(TiffCommonTag::CFAPattern)[0];
      let width = self.camera.raw_width;
      let height = self.camera.raw_height;
      let off = fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_usize(0);
      let white = self.camera.whitelevel.clone().expect("KDC needs a whitelevel in camera config")[0];
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
      let photometric = RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&self.camera));
      let img = RawImage::new(self.camera.clone(), image, cpp, [1.0, 1.0, 1.0, f32::NAN], photometric, None, whitelevel, dummy);
      return Ok(img);
    }

    let raw = self
      .tiff
      .find_first_ifd_with_tag(TiffCommonTag::KdcWidth)
      .ok_or_else(|| RawlerError::DecoderFailed(format!("Failed to find a IFD with KdcWidth tag")))?;

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

    let src = file.subview_until_eof(off as u64)?;
    let image = decode_12be(src, width, height, dummy);
    let cpp = 1;
    ok_cfa_image(self.camera.clone(), cpp, self.get_wb()?, image, dummy)
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
    FormatHint::KDC
  }
}

impl<'a> KdcDecoder<'a> {
  fn get_wb(&self) -> Result<[f32; 4]> {
    if let Some(makernote) = self.makernote.as_ref() {
      match makernote.get_entry(TiffCommonTag::KdcWB) {
        Some(levels) => {
          if levels.count() != 3 {
            Err(format!("KDC: Levels count is off: {}", levels.count()).into())
          } else {
            Ok([levels.force_f32(0), levels.force_f32(1), levels.force_f32(2), f32::NAN])
          }
        }
        None => {
          let levels = fetch_tiff_tag!(makernote, TiffCommonTag::KodakWB);
          if ![734, 1502, 1512, 2288].contains(&levels.count()) {
            Err(format!("KDC: Levels count is off: {}", levels.count()).into())
          } else {
            let r = BEu16(levels.get_data(), 148) as f32;
            let b = BEu16(levels.get_data(), 150) as f32;
            Ok([r / 256.0, 1.0, b / 256.0, f32::NAN])
          }
        }
      }
    } else {
      Ok([f32::NAN, f32::NAN, f32::NAN, f32::NAN])
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

    let img = image::load_from_memory_with_format(&swapped_src, image::ImageFormat::Jpeg)
      .map_err(|err| RawlerError::DecoderFailed(format!("Failed to read JPEG image: {:?}", err)))?;

    assert_eq!(width, img.width() as usize);
    assert_eq!(height, img.height() as usize * 2);
    let buf = img.as_flat_samples_u8().unwrap();
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
