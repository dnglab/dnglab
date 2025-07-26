use image::DynamicImage;
use image::ImageBuffer;
use image::Rgb;
use log::debug;
use log::warn;
use serde::Deserialize;
use serde::Serialize;

use crate::RawImage;
use crate::RawLoader;
use crate::RawlerError;
use crate::Result;
use crate::alloc_image_ok;
use crate::analyze::FormatDump;
use crate::bits::BEu16;
use crate::bits::BEu32;
use crate::bits::Endian;
use crate::bits::LEu32;
use crate::bits::LookupTable;
use crate::bits::clampbits;
use crate::buffer::PaddedBuf;
use crate::decoders::decode_threaded;
use crate::decoders::nef::lensdata::NefLensData;
use crate::decompressors::ljpeg::huffman::HuffTable;
use crate::exif::Exif;
use crate::formats::tiff::GenericTiffReader;
use crate::formats::tiff::IFD;
use crate::formats::tiff::Value;
use crate::formats::tiff::ifd::OffsetMode;
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
use crate::pumps::ByteStream;
use crate::rawimage::CFAConfig;
use crate::rawimage::RawPhotometricInterpretation;
use crate::rawimage::WhiteLevel;
use crate::rawsource::RawSource;
use crate::tags::ExifTag;
use crate::tags::TiffCommonTag;

use super::BlackLevel;
use super::Camera;
use super::Decoder;
use super::FormatHint;
use super::RawDecodeParams;
use super::RawMetadata;

mod decrypt;
pub mod lensdata;

const NIKON_F_MOUNT: &str = "F-mount";
const NIKON_Z_MOUNT: &str = "Z-mount";

