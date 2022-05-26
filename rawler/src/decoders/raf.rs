use byteorder::BigEndian;
use byteorder::ReadBytesExt;
use image::DynamicImage;
use std::collections::BTreeMap;
use std::f32::NAN;
use std::io::Cursor;
use std::io::SeekFrom;
use std::mem::swap;

use crate::alloc_image_plain;
use crate::analyze::FormatDump;
use crate::bits::BEu32;
use crate::bits::Endian;
use crate::bits::LEu32;
use crate::decoders::ok_image_with_blacklevels;
use crate::exif::Exif;
use crate::formats::tiff::ifd::OffsetMode;
use crate::formats::tiff::*;
use crate::packed::*;
use crate::pixarray::PixU16;
use crate::tags::ExifTag;
use crate::tags::TiffCommonTag;
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

    //makernote.dump::<RafMakernotes>(0).iter().for_each(|line| eprintln!("DUMP: {}", line)); // TODO: remove

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
    } else {
      if src.len() < bps * width * height / 8 {
        //PixU16::new(width, height)
        return Err(RawlerError::unsupported(&self.camera, "RAF: Don't know how to decode compressed yet"));
      } else {
        match bps {
          12 => decode_12le(&src, width, height, dummy),
          14 => decode_14le_unpacked(&src, width, height, dummy),
          16 => {
            //deocde_dbp(&src, width, height, dummy) TODO

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
      }
    };

    let blacks = self.get_blacklevel()?.unwrap_or(self.camera.blacklevels);
    log::debug!("RAF Blacklevels: {:?}", blacks);

    let cpp = 1;
    if self.camera.find_hint("fuji_rotation") || self.camera.find_hint("fuji_rotation_alt") {
      log::debug!("Apply Fuji image rotation");
      let (width, height, image) = self.rotate_image(image.pixels(), &self.camera, width, height, dummy)?;

      let mut image = RawImage::new(self.camera.clone(), width, height, cpp, normalize_wb(self.get_wb()?), image, dummy);
      image.blacklevels = blacks;
      if bps != 0 {
        //image.bps = bps; // TODO
      }
      // Reset crops because we have rotated the data.
      image.active_area = None;
      image.crop_area = None;
      Ok(image)
    } else {
      //ok_image(self.camera.clone(), width, height, cpp, self.get_wb()?, image.into_inner())
      ok_image_with_blacklevels(
        self.camera.clone(),
        image.width,
        image.height,
        cpp,
        normalize_wb(self.get_wb()?),
        blacks,
        image.into_inner(),
      )
    }
  }

  fn raw_metadata(&self, _file: &mut RawFile, _params: RawDecodeParams) -> Result<RawMetadata> {
    let mut exif = Exif::new(&self.ifd)?;
    // Fuji RAF has all EXIF tags we need and there is no LensID or something
    // we can lookup. So this is an exception, we just pass the information.
    // TODO: better imeplement LensData::from_exif()?
    if let Some(ifd) = self.ifd.get_sub_ifds(TiffCommonTag::ExifIFDPointer) {
      exif.lens_make = ifd[0].get_entry(ExifTag::LensMake).and_then(|entry| entry.as_string().cloned());
      exif.lens_model = ifd[0].get_entry(ExifTag::LensModel).and_then(|entry| entry.as_string().cloned());
      exif.lens_spec = ifd[0].get_entry(ExifTag::LensSpecification).and_then(|entry| match &entry.value {
        Value::Rational(data) => Some([data[0], data[1], data[2], data[3]]),
        _ => None,
      });
    }
    let mdata = RawMetadata::new(&self.camera, exif);
    Ok(mdata)
  }

  fn full_image(&self, file: &mut RawFile) -> Result<Option<DynamicImage>> {
    // The offset and len of JPEG preview is in the RAF structure
    let buf = file.subview(0, 84 + 8)?;
    let jpeg_off = BEu32(&buf, 84) as u64;
    let jpeg_len = BEu32(&buf, 84 + 4) as u64;
    log::debug!("JPEG off: {}, len: {}", jpeg_off, jpeg_len);
    let jpeg = file.subview(jpeg_off, jpeg_len)?;
    let img = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg).unwrap();
    Ok(Some(img))
  }

  fn format_dump(&self) -> FormatDump {
    todo!()
  }
}

pub fn deocde_dbp(buf: &[u8], width: usize, height: usize, dummy: bool) -> PixU16 {
  let mut out = vec![0_u16; width * height];

  let mut cursor = Cursor::new(buf);

  let nTiles = 8;
  let tile_width = width / nTiles;
  let tile_height = 3856;

  log::error!("width: {}, height: {}, tile: {}", width, height, tile_width);

  let mut tile = vec![0_u16; height * tile_width];

  for tile_n in 0..nTiles {
    cursor.read_u16_into::<BigEndian>(&mut tile).unwrap();
    for scan_line in 0..height {
      let off = scan_line * width + tile_n * tile_width;
      out[off..off + tile_width].copy_from_slice(&tile[scan_line * tile_width..scan_line * tile_width + tile_width]);

      //  memcpy(&raw_image[scan_line * raw_width + tile_n * tile_width],
      //    &tile[scan_line * tile_width], tile_width * 2);
    }
  }

  /*
  for Fuji DBP for GX680, aka DX-2000
    DBP_tile_width = 688;
    DBP_tile_height = 3856;
    DBP_n_tiles = 8;

  {
    int scan_line, tile_n;
    int nTiles;

    nTiles = 8;
    tile_width = raw_width / nTiles;

    ushort *tile;
    tile = (ushort *)calloc(raw_height, tile_width * 2);

    for (tile_n = 0; tile_n < nTiles; tile_n++)
    {
      read_shorts(tile, tile_width * raw_height);
      for (scan_line = 0; scan_line < raw_height; scan_line++)
      {
        memcpy(&raw_image[scan_line * raw_width + tile_n * tile_width],
               &tile[scan_line * tile_width], tile_width * 2);
      }
    }
    free(tile);
    fseek(ifp, -2, SEEK_CUR); // avoid EOF error
  }
  */

  //let mut x = PixU16::new(width, height);
  let mut x = PixU16::new(height, width);

  let flip_index = |row: usize, col: usize| -> usize {
    let irow = width - 1 - col;
    let icol = height - 1 - row;
    irow * height + icol
  };

  for row in 0..height {
    for col in 0..width {
      //*x.at_mut(row, col) = out[flip_index(row, col)];
      //*x.at_mut(row, col) = out[(width - 1 - col) * height + (height - 1 - row)];
      *x.at_mut(width - 1 - col, height - 1 - row) = out[row * width + col];
    }
  }

  //x
  PixU16::new_with(out, width, height)
}

impl<'a> RafDecoder<'a> {
  fn get_wb(&self) -> Result<[f32; 4]> {
    let raw = self.ifd.find_first_ifd_with_tag(RafIFD::StripOffsets).ok_or("No StripOffsets found")?;
    match raw.get_entry(RafIFD::WB_GRBLevels) {
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

  fn get_blacklevel(&self) -> Result<Option<[u16; 4]>> {
    if let Some(ifd) = self.ifd.get_sub_ifds(RafIFD::FujiIFD) {
      let fuji = &ifd[0];
      if let Some(black) = fuji.get_entry(RafIFD::BlackLevel) {
        return Ok(Some([black.force_u16(0), black.force_u16(1), black.force_u16(2), black.force_u16(3)]));
      }
    }
    Ok(None)
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
  [norm[0], (norm[1] + norm[2]) / 2.0, norm[3], NAN]
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
