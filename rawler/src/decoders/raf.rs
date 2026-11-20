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
use crate::alloc_image_plain;
use crate::analyze::FormatDump;
use crate::bits::BEu32;
use crate::bits::Endian;
use crate::cfa::CFALayout;
use crate::decoders::raf::fuji_decompressor::decompress_fuji;
use crate::decompressors::packed::*;
use crate::exif::Exif;
use crate::formats::jfif::Jfif;
use crate::formats::tiff::ifd::OffsetMode;
use crate::formats::tiff::*;
use crate::imgop::Dim2;
use crate::imgop::Point;
use crate::imgop::Rect;
use crate::imgop::stagger::Stagger;
use crate::pixarray::PixU16;
use crate::rawimage::BlackLevel;
use crate::rawimage::CFAConfig;
use crate::rawimage::DefaultCrop;
use crate::rawimage::RawPhotometricInterpretation;
use crate::rawimage::WhiteLevel;
use crate::rawsource::RawSource;
use crate::tags::ExifTag;
use crate::tags::TiffCommonTag;

use super::Camera;
use super::CameraLookup;
use super::Decoder;
use super::FormatHint;
use super::Orientation;
use super::RawDecodeParams;
use super::RawMetadata;
use super::WellKnownIFD;
use super::image_bytes;
use super::makernotes;
use super::tiff_ifd;
use std::sync::Arc;

mod dbp;
mod fuji_decompressor;

/// RAF decoder
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RafDecoder<'a> {
  #[allow(unused)]
  rawloader: &'a RawLoader,
  ifd: IFD,
  makernotes: Arc<IFD>,
  camera: CameraLookup,
  camera_compressed: Option<Camera>,
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
    return Err(decoder_error!("too many entries in IFD ({})", num));
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

// This won't work for many samples, don't use it.
#[allow(dead_code)]
fn get_compression(file: &RawSource) -> Result<u32> {
  let buf = file.subview(0, 0x6c + 4)?;
  let compression = BEu32(buf, 0);
  Ok(compression)
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
  let main_offset = offset
    .checked_add(12)
    .ok_or_else(|| decoder_error!("RAF: offset {} of the TIFF structure is out of range", offset))?;
  let mut main = IFD::new_root(stream, main_offset)?;

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

    /*
    let mode = match get_compression(file)? {
      0 => "uncompressed",
      1 => "lossess",
      2 => "lossy",
      _ => "unknown",
    };
     */

    let camera = rawloader.lookup_camera(&ifd)?;
    let camera_compressed = rawloader.check_supported_with_mode(&ifd, "compressed").ok();

    let makernotes = if let Some(exif) = ifd.find_first_ifd_with_tag(ExifTag::MakerNotes) {
      exif.parse_makernote(&mut file.reader(), OffsetMode::Absolute, &[])?
    } else {
      None
    }
    .ok_or_else(|| RawlerError::decoder_failed("File has not makernotes"))?;
    //makernotes.dump::<RafMakernotes>(10).iter().for_each(|line| eprintln!("MKND: {}", line));
    Ok(RafDecoder {
      ifd,
      makernotes: Arc::new(makernotes),
      rawloader,
      camera,
      camera_compressed,
    })
  }
}

impl<'a> Decoder for RafDecoder<'a> {
  fn raw_image(&self, file: &RawSource, _params: &RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let base_camera = self.camera.supported()?;
    let raw = self.get_raw_ifd()?;
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
        .ok_or_else(|| RawlerError::decoder_failed("No RAF data IFD found"))?;
      let sizes = fetch_tiff_tag!(raf, TiffCommonTag::ImageWidth);
      (sizes.force_usize(1), sizes.force_usize(0))
    };

    let bps = match raw.get_entry(TiffCommonTag::RafBitsPerSample) {
      Some(val) => val.force_u32(0) as usize,
      None => 16,
    };
    // The decoders and the white levels below support these bits per sample
    if !matches!(bps, 12 | 14 | 16) {
      return Err(RawlerError::unsupported(base_camera, format!("RAF: Don't know how to decode bps {}", bps)));
    }
    image_bytes("RAF", width, height, 16)?;

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
      base_camera.cfa.clone()
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

