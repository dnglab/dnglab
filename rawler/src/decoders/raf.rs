use byteorder::BigEndian;
use byteorder::ReadBytesExt;
use std::collections::BTreeMap;
use std::f32::NAN;
use std::io::SeekFrom;

use crate::alloc_image_plain;
use crate::analyze::FormatDump;
use crate::bits::Endian;
use crate::exif::Exif;
use crate::formats::tiff::ifd::OffsetMode;
use crate::formats::tiff::*;
use crate::packed::*;
use crate::tags::ExifTag;
use crate::tags::TiffCommonTag;
use crate::tags::TiffTag;
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

/// RAF decoder
#[derive(Debug, Clone)]
pub struct RafDecoder<'a> {
  #[allow(unused)]
  rawloader: &'a RawLoader,
  ifd: IFD,
  camera: Camera,
}

/// Check if file has RAF signature
pub fn is_raf(file: &mut RawFile) -> bool {
  match file.subview(0, 8) {
    Ok(buf) => buf[0..8] == b"FUJIFILM"[..],
    Err(_) => false,
  }
}

/// We need to inject a virtual IFD into main IFD.
/// The RAF data block is not a regular TIFF structure but
/// a proprietary structure. This tag id should be unlikely
/// to appear in main IFD.
const RAF_TAG_VIRTUAL_RAF_DATA: u16 = 0xfaaa;

/// Parse a proprietary RAF data structure and return a virtual IFD.
pub fn parse_raf_format(file: &mut RawFile, offset: u32) -> Result<IFD> {
  let mut entries = BTreeMap::new();
  let stream = file.inner();
  stream.seek(SeekFrom::Start(offset as u64))?;
  let num = stream.read_u32::<BigEndian>()?; // Directory entries in this IFD
  if num > 4000 {
    return Err(format_args!("too many entries in IFD ({})", num).into());
  }
  for _ in 0..num {
    let tag = stream.read_u16::<BigEndian>()?;
    let len = stream.read_u16::<BigEndian>()?;
    if tag == TiffCommonTag::ImageWidth.into() {
      let mut values = Vec::new();
      for _ in 0..(len / 2) {
        values.push(stream.read_u16::<BigEndian>()?);
      }
      entries.insert(
        TiffCommonTag::ImageWidth.into(),
        Entry {
          tag: TiffCommonTag::ImageWidth.into(),
          value: Value::Short(values),
          embedded: None,
        },
      );
    } else if tag == TiffCommonTag::RafOldWB.into() {
      //assert_eq!(len, 4 * 2);
      let mut values = Vec::new();
      for _ in 0..(len / 2) {
        values.push(stream.read_u16::<BigEndian>()?);
      }
      entries.insert(
        TiffCommonTag::RafOldWB.into(),
        Entry {
          tag: TiffCommonTag::RafOldWB.into(),
          value: Value::Short(values),
          embedded: None,
        },
      );
    } else {
      stream.seek(SeekFrom::Current(len as i64))?;
    }
  }
  Ok(IFD {
    entries,
    endian: Endian::Big,
    offset: 0,
    base: offset as u32,
    corr: 0,
    next_ifd: 0,
    sub: Default::default(),
    chain: Default::default(),
  })
}

/// RAF format contains multiple TIFF and TIFF-like structures.
/// This creates a IFD with all other IFDs found collected as SubIFDs.
fn parse_raf(file: &mut RawFile) -> Result<IFD> {
  const RAF_TIFF1_PTR_OFFSET: u64 = 84;
  const RAF_TIFF2_PTR_OFFSET: u64 = 100;
  const RAF_BLOCK_PTR_OFFSET: u64 = 92;
  log::debug!("parse RAF");
  file
    .inner()
    .seek(SeekFrom::Start(0))
    .map_err(|e| RawlerError::General(format!("I/O error while trying decoders: {:?}", e)))?;

  let stream = file.inner();
  stream.seek(SeekFrom::Start(RAF_TIFF1_PTR_OFFSET))?;
  let offset = stream.read_u32::<BigEndian>()?;

  // Main IFD
  let mut main = IFD::new_root(stream, offset + 12)?;

  // There is a second TIFF structure, the pointer is stored at offset 100.
  // If it is not a valid TIFF structure, the pointer itself is the RAF offset.
  stream.seek(SeekFrom::Start(RAF_TIFF2_PTR_OFFSET))?;
  let ioffset = stream.read_u32::<BigEndian>()?;

  match IFD::new_root_with_correction(stream, 0, ioffset, 0, 10, &[RafIFD::FujiIFD.into()]) {
    Ok(val) => {
      log::debug!("Found valid FujiIFD (0xF000)");
      //val.dump::<RafIFD>(0).iter().for_each(|line| println!("RAF: {}", line));
      main.sub.insert(RafIFD::FujiIFD as u16, vec![val]);
    }
    Err(_) => {
      // We fake an FujiIFD to pass the StripOffsets
      log::debug!("Unable to find FujiIFD (0xF000), let's fake it");
      let mut entries = BTreeMap::<u16, Entry>::new();
      entries.insert(
        RafIFD::StripOffsets as u16,
        Entry {
          tag: RafIFD::StripOffsets as u16,
          value: Value::Long(vec![ioffset]), // The ioffset is absolute to the file start.
          embedded: Some(RAF_TIFF2_PTR_OFFSET as u32),
        },
      );
      let fake = IFD {
        offset: 0,
        base: 0, // For the faked IFD, the offsets are already absolute to the file start.
        corr: 0,
        next_ifd: 0,
        entries,
        endian: main.endian,
        sub: Default::default(),
        chain: Default::default(),
      };
      main.sub.insert(RafIFD::FujiIFD as u16, vec![fake]);
    }
  }
  // And we maybe have a RAF data block, try to parse it.
  stream.seek(SeekFrom::Start(RAF_BLOCK_PTR_OFFSET))?;
  let raf_offset = stream.read_u32::<BigEndian>()?;
  match parse_raf_format(file, raf_offset) {
    Ok(val) => {
      main.sub.insert(RAF_TAG_VIRTUAL_RAF_DATA, vec![val]);
    }
    Err(_) => {
      log::debug!("RAF block pointer is not valid, ignoring");
    }
  }

  Ok(main)
}

