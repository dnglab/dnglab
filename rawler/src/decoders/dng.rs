use std::cmp;
use std::f32::NAN;

use crate::alloc_image_ok;
use crate::bits::LookupTable;
use crate::cfa::*;
use crate::decoders::*;
use crate::decompressors::ljpeg::*;
use crate::imgop::xyz::FlatColorMatrix;
use crate::imgop::xyz::Illuminant;
use crate::imgop::Point;
use crate::imgop::Rect;
use crate::packed::*;
use crate::tags::DngTag;
use crate::tags::TiffCommonTag;
use crate::RawImage;

#[derive(Debug, Clone)]
pub struct DngDecoder<'a> {
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
}

impl<'a> DngDecoder<'a> {
  pub fn new(_file: &mut RawFile, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<DngDecoder<'a>> {
    Ok(DngDecoder { tiff, rawloader })
  }
}

/// DNG format encapsulation for analyzer
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DngFormat {
  tiff: GenericTiffReader,
}

impl<'a> Decoder for DngDecoder<'a> {
  fn raw_image(&self, file: &mut RawFile, _params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let raw = self.get_raw_ifd()?;
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let cpp = fetch_tiff_tag!(raw, TiffCommonTag::SamplesPerPixel).force_usize(0);

    let image = match fetch_tiff_tag!(raw, TiffCommonTag::Compression).force_u32(0) {
      1 => self.decode_uncompressed(file, raw, width * cpp, height, dummy)?,
      7 => self.decode_compressed(file, raw, width * cpp, height, cpp, dummy)?,
      c => return Err(RawlerError::General(format!("Don't know how to read DNGs with compression {}", c))),
    };

    let orientation = Orientation::from_tiff(self.tiff.root_ifd());

    let mut cam = self.make_camera(raw, width, height)?;
    // If we know the camera, re-use the clean names
    if let Ok(known_cam) = self.rawloader.check_supported(self.tiff.root_ifd()) {
      cam.clean_make = known_cam.clean_make;
      cam.clean_model = known_cam.clean_model;
    }

    let mut image = RawImage::new(cam, cpp, self.get_wb()?, image, false);
    image.orientation = orientation;

    Ok(image)
  }

  fn format_dump(&self) -> FormatDump {
    FormatDump::Dng(DngFormat { tiff: self.tiff.clone() })
  }

  fn raw_metadata(&self, _file: &mut RawFile, _params: RawDecodeParams) -> Result<RawMetadata> {
    let raw = self.get_raw_ifd()?;
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let mut cam = self.make_camera(raw, width, height)?;
    // If we know the camera, re-use the clean names
    if let Ok(known_cam) = self.rawloader.check_supported(self.tiff.root_ifd()) {
      cam.clean_make = known_cam.clean_make;
      cam.clean_model = known_cam.clean_model;
    }
    let exif = Exif::new(self.tiff.root_ifd())?;
    let mdata = RawMetadata::new(&cam, exif);
    Ok(mdata)
  }
}

impl<'a> DngDecoder<'a> {
  fn get_raw_ifd(&self) -> Result<&IFD> {
    let ifds = self
      .tiff
      .find_ifds_with_tag(TiffCommonTag::Compression)
      .into_iter()
      .filter(|ifd| {
        let compression = (**ifd).get_entry(TiffCommonTag::Compression).unwrap().force_u32(0);
        let subsampled = match (**ifd).get_entry(TiffCommonTag::NewSubFileType) {
          Some(e) => e.force_u32(0) & 1 != 0,
          None => false,
        };
        !subsampled && (compression == 7 || compression == 1 || compression == 0x884c)
      })
      .collect::<Vec<&IFD>>();
    Ok(ifds[0])
  }

  fn make_camera(&self, raw: &IFD, width: usize, height: usize) -> Result<Camera> {
    let make = fetch_tiff_tag!(self.tiff, TiffCommonTag::Make).get_string()?.to_owned();
    let model = fetch_tiff_tag!(self.tiff, TiffCommonTag::Model).get_string()?.to_owned();
    let mode = String::from("dng");

    let blacklevels = self.get_blacklevels(raw)?;
    let whitelevels = self.get_whitelevels(raw)?;

    let active_area = self.get_active_area(raw, width, height);
    let crop_area = if let Some(crops) = self.get_crop(raw) {
      if let Some(active_area) = &active_area {
        let mut full = crops;
        full.p.x += active_area[0]; // left
        full.p.y += active_area[1]; // Top
        Some(full.as_ltrb_offsets(width, height))
      } else {
        Some(crops.as_ltrb_offsets(width, height))
      }
    } else {
      None
    };

    let linear = fetch_tiff_tag!(raw, TiffCommonTag::PhotometricInt).force_usize(0) == 34892;
    let cfa = if linear { CFA::default() } else { self.get_cfa(raw)? };
    let color_matrix = self.get_color_matrix()?;
    let bps = raw.get_entry(TiffCommonTag::BitsPerSample).map(|v| v.force_usize(0)).unwrap_or(16);

    Ok(Camera {
      clean_make: make.clone(),
      clean_model: model.clone(),
      make,
      model,
      mode,
      whitelevels,
      blacklevels,
      blackareah: None,
      blackareav: None,
      xyz_to_cam: Default::default(),
      color_matrix,
      cfa,
      active_area,
      crop_area,
      bps,
      ..Default::default()
    })
  }