    let mut camera = base_camera.clone();

    let image = if base_camera.find_hint("double_width") {
      // Some fuji SuperCCD cameras include a second raw image next to the first one
      // that is identical but darker to the first. The two combined can produce
      // a higher dynamic range image. Right now we're ignoring it.
      decompress_16le_skiplines(&src, width, height, dummy).map_err(RawlerError::decoder_failed)?
    } else if base_camera.find_hint("jpeg32") {
      match bps {
        12 => decompress_12be_msb32(&src, width, height, dummy).map_err(RawlerError::decoder_failed)?,
        14 => decompress_14be_msb32(&src, width, height, dummy).map_err(RawlerError::decoder_failed)?,
        _ => return Err(RawlerError::unsupported(base_camera, format!("RAF: Don't know how to decode bps {}", bps))),
      }
    } else if base_camera.clean_model == "DBP for GX680" {
      if bps != 16 {
        return Err(RawlerError::unsupported(base_camera, format!("RAF: DBP data with {} bps", bps)));
      }
      dbp::decode_dbp(&src, width, height, dummy)?
    } else if src.len() < bps * width * height / 8 {
      if !dummy {
        //if let Some(camera) = rawloader.check_supported(&ifd)
        if let Some(cam_compr) = self.camera_compressed.clone() {
          camera = cam_compr;
        }
        decompress_fuji(&src, width, height, bps, &corrected_cfa)?
      } else {
        alloc_image_plain!(width, height, dummy)
      }
    } else {
      match bps {
        12 => decompress_12le(&src, width, height, dummy).map_err(RawlerError::decoder_failed)?,
        14 => decompress_14le_unpacked(&src, width, height, dummy).map_err(RawlerError::decoder_failed)?,
        16 => {
          if self.ifd.endian == Endian::Little {
            decompress_16le(&src, width, height, dummy).map_err(RawlerError::decoder_failed)?
          } else {
            decompress_16be(&src, width, height, dummy).map_err(RawlerError::decoder_failed)?
          }
        }
        _ => {
          return Err(RawlerError::unsupported(base_camera, format!("RAF: Don't know how to decode bps {}", bps)));
        }
      }
    };

    let blacklevel = self.get_blacklevel(&corrected_cfa)?;
    log::debug!("RAF Blacklevels: {:?}", blacklevel);

