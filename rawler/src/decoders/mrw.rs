use byteorder::ReadBytesExt;

use crate::analyze::FormatDump;
use crate::bits::BEu16;
use crate::bits::BEu32;
use crate::exif::Exif;
use crate::formats::jfif::is_exif;
use crate::formats::jfif::Jfif;
use crate::formats::jfif::Segment;
use crate::formats::tiff::IFD;
use crate::packed::decode_12be;
use crate::packed::decode_12be_unpacked;
use crate::packed::decode_16le;
use crate::tags::TiffCommonTag;
use crate::RawFile;
use crate::RawImage;
use crate::RawLoader;
use crate::RawlerError;
use crate::Result;
use std::io::Cursor;

use super::ok_cfa_image;
use super::Camera;
use super::Decoder;
use super::FormatHint;
use super::RawDecodeParams;
use super::RawMetadata;

const MRW_MAGIC: u32 = 0x004D524D; // !memcmp (head,"\0MRM",4))?

pub fn is_mrw(file: &mut RawFile) -> bool {
  match file.subview(0, 4) {
    Ok(buf) => {
      if BEu32(&buf, 0) == MRW_MAGIC {
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
  camera: Camera,
}

impl<'a> MrwDecoder<'a> {
  pub fn new(file: &mut RawFile, rawloader: &'a RawLoader) -> Result<MrwDecoder<'a>> {
    if is_mrw(file) {
      Self::new_mrw(file, rawloader)
    } else if is_exif(file) {
      let exif = Jfif::new(file)?;
      Self::new_jfif(file, exif, rawloader)
    } else {
      Err(crate::RawlerError::DecoderFailed(format!(
        "MRW decoder can't decode given file: {}",
        file.path.display()
      )))
    }
  }

  /// Makernotes for MRW starts with "MLY" ASCII string
  fn get_mly_wb(ifd: &IFD, rawfile: &mut RawFile, data_offset: u64) -> Result<[u16; 4]> {
    if let Some(makernotes) = ifd.get_entry_recursive(TiffCommonTag::Makernote) {
      debug_assert_eq!(makernotes.get_data()[0..3], [b'M', b'L', b'Y']);
      if makernotes.get_data().get(0..3) == Some(&[b'M', b'L', b'Y']) {
        let mut buf = Cursor::new(rawfile.as_vec()?);
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

  pub fn new_jfif(file: &mut RawFile, jfif: Jfif, rawloader: &'a RawLoader) -> Result<MrwDecoder<'a>> {
    let tiff = jfif
      .exif_ifd()
      .ok_or(RawlerError::DecoderFailed("No EXIF IFD found in JFIF file".to_string()))?
      .clone();
    let camera = rawloader.check_supported(&tiff)?;
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
    let data_offset = (app1_offset + app1_len + camera.param_i32("offset_corr").unwrap_or(0) as u64) as usize;
    let raw_width = camera.raw_width;
    let raw_height = camera.raw_height;
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

  fn new_mrw(file: &mut RawFile, rawloader: &'a RawLoader) -> Result<MrwDecoder<'a>> {
    let full = file.as_vec()?;
    let buf = &full;
    let data_offset: usize = (BEu32(buf, 4) + 8) as usize;
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
          packed = buf[currpos + 24] == 12;
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
      currpos += (len + 8) as usize;
    }

    let tiff_data = file.subview_until_eof(tiffpos as u64)?;
    let tiff = IFD::new(&mut Cursor::new(tiff_data), 8, 0, 0, crate::bits::Endian::Big, &[])?;

    let camera = rawloader.check_supported(&tiff)?;

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
  fn raw_image(&self, file: &mut RawFile, _params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let src = file.subview_until_eof(self.data_offset as u64)?;

    let buffer = if self.bits == 16 {
      decode_16le(&src, self.raw_width, self.raw_height, dummy)
    } else if self.packed {
      decode_12be(&src, self.raw_width, self.raw_height, dummy)
    } else {
      decode_12be_unpacked(&src, self.raw_width, self.raw_height, dummy)
    };

    let wb_coeffs = if self.camera.find_hint("swapped_wb") {
      [self.wb_vals[2] as f32, self.wb_vals[0] as f32, self.wb_vals[0] as f32, self.wb_vals[1] as f32]
    } else {
      [self.wb_vals[0] as f32, self.wb_vals[1] as f32, self.wb_vals[2] as f32, self.wb_vals[3] as f32]
    };
    let cpp = 1;
    ok_cfa_image(self.camera.clone(), cpp, normalize_wb(wb_coeffs), buffer, dummy)
  }

  fn format_dump(&self) -> FormatDump {
    todo!()
  }

  fn raw_metadata(&self, _file: &mut RawFile, _params: RawDecodeParams) -> Result<RawMetadata> {
    let exif = Exif::new(&self.tiff)?;
    let mdata = RawMetadata::new(&self.camera, exif);
    Ok(mdata)
  }

  fn format_hint(&self) -> FormatHint {
    FormatHint::MRW
  }

  /*
  /// File is EXIF structure, but contains no valid JPEG image, so this is useless...
  fn full_image(&self, file: &mut RawFile) -> Result<Option<image::DynamicImage>> {
    if is_mrw(file) {
      Ok(None)
    } else if is_exif(file) {
      let buf = file.as_vec()?;
      dump_buf("/tmp/dmp1", &buf);
      let img = image::load_from_memory_with_format(&buf, image::ImageFormat::Jpeg)
        .map_err(|err| RawlerError::DecoderFailed(format!("Failed to get full image from RAW file: {:?}", err)))?;
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
  let mut norm = raw_wb;
  norm.iter_mut().for_each(|v| {
    if v.is_normal() {
      *v /= div
    }
  });
  [norm[0], (norm[1] + norm[2]) / 2.0, norm[3], f32::NAN]
}
