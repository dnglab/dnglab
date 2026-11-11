use std::cmp;

use crate::RawImage;
use crate::RawLoader;
use crate::Result;
use crate::alloc_image_ok;
use crate::analyze::FormatDump;
use crate::bits::BEu16;
use crate::bits::Endian;
use crate::bits::LookupTable;
use crate::exif::Exif;
use crate::formats::tiff::GenericTiffReader;
use crate::formats::tiff::IFD;
use crate::formats::tiff::SRational;
use crate::formats::tiff::Value;
use crate::formats::tiff::reader::TiffReader;
use crate::pixarray::PixU16;
use crate::pumps::ByteStream;
use crate::pumps::ByteStreamError;
use crate::rawsource::RawSource;
use crate::tags::TiffCommonTag;

use super::BlackLevel;
use super::CameraLookup;
use super::Decoder;
use super::FormatHint;
use super::Orientation;
use super::RawDecodeParams;
use super::RawMetadata;
use super::ok_cfa_image;

#[derive(Debug, Clone)]
pub struct DcrDecoder<'a> {
  #[allow(unused)]
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
  makernote: IFD,
  camera: CameraLookup,
}

impl<'a> DcrDecoder<'a> {
  pub fn new(file: &RawSource, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<DcrDecoder<'a>> {
    let camera = rawloader.lookup_camera(tiff.root_ifd())?;

    let kodak_ifd = fetch_tiff_tag!(tiff, TiffCommonTag::KodakIFD);
    let makernote = IFD::new(&mut file.reader(), kodak_ifd.force_u32(0), 0, 0, tiff.get_endian(), &[])?;

    Ok(DcrDecoder {
      tiff,
      rawloader,
      camera,
      makernote,
    })
  }
}