  fn get_wb(&self) -> Result<[f32; 4]> {
    if let Some(levels) = self.tiff.get_entry(TiffCommonTag::AsShotNeutral) {
      Ok([1.0 / levels.force_f32(0), 1.0 / levels.force_f32(1), 1.0 / levels.force_f32(2), NAN])
    } else {
      Ok([NAN, NAN, NAN, NAN])
    }
  }

  fn get_blacklevels(&self, raw: &IFD) -> Result<[u16; 4]> {
    if let Some(levels) = raw.get_entry(TiffCommonTag::BlackLevels) {
      if levels.count() < 4 {
        let black = levels.force_f32(0) as u16;
        Ok([black, black, black, black])
      } else {
        Ok([
          levels.force_f32(0) as u16,
          levels.force_f32(1) as u16,
          levels.force_f32(2) as u16,
          levels.force_f32(3) as u16,
        ])
      }
    } else {
      Ok([0, 0, 0, 0])
    }
  }

  fn get_whitelevels(&self, raw: &IFD) -> Result<[u16; 4]> {
    let level = fetch_tiff_tag!(raw, TiffCommonTag::WhiteLevel).force_u32(0) as u16;
    Ok([level, level, level, level])
  }

  fn get_cfa(&self, raw: &IFD) -> Result<CFA> {
    let pattern = fetch_tiff_tag!(raw, TiffCommonTag::CFAPattern);
    Ok(CFA::new_from_tag(pattern))
  }

  fn get_active_area(&self, raw: &IFD, width: usize, height: usize) -> Option<[usize; 4]> {
    if let Some(crops) = raw.get_entry(DngTag::ActiveArea) {
      let rect = [crops.force_usize(0), crops.force_usize(1), crops.force_usize(2), crops.force_usize(3)];
      Some(Rect::new_with_dng(&rect).as_ltrb_offsets(width, height))
    } else {
      // Ignore missing crops, at least some pentax DNGs don't have it
      None
    }
  }

  fn get_crop(&self, raw: &IFD) -> Option<Rect> {
    if let Some(crops) = raw.get_entry(DngTag::DefaultCropOrigin) {
      let p = Point::new(crops.force_usize(0), crops.force_usize(1));
      if let Some(size) = raw.get_entry(DngTag::DefaultCropSize) {
        let s = Point::new(size.force_usize(0), size.force_usize(1));
        return Some(Rect::new_with_points(p, s));
      }
    }
    None
  }

  fn _get_masked_areas(&self, raw: &IFD) -> Vec<Rect> {
    let mut areas = Vec::new();

    if let Some(masked_area) = raw.get_entry(TiffCommonTag::MaskedAreas) {
      for x in (0..masked_area.count() as usize).step_by(4) {
        areas.push(Rect::new_with_points(
          Point::new(masked_area.force_usize(x), masked_area.force_usize(x + 1)),
          Point::new(masked_area.force_usize(x + 2), masked_area.force_usize(x + 3)),
        ));
      }
    }

    areas
  }

  fn get_color_matrix(&self) -> Result<HashMap<Illuminant, FlatColorMatrix>> {
    let mut result = HashMap::new();

    let mut read_matrix = |cal: DngTag, mat: DngTag| -> Result<()> {
      if let Some(c) = self.tiff.get_entry(mat) {
        let illuminant: Illuminant = fetch_tiff_tag!(self.tiff, cal).force_u16(0).try_into()?;
        let mut matrix = FlatColorMatrix::new();
        for i in 0..c.count() as usize {
          matrix.push(c.force_f32(i));
        }
        assert!(matrix.len() <= 12 && !matrix.is_empty());
        result.insert(illuminant, matrix);
      }
      Ok(())
    };

    read_matrix(DngTag::CalibrationIlluminant1, DngTag::ColorMatrix1)?;
    read_matrix(DngTag::CalibrationIlluminant2, DngTag::ColorMatrix2)?;
    // TODO: add 3

    Ok(result)
  }

