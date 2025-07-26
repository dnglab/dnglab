use std::cmp;
use std::io::Read;
use std::io::Seek;

use crate::RawImage;
use crate::RawLoader;
use crate::RawlerError;
use crate::Result;
use crate::alloc_image;
use crate::analyze::FormatDump;
use crate::buffer::PaddedBuf;
use crate::exif::Exif;
use crate::formats::tiff::Entry;
use crate::formats::tiff::GenericTiffReader;
use crate::formats::tiff::IFD;
use crate::formats::tiff::Rational;
use crate::formats::tiff::Value;
use crate::formats::tiff::reader::TiffReader;
use crate::imgop::Dim2;
use crate::imgop::Point;
use crate::imgop::Rect;
use crate::lens::LensDescription;
use crate::lens::LensResolver;
use crate::packed::*;
use crate::pixarray::PixU16;
use crate::pumps::BitPump;
use crate::pumps::BitPumpMSB;
use crate::rawimage::CFAConfig;
use crate::rawimage::RawPhotometricInterpretation;
use crate::rawsource::RawSource;
use crate::tags::ExifTag;
use crate::tags::TiffCommonTag;

use super::BlackLevel;
use super::Camera;
use super::Decoder;
use super::FormatHint;
use super::RawDecodeParams;
use super::RawMetadata;

const MFT_MOUNT: &str = "MFT-mount";

#[derive(Debug, Clone)]
pub struct OrfDecoder<'a> {
  #[allow(unused)]
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
  camera: Camera,
  makernote: IFD,
}

pub fn parse_makernote<R: Read + Seek>(reader: &mut R, exif_ifd: &IFD) -> Result<Option<IFD>> {
  if let Some(exif) = exif_ifd.get_entry(ExifTag::MakerNotes) {
    let offset = exif.offset().unwrap() as u32;
    log::debug!("Makernote offset: {}", offset);
    match &exif.value {
      Value::Undefined(data) => {
        let mut off = 0;
        // Olympus starts the makernote with their own name, sometimes truncated
        if data[0..5] == b"OLYMP"[..] {
          off += 8;
          if data[0..7] == b"OLYMPUS"[..] {
            off += 4;
          }
        }
        // OM Digital Solutions put their name in front of the TIFF structure, too
        if data[0..9] == b"OM SYSTEM"[..] {
          off += 16;
          assert_eq!(data[12..14], b"II"[..]);
        }
        let endian = exif_ifd.endian;
        //assert!(data[off..off + 2] == b"II"[..] || data[off..off + 2] == b"MM"[..], "ORF: must contain endian marker in makernote IFD");
        //let endian = if data[off..off + 2] == b"II"[..] { Endian::Little } else { Endian::Big };
        //off += 4;

        let mut mainifd = IFD::new(reader, offset + off as u32, exif_ifd.base, exif_ifd.corr, endian, &[0x3000])?;

        // Parse the Olympus Equipment section if it exists
        if let Some(entry) = mainifd.get_entry_raw_with_len(OrfMakernotes::EquipmentIFD, reader, 4)? {
          // The entry is of type UNDEFINED and count = 1. This tag contains a single 32 bit
          // offset to the IFD.
          let ioff = entry.get_force_u32(0);
          log::debug!("Found EquipmentIFD at offset: {}", ioff);
          // The IFD start at offset+ioff, but all offsets inside the IFD a relative to the main makernote IFD offset.
          // So we use the main IFD as base offset, but start parsing IFD at ioff.
          let ifd = IFD::new(reader, ioff, offset, 0, endian, &[])?;
          mainifd.sub.insert(OrfMakernotes::EquipmentIFD.into(), vec![ifd]);
        }

        // For Olympus or OM-System models
        if off == 12 || off == 16 {
          // Parse the Olympus ImgProc section if it exists
          let ioff = if let Some(entry) = mainifd.get_entry_raw_with_len(OrfMakernotes::ImageProcessingIFD, reader, 4)? {
            // The entry is of type UNDEFINED and count = 1. This tag contains a single 32 bit
            // offset to the IFD.
            entry.get_force_u32(0)
          } else {
            0
          };
          if ioff != 0 {
            log::debug!("Found ImageIFD at offset: {}", ioff);
            // The IFD start at offset+ioff, but all offsets inside the IFD a relative to the main makernote IFD offset.
            // So we use the main IFD as base offset, but start parsing IFD at ioff.
            let iprocifd = IFD::new(reader, ioff, offset, 0, endian, &[])?;
            mainifd.sub.insert(OrfMakernotes::ImageProcessingIFD.into(), vec![iprocifd]);
          } else {
            log::debug!("ORF ImageIFD not found");
          }
        }
        Ok(Some(mainifd))
      }
      _ => Err(RawlerError::DecoderFailed("EXIF makernote has unknown type".to_string())),
    }
  } else {
    Ok(None)
  }
}