    let cpp = 1;
    if base_camera.find_hint("fuji_rotation") || base_camera.find_hint("fuji_rotation_alt") {
      self.staggered_image(image, base_camera, &corrected_cfa, blacklevel, dummy)
    } else {
      //ok_image(self.camera.clone(), width, height, cpp, self.get_wb()?, image.into_inner())

      //let mut camera = self.camera.clone();
      camera.cfa = corrected_cfa;
      let whitelevel = if base_camera.whitelevel.is_none() {
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
      image.orientation = Orientation::from_tiff(&self.ifd);

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

  fn check_supported(&self) -> Result<()> {
    self.camera.supported().map(|_| ())
  }

  fn raw_metadata(&self, _file: &RawSource, _params: &RawDecodeParams) -> Result<RawMetadata> {
    // Fuji RAF has all EXIF tags we need and there is no LensID or something
    // we can lookup. So this is an exception, we just pass the information.
    // The lens tags are parsed by Exif::new() from the EXIF IFD.
    let mut exif = Exif::new(&self.ifd)?;
    let camera = self.camera.metadata();
    if camera.find_hint("fuji_rotate_90cw") {
      // Writers use the orientation of the raw image, which includes the turn
      exif.orientation = self.orientation(camera).map(|orientation| orientation.to_u16());
    }
    let mdata = RawMetadata::new(camera, exif);
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

  fn preview_image(&self, file: &RawSource, params: &RawDecodeParams) -> Result<Option<DynamicImage>> {
    if params.image_index != 0 {
      return Ok(None);
    }
    let jpeg_buf = self.read_embedded_jpeg(file)?;
    let img = image::load_from_memory_with_format(jpeg_buf, image::ImageFormat::Jpeg).map_err(|err| decoder_error!("Failed to read JPEG: {:?}", err))?;
    Ok(Some(img))
  }

  fn format_dump(&self) -> FormatDump {
    todo!()
  }

  fn ifd(&self, _file: &RawSource, _params: &RawDecodeParams, wk_ifd: WellKnownIFD) -> Result<Option<Arc<IFD>>> {
    Ok(match wk_ifd {
      WellKnownIFD::Root | WellKnownIFD::Exif | WellKnownIFD::ExifGps => tiff_ifd(&self.ifd, wk_ifd),
      WellKnownIFD::Raw => Some(Arc::new(self.get_raw_ifd()?.clone())),
      WellKnownIFD::Makernote => Some(self.makernotes.clone()),
      WellKnownIFD::VirtualMakernotes => makernotes::from_exif(
        self.format_hint(),
        self.ifd.find_first_ifd_with_tag(ExifTag::MakerNotes),
        Some(&self.makernotes),
      )
      .map(Arc::new),
      _ => None,
    })
  }

  fn format_hint(&self) -> FormatHint {
    FormatHint::RAF
  }
}

impl<'a> RafDecoder<'a> {
  /// Orientation of the raw image. Cameras with `fuji_rotate_90cw` store the
  /// image turned by 90°, the EXIF orientation applies after the turn.
  fn orientation(&self, camera: &Camera) -> Option<Orientation> {
    let orientation = Orientation::from_tiff(&self.ifd);
    if camera.find_hint("fuji_rotate_90cw") {
      Some(Orientation::Rotate90.then(orientation.unwrap_or(Orientation::Normal)))
    } else {
      orientation
    }
  }

  /// The IFD with the raw image data
  fn get_raw_ifd(&self) -> Result<&IFD> {
    self
      .ifd
      .find_first_ifd_with_tag(FujiIFD::StripOffsets)
      .ok_or_else(|| RawlerError::decoder_failed("No StripOffsets found"))
  }

  fn get_wb(&self) -> Result<[f32; 4]> {
    let raw = self.get_raw_ifd()?;
    match raw.get_entry(FujiIFD::WB_GRBLevels) {
      Some(levels) => Ok([levels.force_f32(1), levels.force_f32(0), levels.force_f32(0), levels.force_f32(2)]),
      None => {
        let raf = &self
          .ifd
          .sub_ifds()
          .get(&RAF_TAG_VIRTUAL_RAF_DATA)
          .and_then(|ifds| ifds.get(0))
          .ok_or_else(|| RawlerError::decoder_failed("No RAF data IFD found"))?;
        let levels = fetch_tiff_tag!(raf, TiffCommonTag::RafOldWB);
        Ok([levels.force_f32(1), levels.force_f32(0), levels.force_f32(0), levels.force_f32(3)])
      }
    }
  }

  fn get_blacklevel(&self, cfa: &CFA) -> Result<Option<BlackLevel>> {
    if let Some(fuji) = self.ifd.get_sub_ifd(FujiIFD::FujiIFD) {
      if let Some(Entry { value: Value::Long(black), .. }) = fuji.get_entry_recursive(FujiIFD::BlackLevel) {
        let levels: Vec<u16> = black.iter().copied().map(|v| v as u16).collect();
        // There is one level for each position of the CFA
        if levels.len() != cfa.width * cfa.height {
          log::warn!(
            "RAF: {} black levels don't match the CFA of {}x{} pixels, ignoring them",
            levels.len(),
            cfa.width,
            cfa.height
          );
          return Ok(None);
        }
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
                _ => 'X', // Unknown, CFA::try_new() fails
              })
              .collect();
            let cfa = CFA::try_new(&patname);
            if cfa.is_none() {
              log::warn!("RAF: invalid XTransLayout \"{}\", using the CFA of the camera definition", patname);
            }
            cfa
          }
          _ => {
            return Err(RawlerError::decoder_failed("Invalid XTransLayout data type"));
          }
        }
      } else {
        None
      },
    )
  }

  /// Raw image of the staggered CFA data of a Super CCD sensor
  ///
  /// Every second row (`fuji_rotation`) or column (`fuji_rotation_alt`) of
  /// photosites is offset by half a photosite, CFALayout 4 or 2. The camera
  /// definition has the CFA pattern of the grid rotated by 45° and the crop of
  /// the image that the rotation back gave before the stage 3 grid was used,
  /// both are converted to the staggered layout. Like the DefaultScale of DNG,
  /// the scale of the camera refers to the staggered pixels, the rows or
  /// columns doubled by the stage 3 grid are part of it. The image of cameras with
  /// `fuji_rotate_90cw` is turned by 90° clockwise, which becomes part of the
  /// orientation.
  fn staggered_image(&self, image: PixU16, base_camera: &Camera, rotated_cfa: &CFA, blacklevel: Option<BlackLevel>, dummy: bool) -> Result<RawImage> {
    let alt = base_camera.find_hint("fuji_rotation_alt");
    if self.get_alt_layout() != alt {
      log::warn!("RAF: the layout of the file differs from the camera definition, using the camera definition");
    }
    let dim = Dim2::new(image.width, image.height);
    let area = match base_camera.active_area {
      Some(borders) => Rect::new_with_borders(dim, &borders),
      None => Some(Rect::new(Point::zero(), dim)),
    }
    .filter(|area| area.d.w >= 2 && area.d.h >= 2)
    .ok_or_else(|| {
      decoder_error!(
        "RAF: image of {}x{} pixels is too small for the active area {:?}",
        dim.w,
        dim.h,
        base_camera.active_area
      )
    })?;
    // The layout of the area, CFAConfig has it relative to the top-left sample
    let area_layout = if alt { CFALayout::StaggeredA } else { CFALayout::StaggeredC };
    let layout = area_layout.shift(area.p.x, area.p.y);
    let stagger = Stagger::new(layout, area)?.ok_or_else(|| decoder_error!("RAF: Super CCD layout is not staggered"))?;
    let at = |row, col| rotated_cfa.color_at(row, col) as u8;
    let (colors, width, height) = stagger.staggered_pattern([[at(0, 0), at(0, 1)], [at(1, 0), at(1, 1)]]);
    let to_sample_origin = ((width - area.p.x % width) % width, (height - area.p.y % height) % height);
    let cfa = CFA::from_colors(&colors, width, height)
      .ok_or_else(|| decoder_error!("RAF: CFA pattern {} gives no staggered pattern", rotated_cfa))?
      .shift(to_sample_origin.0, to_sample_origin.1);
    // Black levels are given for the positions of the rotated CFA
    let blacklevel = blacklevel.map(|black| {
      if black.width == 2 && black.height == 2 && black.cpp == 1 {
        let level = |row: usize, col: usize| black.levels[row * 2 + col];
        let (levels, width, height) = stagger.staggered_pattern([[level(0, 0), level(0, 1)], [level(1, 0), level(1, 1)]]);
        BlackLevel::new(&levels, width, height, 1).shift_pattern(to_sample_origin.0, to_sample_origin.1)
      } else {
        black
      }
    });

    let mut camera = base_camera.clone();
    camera.cfa = cfa;
    camera.default_scale = base_camera.default_scale.scaled(stagger.full_scale());
    let photometric = RawPhotometricInterpretation::Cfa(CFAConfig {
      layout,
      ..CFAConfig::new_from_camera(&camera)
    });
    let mut rawimage = RawImage::new(camera, image, 1, normalize_wb(self.get_wb()?), photometric, blacklevel, None, dummy);
    rawimage.orientation = self.orientation(base_camera);
    // The crop of the camera definition refers to the image of the earlier rotation
    rawimage.default_crop = base_camera
      .crop_area
      .and_then(|borders| staggered_crop(&stagger, area, &borders, alt, base_camera.find_hint("fuji_rotate_90cw")));
    rawimage.crop_area = rawimage.default_crop.and_then(|crop| crop.rect(area.d)).map(|crop| crop.translate(area.p));
    Ok(rawimage)
  }

  fn get_alt_layout(&self) -> bool {
    if let Some(entry) = &self
      .ifd
      .sub_ifds()
      .get(&RAF_TAG_VIRTUAL_RAF_DATA)
      .and_then(|ifds| ifds.get(0).and_then(|ifd| ifd.get_entry(RafTags::FujiLayout)))
    {
      (entry.force_u8(0) >> 7) == 0
    } else {
      log::debug!("RafTags::FujiLayout tag not found");
      false
    }
  }

  fn read_embedded_jpeg<'b>(&self, file: &'b RawSource) -> Result<&'b [u8]> {
    // The offset and len of JPEG preview is in the RAF structure
    let buf = file.subview(0, 84 + 8)?;
    let jpeg_off = BEu32(buf, 84) as u64;
    let jpeg_len = BEu32(buf, 84 + 4) as u64;
    log::debug!("JPEG off: {}, len: {}", jpeg_off, jpeg_len);
    Ok(file.subview(jpeg_off, jpeg_len)?)
  }
}