impl<'a> RafDecoder<'a> {
  pub fn new(file: &mut RawFile, rawloader: &'a RawLoader) -> Result<RafDecoder<'a>> {
    let ifd = parse_raf(file)?;
    let camera = rawloader.check_supported(&ifd)?;
    let makernote = if let Some(exif) = ifd.find_first_ifd_with_tag(ExifTag::MakerNotes) {
      exif.parse_makernote(file.inner(), OffsetMode::Absolute, &[])?
    } else {
      log::warn!("RAF makernote not found");
      None
    }
    .ok_or("File has not makernotes")?;

    makernote.dump::<RafMakernotes>(0).iter().for_each(|line| eprintln!("DUMP: {}", line)); // TODO: remove

    Ok(RafDecoder { ifd, rawloader, camera })
  }
}

impl<'a> Decoder for RafDecoder<'a> {
  fn raw_image(&self, file: &mut RawFile, _params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let raw = self.ifd.find_first_ifd_with_tag(RafIFD::StripOffsets).ok_or("No StripOffsets found")?;
    let (width, height) = if raw.has_entry(RafIFD::RawImageFullWidth) {
      (
        fetch_tiff_tag!(raw, RafIFD::RawImageFullWidth).force_usize(0),
        fetch_tiff_tag!(raw, RafIFD::RawImageFullHeight).force_usize(0),
      )
    } else {
      let raf = &self
        .ifd
        .sub_ifds()
        .get(&RAF_TAG_VIRTUAL_RAF_DATA)
        .and_then(|ifds| ifds.get(0))
        .ok_or("No RAF data IFD found")?;
      let sizes = fetch_tiff_tag!(raf, TiffCommonTag::ImageWidth);
      (sizes.force_usize(1), sizes.force_usize(0))
    };

    let bps = match raw.get_entry(TiffCommonTag::RafBitsPerSample) {
      Some(val) => val.force_u32(0) as usize,
      None => 16,
    };

    // Strip offset is relative to IFD base
    let offset = raw.base as u64 + fetch_tiff_tag!(raw, RafIFD::StripOffsets).force_u64(0);
    let src = if raw.has_entry(RafIFD::StripByteCounts) {
      let strip_count = fetch_tiff_tag!(raw, RafIFD::StripByteCounts).force_u64(0);
      file.subview(offset, strip_count)?
    } else {
      file.subview_until_eof(offset)?
    };

    let image = if self.camera.find_hint("double_width") {
      // Some fuji SuperCCD cameras include a second raw image next to the first one
      // that is identical but darker to the first. The two combined can produce
      // a higher dynamic range image. Right now we're ignoring it.
      decode_16le_skiplines(&src, width, height, dummy)
    } else if self.camera.find_hint("jpeg32") {
      decode_12be_msb32(&src, width, height, dummy)
    } else {
      if src.len() < bps * width * height / 8 {
        return Err(RawlerError::unsupported(&self.camera, "RAF: Don't know how to decode compressed yet"));
      }
      match bps {
        12 => decode_12le(&src, width, height, dummy),
        14 => decode_14le_unpacked(&src, width, height, dummy),
        16 => {
          if self.ifd.endian == Endian::Little {
            decode_16le(&src, width, height, dummy)
          } else {
            decode_16be(&src, width, height, dummy)
          }
        }
        _ => {
          return Err(RawlerError::unsupported(&self.camera, format!("RAF: Don't know how to decode bps {}", bps)));
        }
      }
    };

    let cpp = 1;
    if self.camera.find_hint("fuji_rotation") || self.camera.find_hint("fuji_rotation_alt") {
      log::debug!("Apply Fuji image rotation");
      let (width, height, image) = self.rotate_image(image.pixels(), &self.camera, width, height, dummy)?;

      let mut image = RawImage::new(self.camera.clone(), width, height, cpp, self.get_wb()?, image, dummy);
      image.bps = bps;
      // Reset crops because we have rotated the data.
      image.active_area = None;
      image.crop_area = None;
      Ok(image)
    } else {
      ok_image(self.camera.clone(), width, height, cpp, self.get_wb()?, image.into_inner())
    }
  }

