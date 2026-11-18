use byteorder::ReadBytesExt;

use crate::RawImage;
use crate::RawLoader;
use crate::RawlerError;
use crate::Result;
use crate::analyze::FormatDump;
use crate::bits::BEu16;
use crate::bits::BEu32;
use crate::decompressors::packed::decompress_12be;
use crate::decompressors::packed::decompress_12be_unpacked;
use crate::decompressors::packed::decompress_16le;
use crate::exif::Exif;
use crate::formats::jfif::Jfif;
use crate::formats::jfif::Segment;
use crate::formats::jfif::is_exif;
use crate::formats::tiff::IFD;
use crate::rawsource::RawSource;
use crate::tags::TiffCommonTag;
use std::io::Cursor;

use super::CameraLookup;
use super::Decoder;
use super::FormatHint;
use super::Orientation;
use super::RawDecodeParams;
use super::RawMetadata;
use super::WellKnownIFD;
use super::image_bytes;
use super::makernotes;
use super::ok_cfa_image;
use super::tiff_ifd;
use crate::tags::ExifTag;
use std::rc::Rc;

const MRW_MAGIC: u32 = 0x004D524D; // !memcmp (head,"\0MRM",4))?

pub fn is_mrw(file: &RawSource) -> bool {
  match file.subview(0, 4) {
    Ok(buf) => {
      if BEu32(buf, 0) == MRW_MAGIC {
        true
      } else {
        log::debug!("MRW: File MAGIC not found");
        false
      }
    }
    Err(err) => {
      log::error!("is_mrw() error: {:?}", err);
      false
    }
  }
}

#[derive(Debug, Clone)]
pub struct MrwDecoder<'a> {
  #[allow(unused)]
  rawloader: &'a RawLoader,
  data_offset: usize,
  raw_width: usize,
  raw_height: usize,
  bits: u8,
  packed: bool,
  wb_vals: [u16; 4],
  tiff: IFD,
  camera: CameraLookup,
}