  pub fn decode_uncompressed(&self, file: &mut RawFile, raw: &IFD, width: usize, height: usize, dummy: bool) -> Result<PixU16> {
    let offset = fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_u64(0);
    let size = fetch_tiff_tag!(raw, TiffCommonTag::StripByteCounts).force_u64(0);
    let src = file.subview(offset, size).unwrap();

    match fetch_tiff_tag!(raw, TiffCommonTag::BitsPerSample).force_u32(0) {
      16 => Ok(decode_16le(&src, width, height, dummy)),
      12 => Ok(decode_12be(&src, width, height, dummy)),
      10 => Ok(decode_10le(&src, width, height, dummy)),
      8 => {
        // It's 8 bit so there will be linearization involved surely!
        let linearization = fetch_tiff_tag!(self.tiff, TiffCommonTag::Linearization);
        let curve = {
          let mut points = vec![0_u16; 256];
          for i in 0..256 {
            points[i] = linearization.force_u32(i) as u16;
          }
          LookupTable::new(&points)
        };
        Ok(decode_8bit_wtable(&src, &curve, width, height, dummy))
      }
      bps => Err(format_args!("DNG: Don't know about {} bps images", bps).into()),
    }
  }

  pub fn decode_compressed(&self, file: &mut RawFile, raw: &IFD, width: usize, height: usize, cpp: usize, dummy: bool) -> Result<PixU16> {
    if let Some(offsets) = raw.get_entry(TiffCommonTag::StripOffsets) {
      // We're in a normal offset situation
      if offsets.count() != 1 {
        return Err("DNG: files with more than one slice not supported yet".into());
      }
      let offset = offsets.force_u64(0);
      let size = fetch_tiff_tag!(raw, TiffCommonTag::StripByteCounts).force_u64(0);
      let src = file.subview(offset, size).unwrap();
      let mut out = alloc_image_ok!(width, height, dummy);
      let decompressor = LjpegDecompressor::new(&src)?;
      decompressor.decode(out.pixels_mut(), 0, width, width, height, dummy)?;
      Ok(out)
    } else if let Some(offsets) = raw.get_entry(TiffCommonTag::TileOffsets) {
      // They've gone with tiling
      let twidth = fetch_tiff_tag!(raw, TiffCommonTag::TileWidth).force_usize(0) * cpp;
      let tlength = fetch_tiff_tag!(raw, TiffCommonTag::TileLength).force_usize(0);
      let coltiles = (width - 1) / twidth + 1;
      let rowtiles = (height - 1) / tlength + 1;
      if coltiles * rowtiles != offsets.count() as usize {
        return Err(format_args!("DNG: trying to decode {} tiles from {} offsets", coltiles * rowtiles, offsets.count()).into());
      }
      let buffer = file.as_vec().unwrap();
      Ok(decode_threaded_multiline(
        width,
        height,
        tlength,
        dummy,
        &(|strip: &mut [u16], row| {
          let row = row / tlength;
          for col in 0..coltiles {
            let offset = offsets.force_usize(row * coltiles + col);
            let src = &buffer[offset..];
            let decompressor = LjpegDecompressor::new(src).unwrap();
            let bwidth = cmp::min(width, (col + 1) * twidth) - col * twidth;
            let blength = cmp::min(height, (row + 1) * tlength) - row * tlength;
            // FIXME: instead of unwrap() we need to propagate the error
            decompressor.decode(strip, col * twidth, width, bwidth, blength, dummy).unwrap();
          }
        }),
      ))
    } else {
      Err(RawlerError::General("DNG: didn't find tiles or strips".to_string()))
    }
  }
}

pub fn decode_tiles(file: &mut RawFile, raw: &IFD, dummy: bool) -> Result<PixU16> {
  let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
  let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
  let cpp = fetch_tiff_tag!(raw, TiffCommonTag::SamplesPerPixel).force_usize(0);

  let offsets = raw.get_entry(TiffCommonTag::TileOffsets).unwrap();

  // They've gone with tiling
  let twidth = fetch_tiff_tag!(raw, TiffCommonTag::TileWidth).force_usize(0) * cpp;
  let tlength = fetch_tiff_tag!(raw, TiffCommonTag::TileLength).force_usize(0);
  let coltiles = (width - 1) / twidth + 1;
  let rowtiles = (height - 1) / tlength + 1;
  if coltiles * rowtiles != offsets.count() as usize {
    return Err(format_args!("DNG: trying to decode {} tiles from {} offsets", coltiles * rowtiles, offsets.count()).into());
  }
  let buffer = file.as_vec().unwrap();

  Ok(decode_threaded_multiline(
    width,
    height,
    tlength,
    dummy,
    &(|strip: &mut [u16], row| {
      let row = row / tlength;
      for col in 0..coltiles {
        let offset = offsets.force_usize(row * coltiles + col);
        let src = &buffer[offset..];
        let decompressor = LjpegDecompressor::new(src).unwrap();
        let bwidth = cmp::min(width, (col + 1) * twidth) - col * twidth;
        let blength = cmp::min(height, (row + 1) * tlength) - row * tlength;
        // FIXME: instead of unwrap() we need to propagate the error
        decompressor.decode(strip, col * twidth, width, bwidth, blength, dummy).unwrap();
      }
    }),
  ))
}