// NEF Huffman tables in order. First two are the normal huffman definitions.
// Third one are weird shifts that are used in the lossy split encodings only
// Values are extracted from dcraw with the shifts unmangled out.
const NIKON_TREE: [[[u8; 16]; 3]; 6] = [
  [
    // 12-bit lossy
    [0, 0, 1, 5, 1, 1, 1, 1, 1, 1, 2, 0, 0, 0, 0, 0],
    [5, 4, 3, 6, 2, 7, 1, 0, 8, 9, 11, 10, 12, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  ],
  [
    // 12-bit lossy after split
    [0, 0, 1, 5, 1, 1, 1, 1, 1, 1, 2, 0, 0, 0, 0, 0],
    [6, 5, 5, 5, 5, 5, 4, 3, 2, 1, 0, 11, 12, 12, 0, 0],
    [3, 5, 3, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  ],
  [
    // 12-bit lossless
    [0, 0, 1, 4, 2, 3, 1, 2, 0, 0, 0, 0, 0, 0, 0, 0],
    [5, 4, 6, 3, 7, 2, 8, 1, 9, 0, 10, 11, 12, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  ],
  [
    // 14-bit lossy
    [0, 0, 1, 4, 3, 1, 1, 1, 1, 1, 2, 0, 0, 0, 0, 0],
    [5, 6, 4, 7, 8, 3, 9, 2, 1, 0, 10, 11, 12, 13, 14, 0],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  ],
  [
    // 14-bit lossy after split
    [0, 0, 1, 5, 1, 1, 1, 1, 1, 1, 1, 2, 0, 0, 0, 0],
    [8, 7, 7, 7, 7, 7, 6, 5, 4, 3, 2, 1, 0, 13, 14, 0],
    [0, 5, 4, 3, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  ],
  [
    // 14-bit lossless
    [0, 0, 1, 4, 2, 2, 3, 1, 2, 0, 0, 0, 0, 0, 0, 0],
    [7, 6, 8, 5, 9, 4, 10, 3, 11, 12, 2, 0, 1, 13, 14, 0],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  ],
];

// We use this for the D50 and D2X whacky WB "encryption"
const WB_SERIALMAP: [u8; 256] = [
  0xc1, 0xbf, 0x6d, 0x0d, 0x59, 0xc5, 0x13, 0x9d, 0x83, 0x61, 0x6b, 0x4f, 0xc7, 0x7f, 0x3d, 0x3d, 0x53, 0x59, 0xe3, 0xc7, 0xe9, 0x2f, 0x95, 0xa7, 0x95, 0x1f,
  0xdf, 0x7f, 0x2b, 0x29, 0xc7, 0x0d, 0xdf, 0x07, 0xef, 0x71, 0x89, 0x3d, 0x13, 0x3d, 0x3b, 0x13, 0xfb, 0x0d, 0x89, 0xc1, 0x65, 0x1f, 0xb3, 0x0d, 0x6b, 0x29,
  0xe3, 0xfb, 0xef, 0xa3, 0x6b, 0x47, 0x7f, 0x95, 0x35, 0xa7, 0x47, 0x4f, 0xc7, 0xf1, 0x59, 0x95, 0x35, 0x11, 0x29, 0x61, 0xf1, 0x3d, 0xb3, 0x2b, 0x0d, 0x43,
  0x89, 0xc1, 0x9d, 0x9d, 0x89, 0x65, 0xf1, 0xe9, 0xdf, 0xbf, 0x3d, 0x7f, 0x53, 0x97, 0xe5, 0xe9, 0x95, 0x17, 0x1d, 0x3d, 0x8b, 0xfb, 0xc7, 0xe3, 0x67, 0xa7,
  0x07, 0xf1, 0x71, 0xa7, 0x53, 0xb5, 0x29, 0x89, 0xe5, 0x2b, 0xa7, 0x17, 0x29, 0xe9, 0x4f, 0xc5, 0x65, 0x6d, 0x6b, 0xef, 0x0d, 0x89, 0x49, 0x2f, 0xb3, 0x43,
  0x53, 0x65, 0x1d, 0x49, 0xa3, 0x13, 0x89, 0x59, 0xef, 0x6b, 0xef, 0x65, 0x1d, 0x0b, 0x59, 0x13, 0xe3, 0x4f, 0x9d, 0xb3, 0x29, 0x43, 0x2b, 0x07, 0x1d, 0x95,
  0x59, 0x59, 0x47, 0xfb, 0xe5, 0xe9, 0x61, 0x47, 0x2f, 0x35, 0x7f, 0x17, 0x7f, 0xef, 0x7f, 0x95, 0x95, 0x71, 0xd3, 0xa3, 0x0b, 0x71, 0xa3, 0xad, 0x0b, 0x3b,
  0xb5, 0xfb, 0xa3, 0xbf, 0x4f, 0x83, 0x1d, 0xad, 0xe9, 0x2f, 0x71, 0x65, 0xa3, 0xe5, 0x07, 0x35, 0x3d, 0x0d, 0xb5, 0xe9, 0xe5, 0x47, 0x3b, 0x9d, 0xef, 0x35,
  0xa3, 0xbf, 0xb3, 0xdf, 0x53, 0xd3, 0x97, 0x53, 0x49, 0x71, 0x07, 0x35, 0x61, 0x71, 0x2f, 0x43, 0x2f, 0x11, 0xdf, 0x17, 0x97, 0xfb, 0x95, 0x3b, 0x7f, 0x6b,
  0xd3, 0x25, 0xbf, 0xad, 0xc7, 0xc5, 0xc5, 0xb5, 0x8b, 0xef, 0x2f, 0xd3, 0x07, 0x6b, 0x25, 0x49, 0x95, 0x25, 0x49, 0x6d, 0x71, 0xc7,
];

const WB_KEYMAP: [u8; 256] = [
  0xa7, 0xbc, 0xc9, 0xad, 0x91, 0xdf, 0x85, 0xe5, 0xd4, 0x78, 0xd5, 0x17, 0x46, 0x7c, 0x29, 0x4c, 0x4d, 0x03, 0xe9, 0x25, 0x68, 0x11, 0x86, 0xb3, 0xbd, 0xf7,
  0x6f, 0x61, 0x22, 0xa2, 0x26, 0x34, 0x2a, 0xbe, 0x1e, 0x46, 0x14, 0x68, 0x9d, 0x44, 0x18, 0xc2, 0x40, 0xf4, 0x7e, 0x5f, 0x1b, 0xad, 0x0b, 0x94, 0xb6, 0x67,
  0xb4, 0x0b, 0xe1, 0xea, 0x95, 0x9c, 0x66, 0xdc, 0xe7, 0x5d, 0x6c, 0x05, 0xda, 0xd5, 0xdf, 0x7a, 0xef, 0xf6, 0xdb, 0x1f, 0x82, 0x4c, 0xc0, 0x68, 0x47, 0xa1,
  0xbd, 0xee, 0x39, 0x50, 0x56, 0x4a, 0xdd, 0xdf, 0xa5, 0xf8, 0xc6, 0xda, 0xca, 0x90, 0xca, 0x01, 0x42, 0x9d, 0x8b, 0x0c, 0x73, 0x43, 0x75, 0x05, 0x94, 0xde,
  0x24, 0xb3, 0x80, 0x34, 0xe5, 0x2c, 0xdc, 0x9b, 0x3f, 0xca, 0x33, 0x45, 0xd0, 0xdb, 0x5f, 0xf5, 0x52, 0xc3, 0x21, 0xda, 0xe2, 0x22, 0x72, 0x6b, 0x3e, 0xd0,
  0x5b, 0xa8, 0x87, 0x8c, 0x06, 0x5d, 0x0f, 0xdd, 0x09, 0x19, 0x93, 0xd0, 0xb9, 0xfc, 0x8b, 0x0f, 0x84, 0x60, 0x33, 0x1c, 0x9b, 0x45, 0xf1, 0xf0, 0xa3, 0x94,
  0x3a, 0x12, 0x77, 0x33, 0x4d, 0x44, 0x78, 0x28, 0x3c, 0x9e, 0xfd, 0x65, 0x57, 0x16, 0x94, 0x6b, 0xfb, 0x59, 0xd0, 0xc8, 0x22, 0x36, 0xdb, 0xd2, 0x63, 0x98,
  0x43, 0xa1, 0x04, 0x87, 0x86, 0xf7, 0xa6, 0x26, 0xbb, 0xd6, 0x59, 0x4d, 0xbf, 0x6a, 0x2e, 0xaa, 0x2b, 0xef, 0xe6, 0x78, 0xb6, 0x4e, 0xe0, 0x2f, 0xdc, 0x7c,
  0xbe, 0x57, 0x19, 0x32, 0x7e, 0x2a, 0xd0, 0xb8, 0xba, 0x29, 0x00, 0x3c, 0x52, 0x7d, 0xa8, 0x49, 0x3b, 0x2d, 0xeb, 0x25, 0x49, 0xfa, 0xa3, 0xaa, 0x39, 0xa7,
  0xc5, 0xa7, 0x50, 0x11, 0x36, 0xfb, 0xc6, 0x67, 0x4a, 0xf5, 0xa5, 0x12, 0x65, 0x7e, 0xb0, 0xdf, 0xaf, 0x4e, 0xb3, 0x61, 0x7f, 0x2f,
];

/// NEF format encapsulation for analyzer
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NefFormat {
  tiff: GenericTiffReader,
}

#[derive(Debug, Clone)]
pub struct NefDecoder<'a> {
  #[allow(unused)]
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
  makernote: IFD,
  camera: Camera,
}

impl<'a> NefDecoder<'a> {
  pub fn new(file: &RawSource, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<NefDecoder<'a>> {
    let raw = tiff
      .find_first_ifd_with_tag(TiffCommonTag::CFAPattern)
      .ok_or_else(|| RawlerError::DecoderFailed(format!("Failed to find a IFD with CFAPattern tag")))?;
    let bps = fetch_tiff_tag!(raw, TiffCommonTag::BitsPerSample).force_usize(0);

    // Make sure we always use a 12/14 bit mode to get correct white/blackpoints
    let mode = format!("{}bit", bps);
    let camera = rawloader.check_supported_with_mode(tiff.root_ifd(), &mode)?;

    let makernote = if let Some(exif) = tiff.find_first_ifd_with_tag(ExifTag::MakerNotes) {
      exif.parse_makernote(&mut file.reader(), OffsetMode::Absolute, &[])?
    } else {
      warn!("NEF makernote not found");
      None
    }
    .ok_or("File has not makernotes")?;

    //makernote.dump::<ExifTag>(0).iter().for_each(|line| eprintln!("DUMP: {}", line));

    Ok(NefDecoder {
      tiff,
      rawloader,
      makernote,
      camera,
    })
  }
}

impl<'a> Decoder for NefDecoder<'a> {
  fn raw_image(&self, file: &RawSource, _params: &RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let raw = self
      .tiff
      .find_first_ifd_with_tag(TiffCommonTag::CFAPattern)
      .ok_or_else(|| RawlerError::DecoderFailed(format!("Failed to find a IFD with CFAPattern tag")))?;
    let mut width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let bps = fetch_tiff_tag!(raw, TiffCommonTag::BitsPerSample).force_usize(0);
    let compression = fetch_tiff_tag!(raw, TiffCommonTag::Compression).force_usize(0);

    let nef_compression = if let Some(z_makernote) = self.makernote.get_entry(NikonMakernote::Makernotes0x51) {
      // For new Z models, a new tag 0x51 for makernotes appears. This contains
      // The new-old NEFCompression tag. The old tag is unavailable in this models.
      Some(NefCompression::try_from(crate::bits::LEu16(z_makernote.get_data(), 10)).map_err(RawlerError::from)?)
    } else {
      self
        .makernote
        .get_entry(NikonMakernote::NefCompression)
        .map(|entry| entry.force_u16(0))
        .map(NefCompression::try_from)
        .transpose()
        .map_err(RawlerError::from)?
    };
    debug!("TIFF compression flag: {}, NEF compression mode: {:?}", compression, nef_compression);

    if matches!(nef_compression, Some(NefCompression::HighEfficency)) || matches!(nef_compression, Some(NefCompression::HighEfficencyStar)) {
      return Err(RawlerError::DecoderFailed(format!("NEF compression {:?} is not supported", nef_compression)));
    }

    let offset = fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_usize(0);
    let size = fetch_tiff_tag!(raw, TiffCommonTag::StripByteCounts).force_usize(0);
    let rows_per_strip = fetch_tiff_tag!(raw, TiffCommonTag::RowsPerStrip).get_usize(0).ok().flatten().unwrap_or(height);

    // That's little bit hacky here. Some files like D500 using multiple strips.
    // Because the strips has no holes between and are perfectly aligned, we can process the whole
    // chunk at once, instead of iterating over every strip.
    // It would be safer to process each strip offset, but it is not need for any known model so far.
    let src = if rows_per_strip == height {
      file.subview_padded(offset as u64, size as u64)?
    } else {
      let full_size: u32 = match fetch_tiff_tag!(raw, TiffCommonTag::StripByteCounts) {
        Value::Long(data) => data.iter().copied().sum(),
        _ => {
          return Err("StripByteCounts is not of type LONG".into());
        }
      };
      file.subview_padded(offset as u64, full_size as u64)?
    };

    let mut cpp = 1;
    let coeffs = normalize_wb(self.get_wb()?);
    debug!("WB coeff: {:?}", coeffs);

    assert_eq!(self.tiff.little_endian(), self.makernote.endian == Endian::Little);

    let image = if self.camera.model == "NIKON D100" {
      width = 3040;
      decode_12be_wcontrol(&src, width, height, dummy)
    } else if self.camera.find_hint("coolpixsplit") {
      decode_12be_interlaced_unaligned(&src, width, height, dummy)
    } else if self.camera.find_hint("msb32") {
      decode_12be_msb32(&src, width, height, dummy)
    } else if self.camera.find_hint("unpacked") {
      // P7800 and others is LE, but data is BE, so we use hints here
      if (self.tiff.little_endian() || self.camera.find_hint("little_endian")) && !self.camera.find_hint("big_endian") {
        decode_16le(&src, width, height, dummy)
      } else {
        decode_16be(&src, width, height, dummy)
      }
    } else if let Some(padding) = self.is_uncompressed(raw)? {
      debug!("NEF uncompressed row padding: {}, little-endian: {}", padding, self.tiff.little_endian());
      match bps {
        14 => {
          if (self.tiff.little_endian() || self.camera.find_hint("little_endian")) && !self.camera.find_hint("big_endian") {
            // Models like D6 uses packed instead of unpacked 14le encoding. And D6 uses
            // row padding.
            if matches!(nef_compression, Some(NefCompression::Packed14Bits)) {
              decode_14le_padded(&src, width, height, (width * bps / u8::BITS as usize) + padding, dummy)
            } else {
              decode_14le_unpacked(&src, width, height, dummy)
            }
          } else {
            decode_14be_unpacked(&src, width, height, dummy)
          }
        }
        12 => {
          if (self.tiff.little_endian() || self.camera.find_hint("little_endian")) && !self.camera.find_hint("big_endian") {
            decode_12le_padded(&src, width, height, (width * bps / u8::BITS as usize) + padding, dummy)
          } else {
            decode_12be(&src, width, height, dummy)
          }
        }
        x => return Err(RawlerError::unsupported(&self.camera, format!("Don't know uncompressed bps {}", x))),
      }
    } else if size == width * height * 3 {
      cpp = 3;
      Self::decode_snef_compressed(&src, coeffs, width, height, dummy)
    } else if compression == 34713 {
      self.decode_compressed(&src, width, height, bps, dummy)?
    } else {
      return Err(RawlerError::unsupported(&self.camera, format!("NEF: Don't know compression {}", compression)));
    };

    assert_eq!(image.width, width * cpp);
    let blacklevel = self.get_blacklevel(bps)?;
    let whitelevel = None;
    let photometric = match cpp {
      1 => RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&self.camera)),
      3 => RawPhotometricInterpretation::LinearRaw,
      _ => todo!(),
    };
    let mut img = RawImage::new(self.camera.clone(), image, cpp, coeffs, photometric, blacklevel, whitelevel, dummy);

    if let Some(crop) = self.get_crop()? {
      debug!("RAW Crops: {:?}", crop);
      img.crop_area = Some(crop);
    }

    if cpp == 3 {
      // Reset levels to defaults (0)
      img.blacklevel = BlackLevel::default();
      img.whitelevel = WhiteLevel::new(vec![65535; cpp]);
    }

    Ok(img)
  }

  fn format_dump(&self) -> FormatDump {
    FormatDump::Nef(NefFormat { tiff: self.tiff.clone() })
  }

  fn raw_metadata(&self, _file: &RawSource, _params: &RawDecodeParams) -> Result<RawMetadata> {
    let exif = Exif::new(self.tiff.root_ifd())?;
    //let mdata = RawMetadata::new(&self.camera, exif);
    let mdata = RawMetadata::new_with_lens(&self.camera, exif, self.get_lens_description()?.cloned());
    Ok(mdata)
  }

  fn full_image(&self, file: &RawSource, params: &RawDecodeParams) -> Result<Option<DynamicImage>> {
    if params.image_index != 0 {
      return Ok(None);
    }
    let root_ifd = &self.tiff.root_ifd();
    if !root_ifd.contains_singlestrip_image() {
      // TODO: implement multistrip
      return Ok(None);
    }
    let buf = root_ifd
      .singlestrip_data_rawsource(file)
      .map_err(|e| RawlerError::DecoderFailed(format!("Failed to get strip data: {}", e)))?;
    let compression = root_ifd.get_entry(TiffCommonTag::Compression).ok_or("Missing tag")?.force_usize(0);
    let width = fetch_tiff_tag!(root_ifd, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(root_ifd, TiffCommonTag::ImageLength).force_usize(0);
    if compression == 1 {
      Ok(Some(DynamicImage::ImageRgb8(
        ImageBuffer::<Rgb<u8>, Vec<u8>>::from_raw(width as u32, height as u32, buf.to_vec())
          .ok_or_else(|| RawlerError::DecoderFailed(format!("Failed to read image")))?,
      )))
    } else {
      let img = image::load_from_memory_with_format(buf, image::ImageFormat::Jpeg)
        .map_err(|err| RawlerError::DecoderFailed(format!("Failed to read JPEG image: {:?}", err)))?;
      Ok(Some(img))
    }
  }

  fn format_hint(&self) -> FormatHint {
    FormatHint::NEF
  }
}

impl<'a> NefDecoder<'a> {
  /// For older formats, we use the camera definitions and this here
  /// is useless. But if we found here the levels in makernotes, we
  /// use these instead. For 12 bit images, the blacklevels are still relative to
  /// 14 bit image data. So we need to reduce them by 2 bits.
  fn get_blacklevel(&self, bps: usize) -> Result<Option<BlackLevel>> {
    if let Some(levels) = self.makernote.get_entry(NikonMakernote::BlackLevel) {
      let mut black = [levels.force_u16(0), levels.force_u16(1), levels.force_u16(2), levels.force_u16(3)];
      if bps == 12 {
        black.iter_mut().for_each(|v| *v >>= 14 - 12);
      }
      Ok(Some(BlackLevel::new(&black, self.camera.cfa.width, self.camera.cfa.height, 1)))
    } else {
      Ok(None)
    }
  }

  fn get_crop(&self) -> Result<Option<Rect>> {
    if let Some(crop) = self.makernote.get_entry(NikonMakernote::CropArea) {
      let values = [crop.force_u16(0), crop.force_u16(1), crop.force_u16(2), crop.force_u16(3)];
      let rect = Rect::new(
        Point::new(values[0] as usize, values[1] as usize),
        Dim2::new(values[2] as usize, values[3] as usize),
      );
      Ok(Some(rect))
    } else {
      Ok(None)
    }
  }

  /// Get lens description by analyzing TIFF tags and makernotes
  fn get_lens_description(&self) -> Result<Option<&'static LensDescription>> {
    if let Some(lensdata) = lensdata::from_makernote(&self.makernote)? {
      if let Some(lenstype) = self.makernote.get_entry(NikonMakernote::LensType) {
        match lensdata {
          NefLensData::FMount(oldv) => {
            let composite_id = oldv.composite_id(lenstype.force_u8(0));
            log::debug!("NEF lens composite ID: {}", composite_id);
            let resolver = LensResolver::new()
              .with_nikon_id(Some(composite_id))
              .with_camera(&self.camera)
              .with_mounts(&[NIKON_F_MOUNT.into()]);
            return Ok(resolver.resolve());
          }
          NefLensData::ZMount(newv) => {
            let resolver = LensResolver::new()
              .with_lens_id((newv.lens_id as u32, 0))
              .with_camera(&self.camera)
              .with_mounts(&[NIKON_Z_MOUNT.into()]);
            return Ok(resolver.resolve());
          }
        }
      }
    }
    Ok(None)
  }

  fn get_wb(&self) -> Result<[f32; 4]> {
    if self.camera.find_hint("nowb") {
      Ok([f32::NAN, f32::NAN, f32::NAN, f32::NAN])
    } else if let Some(levels) = self.makernote.get_entry(TiffCommonTag::NefWB0) {
      Ok([levels.force_f32(0), 1.0, 1.0, levels.force_f32(1)])
    } else if let Some(levels) = self.makernote.get_entry(TiffCommonTag::NrwWB) {
      let data = levels.get_data();
      if data[0..3] == b"NRW"[..] {
        let offset = if data[4..8] == b"0100"[..] { 1556 } else { 56 };
        Ok([
          (LEu32(data, offset) << 2) as f32,
          (LEu32(data, offset + 4) + LEu32(data, offset + 8)) as f32,
          (LEu32(data, offset + 4) + LEu32(data, offset + 8)) as f32,
          (LEu32(data, offset + 12) << 2) as f32,
        ])
      } else {
        Ok([BEu16(data, 1248) as f32, 256.0, 256.0, BEu16(data, 1250) as f32])
      }
    } else if let Some(levels) = self.makernote.get_entry(TiffCommonTag::NefWB1) {
      let mut version: u32 = 0;
      for i in 0..4 {
        version = (version << 4) + (levels.get_data()[i] - b'0') as u32;
      }
      let buf = levels.get_data();
      debug!("NEF Color balance version: 0x{:x}", version);

      match version {
        0x100 => Ok([
          BEu16(buf, 36 * 2) as f32,
          BEu16(buf, 38 * 2) as f32,
          BEu16(buf, 38 * 2) as f32,
          BEu16(buf, 37 * 2) as f32,
        ]),
        // Nikon D2H
        0x102 => Ok([
          BEu16(buf, 5 * 2) as f32,
          BEu16(buf, 6 * 2) as f32,
          BEu16(buf, 6 * 2) as f32,
          BEu16(buf, 8 * 2) as f32,
        ]),
        // Nikon D70
        0x103 => Ok([
          BEu16(buf, 10 * 2) as f32,
          BEu16(buf, 11 * 2) as f32,
          BEu16(buf, 11 * 2) as f32,
          BEu16(buf, 12 * 2) as f32,
        ]),
        0x204 | 0x205 => {
          let serial = fetch_tiff_tag!(self.makernote, TiffCommonTag::NefSerial);
          let data = serial.get_data();
          let mut serialno = 0_usize;
          for i in 0..serial.count() as usize {
            if data[i] == 0 {
              break;
            }
            serialno = serialno * 10
              + if data[i] >= 48 && data[i] <= 57 {
                // "0" to "9"
                (data[i] - 48) as usize
              } else {
                (data[i] % 10) as usize
              };
          }

          // Get the "decryption" key
          let keydata = fetch_tiff_tag!(self.makernote, TiffCommonTag::NefKey).force_u32(0).to_le_bytes();
          let keyno = (keydata[0] ^ keydata[1] ^ keydata[2] ^ keydata[3]) as usize;

          let src = if version == 0x204 {
            &levels.get_data()[284..]
          } else {
            &levels.get_data()[4..]
          };

          let ci = WB_SERIALMAP[serialno & 0xff] as u32;
          let mut cj = WB_KEYMAP[keyno & 0xff] as u32;
          let mut ck = 0x60_u32;
          let mut buf = [0_u8; 280];
          for i in 0..280 {
            cj += ci * ck;
            ck += 1;
            buf[i] = src[i] ^ (cj as u8);
          }

          let off = if version == 0x204 { 6 } else { 14 };
          Ok([
            BEu16(&buf, off) as f32,
            BEu16(&buf, off + 2) as f32,
            BEu16(&buf, off + 4) as f32,
            BEu16(&buf, off + 6) as f32,
          ])
        }
        x => Err(RawlerError::unsupported(&self.camera, format!("NEF: Don't know about WB version 0x{:x}", x))),
      }
    } else {
      Err(RawlerError::DecoderFailed("NEF: Don't know how to fetch WB".to_string()))
    }
  }

  fn create_hufftable(num: usize) -> Result<HuffTable> {
    let mut htable = HuffTable::empty();

    for i in 0..15 {
      htable.bits[i] = NIKON_TREE[num][0][i] as u32;
      htable.huffval[i] = NIKON_TREE[num][1][i] as u32;
      htable.shiftval[i] = NIKON_TREE[num][2][i] as u32;
    }

    htable.initialize()?;
    Ok(htable)
  }

  /// The compression flags in some raws are not reliable because of firmware bugs.
  /// We try to figure out the compression by some heuristics.
  /// The return value is None if the file is not uncompressed or Some(x)
  /// where x is the extra amount of bytes after each row.
  fn is_uncompressed(&self, raw: &IFD) -> Result<Option<usize>> {
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let bps = fetch_tiff_tag!(raw, TiffCommonTag::BitsPerSample).force_usize(0);
    let compression = fetch_tiff_tag!(raw, TiffCommonTag::Compression).force_usize(0);
    let size = fetch_tiff_tag!(raw, TiffCommonTag::StripByteCounts).force_usize(0);

    fn div_round_up(a: usize, b: usize) -> usize {
      a.div_ceil(b) // (a + b - 1) / b
    }

    let req_pixels = width * height;
    let req_input_bits = bps * req_pixels;
    let req_input_bytes = div_round_up(req_input_bits, 8);

    Ok(if compression == 1 || size == width * height * bps / 8 {
      Some(0)
    } else if size >= req_input_bytes {
      // Some models (D6) using row padding, so the row width is slightly larger.
      // This should be no more than 16 extra bytes.
      let total_padding = size - req_input_bytes;
      let per_row_padding = total_padding / height;
      if total_padding % height != 0 {
        None
      } else if per_row_padding < 16 {
        Some(per_row_padding)
      } else {
        None
      }
    } else {
      None
    })
  }

  fn decode_compressed(&self, src: &PaddedBuf, width: usize, height: usize, bps: usize, dummy: bool) -> Result<PixU16> {
    let meta = if let Some(meta) = self.makernote.get_entry(TiffCommonTag::NefMeta2) {
      debug!("Found NefMeta2");
      meta
    } else {
      debug!("Fallback NefMeta1");
      fetch_tiff_tag!(self.makernote, TiffCommonTag::NefMeta1)
    };
    Self::do_decode(src, meta.get_data(), self.makernote.endian, width, height, bps, dummy)
  }

  pub(crate) fn do_decode(src: &[u8], meta: &[u8], endian: Endian, width: usize, height: usize, bps: usize, dummy: bool) -> Result<PixU16> {
    debug!("NEF decode with: endian: {:?}, width: {}, height: {}, bps: {}", endian, width, height, bps);
    let mut out = alloc_image_ok!(width, height, dummy);
    let mut stream = ByteStream::new(meta, endian);
    let v0 = stream.get_u8();
    let v1 = stream.get_u8();
    debug!("Nef version v0:{}, v1:{}", v0, v1);

    let mut huff_select = 0;
    if v0 == 73 || v1 == 88 {
      assert!(stream.remaining_bytes() >= 2110);
      stream.consume_bytes(2110);
    }
    if v0 == 70 {
      huff_select = 2;
    }
    if bps == 14 {
      huff_select += 3;
    }

    // Create the huffman table used to decode
    let mut htable = Self::create_hufftable(huff_select)?;

    // Setup the predictors
    let mut pred_up1: [i32; 2] = [stream.get_u16() as i32, stream.get_u16() as i32];
    let mut pred_up2: [i32; 2] = [stream.get_u16() as i32, stream.get_u16() as i32];

    // Get the linearization curve
    let mut points = [0_u16; 1 << 16];
    for i in 0..points.len() {
      points[i] = i as u16;
    }

    // Some models reports 14 bits, but the data is 12 bits.
    // So we reduce the bps to calculate the max value which
    // is needed in the next steps.
    let real_bps = if v0 == 68 && v1 == 64 {
      bps as u32 - 2 // Special for D780, Z7 and others
    } else {
      bps as u32
    };
    let mut max = 1 << real_bps;

    let csize = stream.get_u16() as usize;
    let mut split = 0_usize;
    let step = if csize > 1 { max / (csize - 1) } else { 0 };
    if v0 == 68 && (v1 == 32 || v1 == 64) && step > 0 {
      for i in 0..csize {
        points[i * step] = stream.get_u16();
      }
      for i in 0..max {
        let b_scale = i % step;
        let a_pos = i - b_scale;
        let b_pos = a_pos + step;
        //assert!(a_pos < max);
        //assert!(b_pos > 0);
        //assert!(b_pos < max);
        //assert!(a_pos < b_pos);
        let a_scale = step - b_scale;
        points[i] = ((a_scale * points[a_pos] as usize + b_scale * points[b_pos] as usize) / step) as u16;
      }
      split = endian.read_u16(meta, 562) as usize;
    } else if v0 != 70 && csize <= 0x4001 {
      for i in 0..csize {
        points[i] = stream.get_u16();
      }
      max = csize;
    }
    let curve = LookupTable::new(&points[0..max]);

    let mut pump = BitPumpMSB::new(src);
    let mut random = pump.peek_bits(24);

    for row in 0..height {
      if split > 0 && row == split {
        htable = Self::create_hufftable(huff_select + 1)?;
      }
      pred_up1[row & 1] += htable.huff_decode(&mut pump)?;
      pred_up2[row & 1] += htable.huff_decode(&mut pump)?;
      let mut pred_left1 = pred_up1[row & 1];
      let mut pred_left2 = pred_up2[row & 1];
      for col in (0..width).step_by(2) {
        if col > 0 {
          pred_left1 += htable.huff_decode(&mut pump)?;
          pred_left2 += htable.huff_decode(&mut pump)?;
        }
        out[row * width + col + 0] = curve.dither(clampbits(pred_left1, real_bps), &mut random);
        out[row * width + col + 1] = curve.dither(clampbits(pred_left2, real_bps), &mut random);
      }
    }

    Ok(out)
  }

  // Decodes 12 bit data in an YUY2-like pattern (2 Luma, 1 Chroma per 2 pixels).
  // We un-apply the whitebalance, so output matches lossless.
  pub(crate) fn decode_snef_compressed(src: &PaddedBuf, coeffs: [f32; 4], width: usize, height: usize, dummy: bool) -> PixU16 {
    let inv_wb_r = (1024.0 / coeffs[0]) as i32;
    let inv_wb_b = (1024.0 / coeffs[2]) as i32;

    //println!("Got invwb {} {}", inv_wb_r, inv_wb_b);

    let snef_curve = {
      let g: f32 = 2.4;
      let f: f32 = 0.055;
      let min: f32 = 0.04045;
      let mul: f32 = 12.92;
      let curve = (0..4096)
        .map(|i| {
          let v = (i as f32) / 4095.0;
          let res = if v <= min { v / mul } else { ((v + f) / (1.0 + f)).powf(g) };
          clampbits((res * 65535.0 * 4.0) as i32, 16)
        })
        .collect::<Vec<u16>>();
      LookupTable::new(&curve)
    };

    decode_threaded(
      width * 3,
      height,
      dummy,
      &(|out: &mut [u16], row| {
        let inb = &src[row * width * 3..];
        let mut random = BEu32(inb, 0);
        for (o, i) in out.chunks_exact_mut(6).zip(inb.chunks_exact(6)) {
          let g1: u16 = i[0] as u16;
          let g2: u16 = i[1] as u16;
          let g3: u16 = i[2] as u16;
          let g4: u16 = i[3] as u16;
          let g5: u16 = i[4] as u16;
          let g6: u16 = i[5] as u16;

          let y1 = (g1 | ((g2 & 0x0f) << 8)) as f32;
          let y2 = ((g2 >> 4) | (g3 << 4)) as f32;
          let cb = (g4 | ((g5 & 0x0f) << 8)) as f32 - 2048.0;
          let cr = ((g5 >> 4) | (g6 << 4)) as f32 - 2048.0;

          let r = snef_curve.dither(clampbits((y1 + 1.370705 * cr) as i32, 12), &mut random);
          let g = snef_curve.dither(clampbits((y1 - 0.337633 * cb - 0.698001 * cr) as i32, 12), &mut random);
          let b = snef_curve.dither(clampbits((y1 + 1.732446 * cb) as i32, 12), &mut random);
          // invert the white balance
          o[0] = clampbits((inv_wb_r * r as i32 + (1 << 9)) >> 10, 15);
          o[1] = g;
          o[2] = clampbits((inv_wb_b * b as i32 + (1 << 9)) >> 10, 15);

          let r = snef_curve.dither(clampbits((y2 + 1.370705 * cr) as i32, 12), &mut random);
          let g = snef_curve.dither(clampbits((y2 - 0.337633 * cb - 0.698001 * cr) as i32, 12), &mut random);
          let b = snef_curve.dither(clampbits((y2 + 1.732446 * cb) as i32, 12), &mut random);
          // invert the white balance
          o[3] = clampbits((inv_wb_r * r as i32 + (1 << 9)) >> 10, 15);
          o[4] = g;
          o[5] = clampbits((inv_wb_b * b as i32 + (1 << 9)) >> 10, 15);
        }
      }),
    )
  }
}

fn normalize_wb(raw_wb: [f32; 4]) -> [f32; 4] {
  debug!("NEF raw wb: {:?}", raw_wb);
  // We never have more then RGB colors so far (no RGBE etc.)
  // So we combine G1 and G2 to get RGB wb.
  let div = raw_wb[1];
  let mut norm = raw_wb;
  norm.iter_mut().for_each(|v| {
    if v.is_normal() {
      *v /= div
    }
  });
  [norm[0], (norm[1] + norm[2]) / 2.0, norm[3], f32::NAN]
}

crate::tags::tiff_tag_enum!(NikonMakernote);

#[allow(non_camel_case_types)]
#[derive(Debug, Copy, Clone, PartialEq, enumn::N)]
#[repr(u16)]
pub enum NikonMakernote {
  MakernoteVersion = 0x0001,
  NefWB0 = 0x000C,
  PreviewIFD = 0x0011,
  NrwWB = 0x0014,
  NefSerial = 0x001d,
  ImageSizeRaw = 0x003e,
  CropArea = 0x0045,
  BlackLevel = 0x003d,
  Makernotes0x51 = 0x0051,
  LensType = 0x0083,
  NefMeta1 = 0x008c,
  NefMeta2 = 0x0096,
  ShotInfo = 0x0091,
  NefCompression = 0x0093,
  NefWB1 = 0x0097,
  LensData = 0x0098,
  NefKey = 0x00a7,
}

/// Known NEF compression formats
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[allow(non_camel_case_types)]
enum NefCompression {
  LossyType1 = 1,
  Uncompressed = 2,
  Lossless = 3,
  LossyType2 = 4,
  StripedPacked12Bits = 5,
  UncompressedReduced12Bits = 6,
  Unpacked12Bits = 7,
  Small = 8,
  Packed12Bits = 9,
  Packed14Bits = 10,
  HighEfficency = 13,
  HighEfficencyStar = 14,
}

impl TryFrom<u16> for NefCompression {
  type Error = String;

  fn try_from(v: u16) -> std::result::Result<Self, Self::Error> {
    Ok(match v {
      1 => Self::LossyType1,
      2 => Self::Uncompressed,
      3 => Self::Lossless,
      4 => Self::LossyType2,
      5 => Self::StripedPacked12Bits,
      6 => Self::UncompressedReduced12Bits,
      7 => Self::Unpacked12Bits,
      8 => Self::Small,
      9 => Self::Packed12Bits,
      10 => Self::Packed14Bits,
      13 => Self::HighEfficency,
      14 => Self::HighEfficencyStar,
      _ => return Err(format!("unknown nef compression: {}", v)),
    })
  }
}