impl<'a> MrwDecoder<'a> {
  pub fn new(file: &RawSource, rawloader: &'a RawLoader) -> Result<MrwDecoder<'a>> {
    if is_mrw(file) {
      Self::new_mrw(file, rawloader)
    } else if is_exif(file) {
      let exif = Jfif::new(file)?;
      Self::new_jfif(file, exif, rawloader)
    } else {
      Err(decoder_error!("MRW decoder can't decode given file: {}", file.path().display()))
    }
  }

  /// Makernotes for MRW starts with "MLY" ASCII string
  fn get_mly_wb(ifd: &IFD, rawfile: &RawSource, data_offset: u64) -> Result<[u16; 4]> {
    if let Some(makernotes) = ifd.get_entry_recursive(TiffCommonTag::Makernote) {
      if makernotes.get_data()?.starts_with(b"MLY") {
        let mut buf = Cursor::new(rawfile.buf());
        let mut wb = [0_u16; 4];
        let mut cam_mul = [1_u16; 4];

        while buf.position() < data_offset {
          wb[0] = wb[2];
          wb[2] = wb[1];
          wb[1] = wb[3];
          wb[3] = match buf.read_u16::<byteorder::BigEndian>() {
            Ok(val) => val,
            Err(_) => break,
          };
          if wb[1] == 256 && wb[3] == 256 && wb[0] > 256 && wb[0] < 640 && wb[2] > 256 && wb[2] < 640 {
            cam_mul.copy_from_slice(&wb);
            cam_mul.swap(2, 3);
            log::debug!("Found WB match: {:?}", cam_mul);
          }
        }
        return Ok(cam_mul);
      }
    } else {
      log::warn!("Makernotes not found, fall back to defaults!");
    }
    Ok([1, 1, 1, 1])
  }

  pub fn new_jfif(file: &RawSource, jfif: Jfif, rawloader: &'a RawLoader) -> Result<MrwDecoder<'a>> {
    let tiff = jfif
      .exif_ifd()
      .ok_or(RawlerError::decoder_failed("No EXIF IFD found in JFIF file".to_string()))?
      .clone();
    let camera = rawloader.lookup_camera(&tiff)?;
    let (app1_offset, app1_len) = jfif
      .segments
      .iter()
      .map(|seg| {
        if let Segment::APP1 { offset, app1 } = seg {
          Some((*offset, app1.len))
        } else {
          None
        }
      })
      .find(Option::is_some)
      .flatten()
      .unwrap_or_default();
    // The raw layout comes from the camera definition. For unknown cameras, it is
    // never used, as raw_image() fails for them.
    let raw_def = camera.metadata();
    let data_offset = (app1_offset + app1_len)
      .checked_add_signed(raw_def.param_i32("offset_corr").unwrap_or(0) as i64)
      .ok_or_else(|| decoder_error!("MRW: data offset of the JFIF file is out of range"))? as usize;
    let raw_width = raw_def.raw_width;
    let raw_height = raw_def.raw_height;
    let packed = false;
    let wb_vals = Self::get_mly_wb(&tiff, file, data_offset as u64)?;
    let bits = 16;

    Ok(MrwDecoder {
      data_offset,
      raw_width,
      raw_height,
      bits,
      packed,
      wb_vals,
      tiff,
      rawloader,
      camera,
    })
  }

  fn new_mrw(file: &RawSource, rawloader: &'a RawLoader) -> Result<MrwDecoder<'a>> {
    let data_offset = BEu32(file.subview(0, 8)?, 4) as usize + 8;
    // The blocks before the raw data, a view of them fails if they exceed the file
    let buf = file.subview(0, data_offset as u64)?;
    let mut raw_height: usize = 0;
    let mut raw_width: usize = 0;
    let bits = 12;
    let mut packed = false;
    let mut wb_vals: [u16; 4] = [0; 4];
    let mut tiffpos: usize = 0;

    let mut currpos: usize = 8;
    // At most we read 20 bytes from currpos so check we don't step outside that
    while currpos + 20 < data_offset {
      let tag: u32 = BEu32(buf, currpos);
      let len: u32 = BEu32(buf, currpos + 4);

      match tag {
        0x505244 => {
          // PRD
          raw_height = BEu16(buf, currpos + 16) as usize;
          raw_width = BEu16(buf, currpos + 18) as usize;
          packed = buf.get(currpos + 24) == Some(&12);
        }
        0x574247 => {
          // WBG
          for i in 0..4 {
            wb_vals[i] = BEu16(buf, currpos + 12 + i * 2);
          }
        }
        0x545457 => {
          // TTW
          // Base value for offsets needs to be at the beginning of the
          // TIFF block, not the file
          tiffpos = currpos + 8;
        }
        _ => {}
      }
      currpos = currpos.saturating_add(len as usize).saturating_add(8);
    }

    let tiff_data = file.subview_until_eof(tiffpos as u64)?;
    let tiff = IFD::new(&mut Cursor::new(tiff_data), 8, 0, 0, crate::bits::Endian::Big, &[])?;

    let camera = rawloader.lookup_camera(&tiff)?;

    Ok(MrwDecoder {
      data_offset,
      raw_width,
      raw_height,
      bits,
      packed,
      wb_vals,
      tiff,
      rawloader,
      camera,
    })
  }
}

impl<'a> Decoder for MrwDecoder<'a> {
  fn raw_image(&self, file: &RawSource, _params: &RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let camera = self.camera.supported()?;
    image_bytes("MRW", self.raw_width, self.raw_height, 16)?;
    let src = file.subview_until_eof(self.data_offset as u64)?;

    let buffer = if self.bits == 16 {
      decompress_16le(src, self.raw_width, self.raw_height, dummy).map_err(RawlerError::decoder_failed)?
    } else if self.packed {
      decompress_12be(src, self.raw_width, self.raw_height, dummy).map_err(RawlerError::decoder_failed)?
    } else {
      decompress_12be_unpacked(src, self.raw_width, self.raw_height, dummy).map_err(RawlerError::decoder_failed)?
    };

    let wb_coeffs = if camera.find_hint("swapped_wb") {
      [self.wb_vals[2] as f32, self.wb_vals[0] as f32, self.wb_vals[0] as f32, self.wb_vals[1] as f32]
    } else {
      [self.wb_vals[0] as f32, self.wb_vals[1] as f32, self.wb_vals[2] as f32, self.wb_vals[3] as f32]
    };
    let cpp = 1;
    let mut img = ok_cfa_image(camera.clone(), cpp, normalize_wb(wb_coeffs), buffer, dummy)?;
    img.orientation = Orientation::from_tiff(&self.tiff);
    Ok(img)
  }

  fn check_supported(&self) -> Result<()> {
    self.camera.supported().map(|_| ())
  }

  fn format_dump(&self) -> FormatDump {
    todo!()
  }

  fn raw_metadata(&self, _file: &RawSource, _params: &RawDecodeParams) -> Result<RawMetadata> {
    let exif = Exif::new(&self.tiff)?;
    let mdata = RawMetadata::new(self.camera.metadata(), exif);
    Ok(mdata)
  }

  fn ifd(&self, _file: &RawSource, _params: &RawDecodeParams, wk_ifd: WellKnownIFD) -> Result<Option<Rc<IFD>>> {
    Ok(match wk_ifd {
      WellKnownIFD::Root | WellKnownIFD::Exif | WellKnownIFD::ExifGps => tiff_ifd(&self.tiff, wk_ifd),
      WellKnownIFD::VirtualMakernotes => makernotes::from_exif(self.format_hint(), self.tiff.find_first_ifd_with_tag(ExifTag::MakerNotes), None).map(Rc::new),
      _ => None,
    })
  }

  fn format_hint(&self) -> FormatHint {
    FormatHint::MRW
  }

  /*
  /// File is EXIF structure, but contains no valid JPEG image, so this is useless...
  fn full_image(&self, file: &RawSource) -> Result<Option<image::DynamicImage>> {
    if is_mrw(file) {
      Ok(None)
    } else if is_exif(file) {
      let buf = file.as_vec()?;
      let img = image::load_from_memory_with_format(&buf, image::ImageFormat::Jpeg)
        .map_err(|err| decoder_error!("Failed to get full image from RAW file: {:?}", err))?;
      log::debug!("Got full image from RAW");
      Ok(Some(img))
    } else {
      Ok(None)
    }
  }
   */
}

fn normalize_wb(raw_wb: [f32; 4]) -> [f32; 4] {
  log::debug!("MRW raw wb: {:?}", raw_wb);
  let div = raw_wb[1]; // G1 should be 1024 and we use this as divisor
  if !div.is_normal() {
    return [f32::NAN; 4];
  }
  let mut norm = raw_wb;
  norm.iter_mut().for_each(|v| {
    if v.is_normal() {
      *v /= div
    }
  });
  [norm[0], (norm[1] + norm[2]) / 2.0, norm[3], f32::NAN]
}

#[cfg(test)]
mod tests {
  use super::*;

  /// MRW file with a data offset and the given blocks
  fn mrw_file(data_offset: u32, blocks: &[u8]) -> Vec<u8> {
    let mut buf = b"\0MRM".to_vec();
    buf.extend_from_slice(&(data_offset - 8).to_be_bytes());
    buf.extend_from_slice(blocks);
    buf
  }

  #[test]
  fn short_or_invalid_headers_are_an_error() {
    let loader = crate::global_loader();
    // Too short for the data offset
    assert!(MrwDecoder::new(&RawSource::new_from_slice(b"\0MRM\0\0"), loader).is_err());
    // Data offset behind the end of the file
    assert!(MrwDecoder::new(&RawSource::new_from_slice(&mrw_file(1000, &[0; 32])), loader).is_err());
    // A PRD block that ends before its packing flag, and a block length that overflows
    let mut blocks = b"\0PRD\0\0\0\x10".to_vec();
    blocks.extend_from_slice(&[0; 13]);
    assert!(MrwDecoder::new(&RawSource::new_from_slice(&mrw_file(8 + blocks.len() as u32, &blocks)), loader).is_err());
    let mut blocks = b"\0XYZ\xff\xff\xff\xff".to_vec();
    blocks.extend_from_slice(&[0; 24]);
    assert!(MrwDecoder::new(&RawSource::new_from_slice(&mrw_file(8 + blocks.len() as u32, &blocks)), loader).is_err());
  }

  #[test]
  fn wb_without_green_is_unknown() {
    assert!(normalize_wb([0.0; 4]).iter().all(|v| v.is_nan()));
    assert_eq!(normalize_wb([512.0, 256.0, 256.0, 384.0])[..3], [2.0, 1.0, 1.5]);
  }
}