impl<'a> OrfDecoder<'a> {
  pub fn new(file: &RawSource, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<OrfDecoder<'a>> {
    let camera = rawloader.check_supported(tiff.root_ifd())?;

    let makernote = if let Some(exif) = tiff.find_first_ifd_with_tag(ExifTag::MakerNotes) {
      parse_makernote(&mut file.reader(), exif)?
    } else {
      log::warn!("ORF makernote not found");
      None
    }
    .ok_or("File has not makernotes")?;

    //makernote.dump::<ExifTag>(0).iter().for_each(|line| eprintln!("DUMP: {}", line));

    Ok(OrfDecoder {
      tiff,
      rawloader,
      camera,
      makernote,
    })
  }
}

impl<'a> Decoder for OrfDecoder<'a> {
  fn raw_image(&self, file: &RawSource, _params: &RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let raw = self
      .tiff
      .find_first_ifd_with_tag(TiffCommonTag::StripOffsets)
      .ok_or_else(|| RawlerError::DecoderFailed(format!("Failed to find a IFD with StripOffsets tag")))?;
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let offset = fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_usize(0);
    let counts = fetch_tiff_tag!(raw, TiffCommonTag::StripByteCounts);

    let mut size: usize = 0;
    for i in 0..counts.count() {
      size += counts.force_u32(i as usize) as usize;
    }

    let camera = if width >= self.camera.highres_width {
      self.rawloader.check_supported_with_mode(self.tiff.root_ifd(), "highres")?
    } else {
      self.camera.clone()
    };

    let src = file.subview_padded(offset as u64, size as u64)?; // TODO add size and check all samples

    log::debug!(
      "ORF raw image size: {}, dim: {}x{}, total mp: {}, strip counts: {}",
      size,
      width,
      height,
      width * height,
      counts.count()
    );

    // These conditions are sorted in descending order.
    // All ORF files comes with no hints about the used compression.
    // But we need to differentiate between 12be-interlaced and
    // 12be-msb32 because they are in the same size range.
    let image = if size >= width * height * 2 {
      if self.tiff.little_endian() {
        log::debug!("ORF: decode_12le_unpacked_left_aligned");
        decode_12le_unpacked_left_aligned(&src, width, height, dummy)
      } else {
        log::debug!("ORF: decode_12be_unpacked_left_aligned");
        decode_12be_unpacked_left_aligned(&src, width, height, dummy)
      }
    } else if size >= width * height / 10 * 16 {
      log::debug!("ORF: decode_12le_wcontrol");
      decode_12le_wcontrol(&src, width, height, dummy)
    } else if size >= width * height * 12 / 8 {
      if self.camera.find_hint("interlaced") {
        log::debug!("ORF: decode_12be_interlaced");
        decode_12be_interlaced(&src, width, height, dummy)
      } else {
        log::debug!("ORF: decode_12be_msb32");
        //decode_12be_interlaced(&src, width, height, dummy)
        decode_12be_msb32(&src, width, height, dummy)
      }
    } else {
      log::debug!("ORF: fallback to decode_compressed");
      OrfDecoder::decode_compressed(&src, width, height, dummy)
    };

    let cpp = 1;

    let blacklevel = self.get_blacklevel()?;
    let whitelevel = None;
    let photometric = RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&self.camera));
    let mut img = RawImage::new(camera, image, cpp, normalize_wb(self.get_wb()?), photometric, blacklevel, whitelevel, dummy);
    if let Some(crop) = self.get_crop()? {
      img.crop_area = Some(crop);
    }

    Ok(img)
  }

  fn format_dump(&self) -> FormatDump {
    todo!()
  }

  fn raw_metadata(&self, _file: &RawSource, __params: &RawDecodeParams) -> Result<RawMetadata> {
    let exif = Exif::new(self.tiff.root_ifd())?;
    let mdata = RawMetadata::new_with_lens(&self.camera, exif, self.get_lens_description()?.cloned());
    Ok(mdata)
  }

  fn format_hint(&self) -> FormatHint {
    FormatHint::ORF
  }
}

impl<'a> OrfDecoder<'a> {
  /* This is probably the slowest decoder of them all.
   * I cannot see any way to effectively speed up the prediction
   * phase, which is by far the slowest part of this algorithm.
   * Also there is no way to multithread this code, since prediction
   * is based on the output of all previous pixel (bar the first four)
   */

