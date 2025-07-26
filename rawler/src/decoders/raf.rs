use byteorder::BigEndian;
use byteorder::LittleEndian;
use byteorder::ReadBytesExt;
use image::DynamicImage;
use std::collections::BTreeMap;
use std::io::Cursor;
use std::io::Seek;
use std::io::SeekFrom;
use std::mem::size_of;

use crate::CFA;
use crate::RawImage;
use crate::RawLoader;
use crate::RawlerError;
use crate::Result;
use crate::alloc_image;
use crate::alloc_image_plain;
use crate::analyze::FormatDump;
use crate::bits::BEu32;
use crate::bits::Endian;
use crate::decoders::raf::fuji_decompressor::decompress_fuji;
use crate::exif::Exif;
use crate::formats::jfif::Jfif;
use crate::formats::tiff::ifd::OffsetMode;
use crate::formats::tiff::*;
use crate::imgop::Dim2;
use crate::imgop::Point;
use crate::imgop::Rect;
use crate::packed::*;
use crate::pixarray::PixU16;
use crate::rawimage::BlackLevel;
use crate::rawimage::CFAConfig;
use crate::rawimage::RawPhotometricInterpretation;
use crate::rawimage::WhiteLevel;
use crate::rawsource::RawSource;
use crate::tags::DngTag;
use crate::tags::ExifTag;
use crate::tags::TiffCommonTag;

use super::Camera;
use super::Decoder;
use super::FormatHint;
use super::RawDecodeParams;
use super::RawMetadata;

mod dbp;
mod fuji_decompressor;

/// RAF decoder
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RafDecoder<'a> {
  #[allow(unused)]
  rawloader: &'a RawLoader,
  ifd: IFD,
  makernotes: IFD,
  camera: Camera,
}