impl<'a> Decoder for DcrDecoder<'a> {
  fn raw_image(&self, file: &RawSource, _params: &RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let camera = self.camera.supported()?;
    let raw = self
      .tiff
      .find_first_ifd_with_tag(TiffCommonTag::CFAPattern)
      .ok_or_else(|| decoder_error!("Failed to find a IFD with CFAPattern tag"))?;
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let offset = fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_usize(0);

    let src = file.subview_until_eof_padded(offset as u64)?; // TODO add size and check all samples

    let linearization = fetch_tiff_tag!(self.makernote, TiffCommonTag::DcrLinearization);
    let curve = {
      let mut points = Vec::new();
      for i in 0..linearization.count() {
        points.push(linearization.force_u32(i) as u16);
      }
      LookupTable::new(&points)
    };

    let image = DcrDecoder::decode_kodak65000(&src, &curve, width, height, dummy)?;

    let cpp = 1;
    let wb = kodak_wb(&self.makernote).unwrap_or([f32::NAN, f32::NAN, f32::NAN, f32::NAN]);
    let mut img = ok_cfa_image(camera.clone(), cpp, wb, image, dummy)?;
    img.orientation = Orientation::from_tiff(self.tiff.root_ifd());
    let rows = img.active_area.map_or(img.height, |area| area.d.h);
    if let Some(blacklevel) = kodak_blacklevel(&self.makernote, linearization, rows, cpp) {
      img.blacklevel = blacklevel;
    }
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

  fn format_hint(&self) -> FormatHint {
    FormatHint::DCR
  }
}

/// Black level from the KodakIFD for an image with `rows` rows in the ActiveArea
///
/// The tags have the black levels at the top and the bottom of the image, before
/// `curve` linearizes the values. The level between them changes by row. Tags
/// with zero values don't define a black level.
pub(super) fn kodak_blacklevel(kodak_ifd: &IFD, curve: &Value, rows: usize, cpp: usize) -> Option<BlackLevel> {
  let top = kodak_ifd.get_entry(TiffCommonTag::KodakBlackLevelTop)?.force_usize(0);
  let bottom = kodak_ifd.get_entry(TiffCommonTag::KodakBlackLevelBottom)?.force_usize(0);
  if top == 0 && bottom == 0 {
    return None;
  }
  let (top, bottom) = (curve.force_u16(top) as i32, curve.force_u16(bottom) as i32);
  let delta_v = if top != bottom && rows > 1 {
    (0..rows).map(|row| SRational::new((bottom - top) * row as i32, rows as i32 - 1)).collect()
  } else {
    Vec::new()
  };
  Some(BlackLevel::new(&[top as u32], 1, 1, cpp).with_deltas(Vec::new(), delta_v))
}

/// As shot white balance from the KodakIFD: the levels set in software or else
/// the levels of the white balance mode of the shot. The white balance
/// coefficients are the reciprocal of the levels.
pub(super) fn kodak_wb(kodak_ifd: &IFD) -> Option<[f32; 4]> {
  if let Some(levels) = kodak_ifd.get_entry(TiffCommonTag::DcrWB).filter(|levels| levels.count() == 72) {
    let levels = levels.get_data();
    return Some([
      2048.0 / BEu16(levels, 40) as f32,
      2048.0 / BEu16(levels, 42) as f32,
      2048.0 / BEu16(levels, 44) as f32,
      f32::NAN,
    ]);
  }
  let tag = match kodak_ifd.get_entry(TiffCommonTag::KodakWhiteBalance)?.force_u16(0) {
    0 => TiffCommonTag::KodakWBDaylight,
    1 => TiffCommonTag::KodakWBTungsten,
    2 => TiffCommonTag::KodakWBFluorescent,
    3 => TiffCommonTag::KodakWBFlash,
    4 => TiffCommonTag::KodakWBCustom,
    5 => TiffCommonTag::KodakWBAuto,
    _ => return None,
  };
  let levels = kodak_ifd.get_entry(tag).filter(|levels| levels.count() == 3)?;
  let [r, g, b] = [0, 1, 2].map(|i| levels.force_f32(i));
  (r > 0.0 && g > 0.0 && b > 0.0).then_some([g / r, 1.0, g / b, f32::NAN])
}

impl<'a> DcrDecoder<'a> {
  pub(crate) fn decode_kodak65000(buf: &[u8], curve: &LookupTable, width: usize, height: usize, dummy: bool) -> Result<PixU16> {
    let mut out = alloc_image_ok!(width, height, dummy);
    let mut input = ByteStream::new(buf, Endian::Little);

    let mut random: u32 = 0;
    for row in 0..height {
      for col in (0..width).step_by(256) {
        let mut pred: [i32; 2] = [0; 2];
        let buf = DcrDecoder::decode_segment(&mut input, cmp::min(256, width - col))?;
        for (i, val) in buf.iter().enumerate() {
          pred[i & 1] += *val;
          if pred[i & 1] < 0 {
            log::warn!("DCR Decoder: Found a negative pixel value {}, clamping to 0", pred[i & 1]);
            pred[i & 1] = 0;
          }
          out[row * width + col + i] = curve.dither(pred[i & 1] as u16, &mut random);
        }
      }
    }

    Ok(out)
  }

  fn decode_segment(input: &mut ByteStream, size: usize) -> std::result::Result<Vec<i32>, ByteStreamError> {
    let mut out: Vec<i32> = vec![0; size];

    let mut lens: [usize; 256] = [0; 256];
    for i in (0..size).step_by(2) {
      lens[i] = (input.peek_u8()? & 15) as usize;
      lens[i + 1] = (input.get_u8()? >> 4) as usize;
    }

    let mut bitbuf: u64 = 0;
    let mut bits: usize = 0;
    if (size & 7) == 4 {
      bitbuf = ((input.get_u8()? as u64) << 8) | (input.get_u8()? as u64);
      bits = 16;
    }

    for i in 0..size {
      let len = lens[i];
      if bits < len {
        for j in (0..32).step_by(8) {
          bitbuf += (input.get_u8()? as u64) << (bits + (j ^ 8));
        }
        bits += 32;
      }
      out[i] = (bitbuf & (0xffff >> (16 - len))) as i32;
      bitbuf >>= len;
      bits -= len;
      if len != 0 && (out[i] & (1 << (len - 1))) == 0 {
        out[i] -= (1 << len) - 1;
      }
    }

    Ok(out)
  }
}