/// Size of the image that the rotation back of the rotated grid gave before
/// the stage 3 grid was used: the inscribed rectangle of the grid of
/// `rotated_width` columns with the split width `split`, always landscape
fn legacy_rotated_dimension(rotated_width: usize, split: usize) -> Dim2 {
  let crop_w = (split as f64 * std::f64::consts::SQRT_2).floor() as usize;
  let crop_h = (rotated_width.saturating_sub(split) as f64 * std::f64::consts::SQRT_2).floor() as usize;
  if crop_w > crop_h {
    Dim2::new(crop_w, crop_h)
  } else {
    Dim2::new(crop_h, crop_w)
  }
}

/// DefaultCrop of the staggered `area` for the crop `borders` (left, top,
/// right, bottom) of a camera definition, in raw coordinates relative to the area
///
/// The borders refer to the image that the rotation back gave before the
/// stage 3 grid was used, see [`legacy_rotated_dimension`]. They are scaled to
/// the stage 3 grid, and turned back by 90° for `extra_rotate`, which rotates
/// the stage 3 image clockwise. `None` if no area is left.
fn staggered_crop(stagger: &Stagger, area: Rect, borders: &[usize; 4], alt: bool, extra_rotate: bool) -> Option<DefaultCrop> {
  let (w, h) = (area.d.w, area.d.h);
  let (rotated_width, split) = if alt { (h + w / 2, h) } else { (w + h / 2, w) };
  let legacy = legacy_rotated_dimension(rotated_width, split);
  let stage3 = stagger.stage3();
  let rendered = if extra_rotate { Dim2::new(stage3.h, stage3.w) } else { stage3 };
  // The earlier image was always landscape
  let legacy = if (rendered.w >= rendered.h) == (legacy.w >= legacy.h) {
    legacy
  } else {
    Dim2::new(legacy.h, legacy.w)
  };
  if legacy.w == 0 || legacy.h == 0 {
    return None;
  }
  let (sx, sy) = (rendered.w as f64 / legacy.w as f64, rendered.h as f64 / legacy.h as f64);
  let scaled = [borders[0] as f64 * sx, borders[1] as f64 * sy, borders[2] as f64 * sx, borders[3] as f64 * sy].map(|v| v.round() as usize);
  // The borders of the stage 3 image before the rotation by 90° clockwise
  let [left, top, right, bottom] = if extra_rotate { [scaled[1], scaled[2], scaled[3], scaled[0]] } else { scaled };
  let width = stage3.w.checked_sub(left + right).filter(|w| *w > 0)?;
  let height = stage3.h.checked_sub(top + bottom).filter(|h| *h > 0)?;
  let (fx, fy) = stagger.full_scale();
  let raw = |value: usize, scale: usize| {
    if value.is_multiple_of(scale) {
      Rational::new((value / scale) as u32, 1)
    } else {
      Rational::new(value as u32, scale as u32)
    }
  };
  Some(DefaultCrop {
    origin: [raw(left, fx), raw(top, fy)],
    size: [raw(width, fx), raw(height, fy)],
  })
}

fn normalize_wb(raw_wb: [f32; 4]) -> [f32; 4] {
  log::debug!("RAF raw wb: {:?}", raw_wb);
  // We never have more then RGB colors so far (no RGBE etc.)
  // So we combine G1 and G2 to get RGB wb.
  let div = raw_wb[1];
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

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn wb_without_green_is_unknown() {
    assert!(normalize_wb([1.0, 0.0, 0.0, 1.0]).iter().all(|v| v.is_nan()));
    assert_eq!(normalize_wb([512.0, 256.0, 256.0, 384.0])[..3], [2.0, 1.0, 1.5]);
  }
}