/// Check if file has RAF signature
pub fn is_raf(file: &RawSource) -> bool {
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
/// Unfortunately, Fujifilm forgot to add a type field to these
/// tags, so we need to match by tag.
pub fn parse_raf_format(file: &RawSource, offset: u32) -> Result<IFD> {
  let mut entries = BTreeMap::new();
  let stream = &mut file.reader();
  stream.seek(SeekFrom::Start(offset as u64))?;
  let num = stream.read_u32::<BigEndian>()?; // Directory entries in this IFD
  if num > 4000 {
    return Err(format_args!("too many entries in IFD ({})", num).into());
  }
  for _ in 0..num {
    let tag = stream.read_u16::<BigEndian>()?;
    let len = stream.read_u16::<BigEndian>()? as usize;
    //eprintln!("RAF tag: 0x{:X}, len: {}", tag, len);

    match RafTags::try_from(tag) {
      Ok(RafTags::RawImageFullSize)
      | Ok(RafTags::RawImageCropTopLeft)
      | Ok(RafTags::RawImageCroppedSize)
      | Ok(RafTags::RawImageAspectRatio)
      | Ok(RafTags::WB_GRGBLevels) => {
        let n = len / size_of::<u16>();
        let entry = Entry {
          tag,
          value: Value::Short((0..n).map(|_| stream.read_u16::<BigEndian>()).collect::<std::io::Result<Vec<_>>>()?),
          embedded: None,
        };
        entries.insert(tag, entry);
      }
      Ok(RafTags::FujiLayout) | Ok(RafTags::XTransLayout) => {
        let n = len / size_of::<u8>();
        let entry = Entry {
          tag,
          value: Value::Byte((0..n).map(|_| stream.read_u8()).collect::<std::io::Result<Vec<_>>>()?),
          embedded: None,
        };
        entries.insert(tag, entry);
      }
      // This one is in other byte-order...
      Ok(RafTags::RAFData) => {
        let n = len / size_of::<u32>();
        let entry = Entry {
          tag,
          value: Value::Long((0..n).map(|_| stream.read_u32::<LittleEndian>()).collect::<std::io::Result<Vec<_>>>()?),
          embedded: None,
        };
        entries.insert(tag, entry);
      }
      // Skip other tags
      _ => {
        stream.seek(SeekFrom::Current(len as i64))?;
      }
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
fn parse_raf(file: &RawSource) -> Result<IFD> {
  const RAF_TIFF1_PTR_OFFSET: u64 = 84;
  const RAF_TIFF2_PTR_OFFSET: u64 = 100;
  const RAF_TAGS_PTR_OFFSET: u64 = 92;
  //const RAF_BLOCK_PTR_OFFSET2: u64 = 120; TODO: ?!?
  log::debug!("parse RAF");
  let stream = &mut file.reader();
  stream.seek(SeekFrom::Start(RAF_TIFF1_PTR_OFFSET))?;
  let offset = stream.read_u32::<BigEndian>()?;

  // Main IFD
  let mut main = IFD::new_root(stream, offset + 12)?;

  //main.dump::<TiffCommonTag>(10).iter().for_each(|line| eprintln!("MAIN: {}", line));

  // There is a second TIFF structure, the pointer is stored at offset 100.
  // If it is not a valid TIFF structure, the pointer itself is the RAF offset.
  stream.seek(SeekFrom::Start(RAF_TIFF2_PTR_OFFSET))?;
  let ioffset = stream.read_u32::<BigEndian>()?;

  match IFD::new_root_with_correction(stream, 0, ioffset, 0, 10, &[FujiIFD::FujiIFD.into()]) {
    Ok(val) => {
      log::debug!("Found valid FujiIFD (0xF000)");
      //val.dump::<FujiIFD>(10).iter().for_each(|line| eprintln!("FujiIFD: {}", line));
      main.sub.insert(FujiIFD::FujiIFD as u16, vec![val]);
    }
    Err(_) => {
      // We fake an FujiIFD to pass the StripOffsets
      log::debug!("Unable to find FujiIFD (0xF000), let's fake it");
      let mut entries = BTreeMap::<u16, Entry>::new();
      entries.insert(
        FujiIFD::StripOffsets as u16,
        Entry {
          tag: FujiIFD::StripOffsets as u16,
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
      main.sub.insert(FujiIFD::FujiIFD as u16, vec![fake]);
    }
  }
  // And we maybe have a RAF data block, try to parse it.
  stream.seek(SeekFrom::Start(RAF_TAGS_PTR_OFFSET))?;
  let raf_offset = stream.read_u32::<BigEndian>()?;
  match parse_raf_format(file, raf_offset) {
    Ok(val) => {
      //val.dump::<RafTags>(10).iter().for_each(|line| eprintln!("RAFTAGS: {}", line));
      main.sub.insert(RAF_TAG_VIRTUAL_RAF_DATA, vec![val]);
    }
    Err(_) => {
      log::debug!("RAF block pointer is not valid, ignoring");
    }
  }

  Ok(main)
}

impl<'a> RafDecoder<'a> {
  pub fn new(file: &RawSource, rawloader: &'a RawLoader) -> Result<RafDecoder<'a>> {
    let ifd = parse_raf(file)?;
    let camera = rawloader.check_supported(&ifd)?;
    let makernotes = if let Some(exif) = ifd.find_first_ifd_with_tag(ExifTag::MakerNotes) {
      exif.parse_makernote(&mut file.reader(), OffsetMode::Absolute, &[])?
    } else {
      None
    }
    .ok_or("File has not makernotes")?;
    //makernotes.dump::<RafMakernotes>(10).iter().for_each(|line| eprintln!("MKND: {}", line));
    Ok(RafDecoder {
      ifd,
      makernotes,
      rawloader,
      camera,
    })
  }
}

impl<'a> Decoder for RafDecoder<'a> {
  fn raw_image(&self, file: &RawSource, _params: &RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let raw = self.ifd.find_first_ifd_with_tag(FujiIFD::StripOffsets).ok_or("No StripOffsets found")?;
    let (width, height) = if raw.has_entry(FujiIFD::RawImageFullWidth) {
      (
        fetch_tiff_tag!(raw, FujiIFD::RawImageFullWidth).force_usize(0),
        fetch_tiff_tag!(raw, FujiIFD::RawImageFullHeight).force_usize(0),
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

    // Rotation is only used for SuperCCD sensors, so we handle X-Trans CFA only here.
    // Some cameras like X-T20 uses different CFA when compression is enabled, so we
    // read the correct pattern from metadata.
    let corrected_cfa = if let Some(cfa) = self.get_xtrans_cfa()? {
      log::debug!(
        "Found X-Trans CFA pattern in metadata, use this instead of camera config file. Pattern is: {}",
        cfa
      );
      cfa
    } else {
      self.camera.cfa.clone()
    };

    // Strip offset is relative to IFD base
    let offset = raw.base as u64 + fetch_tiff_tag!(raw, FujiIFD::StripOffsets).force_u64(0);
    let src = if raw.has_entry(FujiIFD::StripByteCounts) {
      let strip_count = fetch_tiff_tag!(raw, FujiIFD::StripByteCounts).force_u64(0);
      file.subview_padded(offset, strip_count)?
    } else {
      // Some models like DBP don't have a byte count, so we read until EOF
      file.subview_until_eof_padded(offset)?
    };

    log::debug!("BPS: {}, width: {}, height: {}, offset: {}", bps, width, height, offset);

    let image = if self.camera.find_hint("double_width") {
      // Some fuji SuperCCD cameras include a second raw image next to the first one
      // that is identical but darker to the first. The two combined can produce
      // a higher dynamic range image. Right now we're ignoring it.
      decode_16le_skiplines(&src, width, height, dummy)
    } else if self.camera.find_hint("jpeg32") {
      match bps {
        12 => decode_12be_msb32(&src, width, height, dummy),
        14 => decode_14be_msb32(&src, width, height, dummy),
        _ => return Err(RawlerError::unsupported(&self.camera, format!("RAF: Don't know how to decode bps {}", bps))),
      }
    } else if self.camera.clean_model == "DBP for GX680" {
      assert_eq!(bps, 16);
      dbp::decode_dbp(&src, width, height, dummy)?
    } else if src.len() < bps * width * height / 8 {
      if !dummy {
        decompress_fuji(&src, width, height, bps, &corrected_cfa)?
      } else {
        alloc_image_plain!(width, height, dummy)
      }
    } else {
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

    let blacklevel = self.get_blacklevel(&corrected_cfa)?;
    log::debug!("RAF Blacklevels: {:?}", blacklevel);

    // For now, we put the rotated data into DNG. Much better solution
    // would be to support staggered layouts, but this is not used much
    // and complicated to implement, because we need rectangular CFA patterns like 2x4.
    // The code path for staggered data is already implemented here, but remains unused.
    let rotate_for_dng = false;
    let cpp = 1;
    if self.camera.find_hint("fuji_rotation") || self.camera.find_hint("fuji_rotation_alt") {
      log::debug!("Apply Fuji image rotation");
      let rotated = if rotate_for_dng {
        if self.camera.find_hint("fuji_rotation") {
          fuji_raw_rotate(&image, dummy) // Only required for fuji_rotation
        } else {
          image
        }
      } else {
        self.rotate_image(image.pixels(), &self.camera, width, height, dummy)?
      };

      let mut camera = self.camera.clone();
      camera.cfa = corrected_cfa;
      let photometric = RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&camera));
      let mut image = RawImage::new(
        self.camera.clone(),
        rotated,
        cpp,
        normalize_wb(self.get_wb()?),
        photometric,
        blacklevel,
        None,
        dummy,
      );

      if rotate_for_dng {
        image.add_dng_tag(TiffCommonTag::CFARepeatPatternDim, [2, 4]);
        image.add_dng_tag(DngTag::CFALayout, 2_u16);
        image.add_dng_tag(TiffCommonTag::CFAPattern, &[0_u8, 1, 2, 1, 2, 1, 0, 1][..]);

        todo!();
        //image.add_dng_tag(DngTag::BlackLevel, image.blacklevel[0]);
        //image.add_dng_tag(DngTag::BlackLevelRepeatDim, [1_u16, 1_u16]);
      }

      // Reset crops because we have rotated the data.
      image.active_area = None;
      image.crop_area = None;
      Ok(image)
    } else {
      //ok_image(self.camera.clone(), width, height, cpp, self.get_wb()?, image.into_inner())

      let mut camera = self.camera.clone();
      camera.cfa = corrected_cfa;
      let whitelevel = if self.camera.whitelevel.is_none() {
        match bps {
          12 | 14 | 16 => {
            let max_value: u32 = (1_u32 << bps) - 1;
            Some(WhiteLevel::new(vec![max_value; cpp]))
          }
          _ => None,
        }
      } else {
        None
      };
      let photometric = RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&camera));
      let mut image = RawImage::new(camera, image, cpp, normalize_wb(self.get_wb()?), photometric, blacklevel, whitelevel, dummy);

      // Overwrite crop if available in metadata
      if let Some(crop) = self.get_crop()? {
        log::debug!("RAW file metadata contains crop info, overriding toml definitions: {:?}", crop);
        image.crop_area = Some(crop);
      }

      // Ideally, someone would expect that area is at bayer pattern
      // boundary. This is not the case, so we don't check this here.
      // if let Some(_area) = image.active_area.as_ref() {
      //   assert_eq!(area.d.w % image.cfa.width , 0);
      //   assert_eq!(area.d.h % image.cfa.height , 0);
      // }
      Ok(image)
    }
  }

  fn raw_metadata(&self, _file: &RawSource, _params: &RawDecodeParams) -> Result<RawMetadata> {
    let mut exif = Exif::new(&self.ifd)?;
    // Fuji RAF has all EXIF tags we need and there is no LensID or something
    // we can lookup. So this is an exception, we just pass the information.
    // TODO: better imeplement LensData::from_exif()?
    if let Some(ifd) = self.ifd.get_sub_ifd(TiffCommonTag::ExifIFDPointer) {
      exif.lens_make = ifd.get_entry(ExifTag::LensMake).and_then(|entry| entry.as_string().cloned());
      exif.lens_model = ifd.get_entry(ExifTag::LensModel).and_then(|entry| entry.as_string().cloned());
      exif.lens_spec = ifd.get_entry(ExifTag::LensSpecification).and_then(|entry| match &entry.value {
        Value::Rational(data) => Some([data[0], data[1], data[2], data[3]]),
        _ => None,
      });
    }
    let mdata = RawMetadata::new(&self.camera, exif);
    Ok(mdata)
  }

  fn xpacket(&self, file: &RawSource, _params: &RawDecodeParams) -> Result<Option<Vec<u8>>> {
    let jpeg_buf = self.read_embedded_jpeg(file)?;
    let mut cur = Cursor::new(jpeg_buf);
    let jfif = Jfif::parse(&mut cur)?;
    match jfif.xpacket().cloned() {
      Some(xpacket) => {
        log::debug!("Found XPacket data in embedded JPEG preview");
        Ok(Some(xpacket))
      }
      None => {
        log::debug!("Found no XPacket data");
        Ok(None)
      }
    }
  }

  fn full_image(&self, file: &RawSource, params: &RawDecodeParams) -> Result<Option<DynamicImage>> {
    if params.image_index != 0 {
      return Ok(None);
    }
    let jpeg_buf = self.read_embedded_jpeg(file)?;
    let img = image::load_from_memory_with_format(jpeg_buf, image::ImageFormat::Jpeg)
      .map_err(|err| RawlerError::DecoderFailed(format!("Failed to read JPEG: {:?}", err)))?;
    Ok(Some(img))
  }

  fn format_dump(&self) -> FormatDump {
    todo!()
  }

  fn format_hint(&self) -> FormatHint {
    FormatHint::RAF
  }
}

impl<'a> RafDecoder<'a> {
  fn get_wb(&self) -> Result<[f32; 4]> {
    let raw = self.ifd.find_first_ifd_with_tag(FujiIFD::StripOffsets).ok_or("No StripOffsets found")?;
    match raw.get_entry(FujiIFD::WB_GRBLevels) {
      Some(levels) => Ok([levels.force_f32(1), levels.force_f32(0), levels.force_f32(0), levels.force_f32(2)]),
      None => {
        let raf = &self
          .ifd
          .sub_ifds()
          .get(&RAF_TAG_VIRTUAL_RAF_DATA)
          .and_then(|ifds| ifds.get(0))
          .ok_or("No RAF data IFD found")?;
        let levels = fetch_tiff_tag!(raf, TiffCommonTag::RafOldWB);
        Ok([levels.force_f32(1), levels.force_f32(0), levels.force_f32(0), levels.force_f32(3)])
      }
    }
  }

  fn get_blacklevel(&self, cfa: &CFA) -> Result<Option<BlackLevel>> {
    if let Some(fuji) = self.ifd.get_sub_ifd(FujiIFD::FujiIFD) {
      if let Some(Entry { value: Value::Long(black), .. }) = fuji.get_entry_recursive(FujiIFD::BlackLevel) {
        let levels: Vec<u16> = black.iter().copied().map(|v| v as u16).collect();
        return Ok(Some(BlackLevel::new(&levels, cfa.width, cfa.height, 1)));
      } else {
        log::debug!("Unable to find black level data");
      }
    }
    Ok(None)
  }

  /// Get crop from metadata
  /// Nearly all models have this parameter, except of FinePix HS10
  fn get_crop(&self) -> Result<Option<Rect>> {
    if let Some(raf) = &self.ifd.sub_ifds().get(&RAF_TAG_VIRTUAL_RAF_DATA).and_then(|ifds| ifds.get(0)) {
      let crops = raf.get_entry(RafTags::RawImageCropTopLeft);
      let size = raf.get_entry(RafTags::RawImageCroppedSize);
      if let (Some(crops), Some(size)) = (crops, size) {
        return Ok(Some(Rect::new(
          Point::new(crops.force_usize(1), crops.force_usize(0)),
          Dim2::new(size.force_usize(1), size.force_usize(0)),
        )));
      }
    }
    Ok(None)
  }

  /// Get the X-Trans CFA pattern
  /// This is encoded in RAF metadata block in XTransLayout.
  /// For unknown reason, the values are stored in reverse order and
  /// also falsely reported by exiftoool.
  fn get_xtrans_cfa(&self) -> Result<Option<CFA>> {
    Ok(
      if let Some(raf) = &self
        .ifd
        .sub_ifds()
        .get(&RAF_TAG_VIRTUAL_RAF_DATA)
        .and_then(|ifds| ifds.get(0).and_then(|ifd| ifd.get_entry(RafTags::XTransLayout)))
      {
        match &raf.value {
          Value::Byte(data) => {
            let patname: String = data
              .iter()
              .rev()
              .map(|v| match v {
                0 => 'R',
                1 => 'G',
                2 => 'B',
                _ => 'X', // Unknown, let CFA::new() fail...
              })
              .collect();
            Some(CFA::new(&patname))
          }
          _ => {
            return Err("Invalid XTransLayout data type".into());
          }
        }
      } else {
        None
      },
    )
  }

  fn read_embedded_jpeg<'b>(&self, file: &'b RawSource) -> Result<&'b [u8]> {
    // The offset and len of JPEG preview is in the RAF structure
    let buf = file.subview(0, 84 + 8)?;
    let jpeg_off = BEu32(buf, 84) as u64;
    let jpeg_len = BEu32(buf, 84 + 4) as u64;
    log::debug!("JPEG off: {}, len: {}", jpeg_off, jpeg_len);
    Ok(file.subview(jpeg_off, jpeg_len)?)
  }

  fn rotate_image(&self, src: &[u16], camera: &Camera, width: usize, height: usize, dummy: bool) -> Result<PixU16> {
    if let Some(active_area) = self.camera.active_area {
      let x = active_area[0];
      let y = active_area[1];
      let cropwidth = width - active_area[2] - x;
      let cropheight = height - active_area[3] - y; // TODO: bug, invalid order of crop index

      if camera.find_hint("fuji_rotation_alt") {
        let rotatedwidth = cropheight + cropwidth / 2;
        let rotatedheight = rotatedwidth - 1;

        let mut out = alloc_image_plain!(rotatedwidth, rotatedheight, dummy);
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
        Ok(out)
      } else {
        let rotatedwidth = cropwidth + cropheight / 2;
        let rotatedheight = rotatedwidth - 1;

        let mut out = alloc_image_plain!(rotatedwidth, rotatedheight, dummy);
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
        Ok(out)
      }
    } else {
      Err(RawlerError::DecoderFailed("no active_area for fuji_rotate".to_string()))
    }
  }
}

fn normalize_wb(raw_wb: [f32; 4]) -> [f32; 4] {
  log::debug!("RAF raw wb: {:?}", raw_wb);
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

crate::tags::tiff_tag_enum!(RafMakernotes);
crate::tags::tiff_tag_enum!(FujiIFD);
crate::tags::tiff_tag_enum!(RafTags);

/// Specific RAF Makernotes tags.
/// These are only related to the Makernote IFD.
#[derive(Debug, Copy, Clone, PartialEq, enumn::N)]
#[repr(u16)]
#[allow(non_camel_case_types)]
pub enum RafMakernotes {
  Version = 0x0000,
  InternalSerialNumber = 0x0010,
  Quality = 0x1000,
  Sharpness = 0x1001,
  WhiteBalance = 0x1002,
  Saturation = 0x1003,
  Contrast = 0x1004,
  ColorTemperature = 0x1005,
  Contrast2 = 0x1006,
  WhiteBalanceFineTune = 0x100a,
  NoiseReduction = 0x100b,
  NoiseReduction2 = 0x100e,
  FujiFlashMode = 0x1010,
  FlashExposureComp = 0x1011,
  Macro = 0x1020,
  FocusMode = 0x1021,
  AFMode = 0x1022,
  FocusPixel = 0x1023,
  PrioritySettings = 0x102b,
  FocusSettings = 0x102d,
  AFCSettings = 0x102e,
  SlowSync = 0x1030,
  PictureMode = 0x1031,
  ExposureCount = 0x1032,
  EXRAuto = 0x1033,
  EXRMode = 0x1034,
  ShadowTone = 0x1040,
  HighlightTone = 0x1041,
  DigitalZoom = 0x1044,
  LensModulationOptimizer = 0x1045,
  GrainEffect = 0x1047,
  ColorChromeEffect = 0x1048,
  BWAdjustment = 0x1049,
  CropMode = 0x104d,
  ColorChromeFXBlue = 0x104e,
  ShutterType = 0x1050,
  AutoBracketing = 0x1100,
  SequenceNumber = 0x1101,
  DriveSettings = 0x1103,
  PixelShiftShots = 0x1105,
  PixelShiftOffset = 0x1106,
  PanoramaAngle = 0x1153,
  PanoramaDirection = 0x1154,
  AdvancedFilter = 0x1201,
  ColorMode = 0x1210,
  BlurWarning = 0x1300,
  FocusWarning = 0x1301,
  ExposureWarning = 0x1302,
  GEImageSize = 0x1304,
  DynamicRange = 0x1400,
  FilmMode = 0x1401,
  DynamicRangeSetting = 0x1402,
  DevelopmentDynamicRange = 0x1403,
  MinFocalLength = 0x1404,
  MaxFocalLength = 0x1405,
  MaxApertureAtMinFocal = 0x1406,
  MaxApertureAtMaxFocal = 0x1407,
  AutoDynamicRange = 0x140b,
  ImageStabilization = 0x1422,
  SceneRecognition = 0x1425,
  Rating = 0x1431,
  ImageGeneration = 0x1436,
  ImageCount = 0x1438,
  DRangePriority = 0x1443,
  DRangePriorityAuto = 0x1444,
  DRangePriorityFixed = 0x1445,
  FlickerReduction = 0x1446,
  VideoRecordingMode = 0x3803,
  PeripheralLighting = 0x3804,
  VideoCompression = 0x3806,
  FrameRate = 0x3820,
  FrameWidth = 0x3821,
  FrameHeight = 0x3822,
  FullHDHighSpeedRec = 0x3824,
  FaceElementSelected = 0x4005,
  FacesDetected = 0x4100,
  FacePositions = 0x4103,
  NumFaceElements = 0x4200,
  FaceElementTypes = 0x4201,
  FaceElementPositions = 0x4203,
  FaceRecInfo = 0x4282,
  FileSource = 0x8000,
  OrderNumber = 0x8002,
  FrameNumber = 0x8003,
  Parallax = 0xb211,
}

/// These are only related to the additional FujiIFD in RAF files
#[derive(Debug, Copy, Clone, PartialEq, enumn::N)]
#[repr(u16)]
#[allow(non_camel_case_types)]
pub enum FujiIFD {
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

/// These are only related to the additional RAF-tags in RAF files
#[derive(Debug, Copy, Clone, PartialEq, enumn::N)]
#[repr(u16)]
#[allow(non_camel_case_types)]
pub enum RafTags {
  RawImageFullSize = 0x0100,
  RawImageCropTopLeft = 0x0110,
  RawImageCroppedSize = 0x0111,
  RawImageAspectRatio = 0x0115,
  RawImageSize = 0x0121,
  FujiLayout = 0x0130,
  XTransLayout = 0x0131,
  WB_GRGBLevels = 0x2ff0,
  RelativeExposure = 0x9200,
  RawExposureBias = 0x9650,
  RAFData = 0xc000,
}

pub fn fuji_raw_rotate(img: &PixU16, dummy: bool) -> PixU16 {
  let mut out = alloc_image!(img.height, img.width, dummy);
  for row in 0..img.height {
    for col in 0..img.width {
      //*x.at_mut(row, col) = out[flip_index(row, col)];
      //*x.at_mut(row, col) = out[(width - 1 - col) * height + (height - 1 - row)];
      //*out.at_mut(img.width - 1 - col, img.height - 1 - row) = *img.at(row, col); //   out[row * width + col];
      *out.at_mut(col, row) = *img.at(row, col); //   out[row * width + col];
    }
  }
  out
}