  fn raw_metadata(&self, _file: &mut RawFile, _params: RawDecodeParams) -> Result<RawMetadata> {
    let exif = Exif::new(&self.ifd)?;
    let mdata = RawMetadata::new(&self.camera, exif);
    Ok(mdata)
  }

  fn format_dump(&self) -> FormatDump {
    todo!()
  }
}

impl<'a> RafDecoder<'a> {
  fn get_wb(&self) -> Result<[f32; 4]> {
    let raw = self.ifd.find_first_ifd_with_tag(RafIFD::StripOffsets).ok_or("No StripOffsets found")?;
    match raw.get_entry(RafIFD::WB_GRBLevels) {
      Some(levels) => Ok([levels.force_f32(1), levels.force_f32(0), levels.force_f32(2), NAN]),
      None => {
        let raf = &self
          .ifd
          .sub_ifds()
          .get(&RAF_TAG_VIRTUAL_RAF_DATA)
          .and_then(|ifds| ifds.get(0))
          .ok_or("No RAF data IFD found")?;
        let levels = fetch_tiff_tag!(raf, TiffCommonTag::RafOldWB);
        Ok([levels.force_f32(1), levels.force_f32(0), levels.force_f32(3), NAN])
      }
    }
  }

  fn rotate_image(&self, src: &[u16], camera: &Camera, width: usize, height: usize, dummy: bool) -> Result<(usize, usize, Vec<u16>)> {
    if let Some(active_area) = self.camera.active_area {
      let x = active_area[0];
      let y = active_area[1];
      let cropwidth = width - active_area[2] - x;
      let cropheight = height - active_area[3] - y; // TODO: bug, invalid order of crop index

      if camera.find_hint("fuji_rotation_alt") {
        let rotatedwidth = cropheight + cropwidth / 2;
        let rotatedheight = rotatedwidth - 1;

        let mut out: Vec<u16> = alloc_image_plain!(rotatedwidth, rotatedheight, dummy);
        if !dummy {
          for row in 0..cropheight {
            let inb = &src[(row + y) * width + x..];
            for col in 0..cropwidth {
              let out_row = rotatedwidth - (cropheight + 1 - row + (col >> 1));
              let out_col = ((col + 1) >> 1) + row;
              out[out_row * rotatedwidth + out_col] = inb[col];
            }
          }
        }

        Ok((rotatedwidth, rotatedheight, out))
      } else {
        let rotatedwidth = cropwidth + cropheight / 2;
        let rotatedheight = rotatedwidth - 1;

        let mut out: Vec<u16> = alloc_image_plain!(rotatedwidth, rotatedheight, dummy);
        if !dummy {
          for row in 0..cropheight {
            let inb = &src[(row + y) * width + x..];
            for col in 0..cropwidth {
              let out_row = cropwidth - 1 - col + (row >> 1);
              let out_col = ((row + 1) >> 1) + col;
              out[out_row * rotatedwidth + out_col] = inb[col];
            }
          }
        }

        Ok((rotatedwidth, rotatedheight, out))
      }
    } else {
      Err(RawlerError::General("no active_area for fuji_rotate".to_string()))
    }
  }
}

crate::tags::tiff_tag_enum!(RafMakernotes);
crate::tags::tiff_tag_enum!(RafIFD);

/// Specific RAF Makernotes tags.
/// These are only related to the Makernote IFD.
#[derive(Debug, Copy, Clone, PartialEq, enumn::N)]
#[repr(u16)]
#[allow(non_camel_case_types)]
pub enum RafMakernotes {
  Version = 0x0000,
  InternalSerialNumber = 0x0010,
  Quality = 0x0100,
  // TODO: more
}

/// These are only related to the Makernote IFD.
#[derive(Debug, Copy, Clone, PartialEq, enumn::N)]
#[repr(u16)]
#[allow(non_camel_case_types)]
pub enum RafIFD {
  FujiIFD = 0xf000,
  RawImageFullWidth = 0xf001,
  RawImageFullHeight = 0xf002,
  BitsPerSample = 0xf003,
  StripOffsets = 0xf007,
  StripByteCounts = 0xf008,
  BlackLevel = 0xf00a,
  GeometricDistortionParams = 0xf00b,
  WB_GRBLevelsStandard = 0xf00c,
  WB_GRBLevelsAuto = 0xf00d,
  WB_GRBLevels = 0xf00e,
  ChromaticAberrationParams = 0xf00f,
  VignettingParams = 0xf010,
}