  pub fn decode_compressed(buf: &PaddedBuf, width: usize, height: usize, dummy: bool) -> PixU16 {
    let mut out = alloc_image!(width, height, dummy);

    /* Build a table to quickly look up "high" value */
    let mut bittable: [u8; 4096] = [0; 4096];
    for i in 0..4096 {
      let mut b = 12;
      for high in 0..12 {
        if ((i >> (11 - high)) & 1) != 0 {
          b = high;
          break;
        }
      }
      bittable[i] = b;
    }

    let mut left: [i32; 2] = [0; 2];
    let mut nw: [i32; 2] = [0; 2];
    let mut pump = BitPumpMSB::new(&buf[7..]);

    for row in 0..height {
      let mut acarry: [[i32; 3]; 2] = [[0; 3]; 2];

      for c in 0..width / 2 {
        let col: usize = c * 2;
        for s in 0..2 {
          // Run twice for odd and even pixels
          let i = if acarry[s][2] < 3 { 2 } else { 0 };
          let mut nbits = 2 + i;
          while ((acarry[s][0] >> (nbits + i)) & 0xffff) > 0 {
            nbits += 1
          }
          nbits = cmp::min(nbits, 16);
          let b = pump.peek_ibits(15);

          let sign: i32 = -(b >> 14);
          let low: i32 = (b >> 12) & 3;
          let mut high: i32 = bittable[(b & 4095) as usize] as i32;

          // Skip bytes used above or read bits
          if high == 12 {
            pump.consume_bits(15);
            high = pump.get_ibits(16 - nbits) >> 1;
          } else {
            pump.consume_bits((high + 4) as u32);
          }

          acarry[s][0] = ((high << nbits) | pump.get_ibits(nbits)) as i32;
          let diff = (acarry[s][0] ^ sign) + acarry[s][1];
          acarry[s][1] = (diff * 3 + acarry[s][1]) >> 5;
          acarry[s][2] = if acarry[s][0] > 16 { 0 } else { acarry[s][2] + 1 };

          if row < 2 || col < 2 {
            // We're in a border, special care is needed
            let pred = if row < 2 && col < 2 {
              // We're in the top left corner
              0
            } else if row < 2 {
              // We're going along the top border
              left[s]
            } else {
              // col < 2, we're at the start of a line
              nw[s] = out[(row - 2) * width + (col + s)] as i32;
              nw[s]
            };
            left[s] = pred + ((diff << 2) | low);
            out[row * width + (col + s)] = left[s] as u16;
          } else {
            let up: i32 = out[(row - 2) * width + (col + s)] as i32;
            let left_minus_nw: i32 = left[s] - nw[s];
            let up_minus_nw: i32 = up - nw[s];
            // Check if sign is different, and one is not zero
            let pred = if left_minus_nw * up_minus_nw < 0 {
              if left_minus_nw.abs() > 32 || up_minus_nw.abs() > 32 {
                left[s] + up_minus_nw
              } else {
                (left[s] + up) >> 1
              }
            } else if left_minus_nw.abs() > up_minus_nw.abs() {
              left[s]
            } else {
              up
            };

            left[s] = pred + ((diff << 2) | low);
            nw[s] = up;
            out[row * width + (col + s)] = left[s] as u16;
          }
        }
      }
    }
    out
  }

  fn get_blacklevel(&self) -> Result<Option<BlackLevel>> {
    let ifd = self.makernote.find_ifds_with_tag(OrfImageProcessing::OrfBlackLevels);
    if ifd.is_empty() {
      log::info!("ORF: Couldn't find ImgProc IFD, unable to read blacklevel");
      return Ok(None);
    }

    let blacks = fetch_tiff_tag!(ifd[0], OrfImageProcessing::OrfBlackLevels);
    let levels = [blacks.force_u16(0), blacks.force_u16(1), blacks.force_u16(2), blacks.force_u16(3)];
    Ok(Some(BlackLevel::new(&levels, self.camera.cfa.width, self.camera.cfa.height, 1)))
  }

  fn get_crop(&self) -> Result<Option<Rect>> {
    let ifd = self.makernote.find_ifds_with_tag(OrfImageProcessing::CropLeft);
    if ifd.is_empty() {
      return Ok(None);
    }
    let crop_left = fetch_tiff_tag!(ifd[0], OrfImageProcessing::CropLeft).force_usize(0);
    let crop_top = fetch_tiff_tag!(ifd[0], OrfImageProcessing::CropTop).force_usize(0);
    let crop_width = fetch_tiff_tag!(ifd[0], OrfImageProcessing::CropWidth).force_usize(0);
    let crop_height = fetch_tiff_tag!(ifd[0], OrfImageProcessing::CropHeight).force_usize(0);
    Ok(Some(Rect::new(Point::new(crop_left, crop_top), Dim2::new(crop_width, crop_height))))
  }

  /// Get lens description by analyzing TIFF tags and makernotes
  fn get_lens_description(&self) -> Result<Option<&'static LensDescription>> {
    if let Some(ifd) = self.makernote.get_sub_ifd(OrfMakernotes::EquipmentIFD) {
      match ifd.get_entry(OrfEquipmentTags::LensType) {
        Some(Entry {
          value: Value::Byte(settings), ..
        }) => {
          log::debug!("Lens type tag: {:?}", settings);
          let make_id = settings[0];
          let model_id = settings[2];
          let submodel_id = settings[3];
          let composite_id = format!("{:02X} {:02X} {:02X}", make_id, model_id, submodel_id);
          log::debug!("ORF lens composite ID: {}", composite_id);
          let resolver = LensResolver::new()
            .with_olympus_id(Some(composite_id))
            .with_camera(&self.camera)
            .with_focal_len(self.get_focal_len()?)
            .with_mounts(&[MFT_MOUNT.into()]);
          return Ok(resolver.resolve());
        }
        _ => {
          log::warn!("Camera settings in makernote not found, no lens data available");
        }
      }
    }
    log::warn!("No lens data found");
    Ok(None)
  }

  fn get_focal_len(&self) -> Result<Option<Rational>> {
    if let Some(exif) = self.tiff.find_first_ifd_with_tag(ExifTag::MakerNotes) {
      if let Some(Entry {
        value: Value::Short(focal), ..
      }) = exif.get_entry(ExifTag::FocalLength)
      {
        return Ok(focal.get(1).map(|v| Rational::new(*v as u32, 1)));
      }
    }
    Ok(None)
  }

  fn get_wb(&self) -> Result<[f32; 4]> {
    let redmul = self.makernote.get_entry(OrfMakernotes::OlympusRedMul);
    let bluemul = self.makernote.get_entry(OrfMakernotes::OlympusBlueMul);
    match (redmul, bluemul) {
      (Some(redmul), Some(bluemul)) => Ok([redmul.force_u32(0) as f32, 256.0, 256.0, bluemul.force_u32(0) as f32]),
      _ => {
        let ifd = self.makernote.find_ifds_with_tag(OrfImageProcessing::OrfBlackLevels);
        if ifd.is_empty() {
          return Err(RawlerError::DecoderFailed("ORF: Couldn't find ImgProc IFD".to_string()));
        }
        let wbs = fetch_tiff_tag!(ifd[0], OrfImageProcessing::WB_RBLevels);
        Ok([wbs.force_f32(0), 256.0, 256.0, wbs.force_f32(1)])
      }
    }
  }
}

fn normalize_wb(raw_wb: [f32; 4]) -> [f32; 4] {
  log::debug!("ORF raw wb: {:?}", raw_wb);
  let div = raw_wb[1];
  let mut norm = raw_wb;
  norm.iter_mut().for_each(|v| {
    if v.is_normal() {
      *v /= div
    }
  });
  [norm[0], (norm[1] + norm[2]) / 2.0, norm[3], f32::NAN]
}

crate::tags::tiff_tag_enum!(OrfMakernotes);
crate::tags::tiff_tag_enum!(OrfImageProcessing);
crate::tags::tiff_tag_enum!(OrfEquipmentTags);

#[allow(non_camel_case_types)]
#[derive(Debug, Copy, Clone, PartialEq, enumn::N)]
#[repr(u16)]
pub enum OrfMakernotes {
  ImageProcessingIFD = 0x2040,
  RawInfo = 0x3000,
  OlympusRedMul = 0x1017,
  OlympusBlueMul = 0x1018,
  EquipmentIFD = 0x2010,
}

#[allow(non_camel_case_types)]
#[derive(Debug, Copy, Clone, PartialEq, enumn::N)]
#[repr(u16)]
pub enum OrfImageProcessing {
  ImageProcessingVersion = 0x0000,
  WB_RBLevels = 0x0100,
  OrfBlackLevels = 0x0600,
  CropLeft = 0x0612,
  CropTop = 0x0613,
  CropWidth = 0x0614,
  CropHeight = 0x0615,
}

#[allow(non_camel_case_types)]
#[derive(Debug, Copy, Clone, PartialEq, enumn::N)]
#[repr(u16)]
pub enum OrfEquipmentTags {
  LensType = 0x0201,
}
