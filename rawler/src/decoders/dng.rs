use std::cmp;
use std::f32::NAN;

use crate::alloc_image_ok;
use crate::alloc_image_plain;
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
      c => return Err(RawlerError::DecoderFailed(format!("Don't know how to read DNGs with compression {}", c))),
    };

    let orientation = Orientation::from_tiff(self.tiff.root_ifd());

    let mut cam = self.make_camera(raw, width, height)?;
    // If we know the camera, re-use the clean names
    if let Ok(known_cam) = self.rawloader.check_supported(self.tiff.root_ifd()) {
      cam.clean_make = known_cam.clean_make;
      cam.clean_model = known_cam.clean_model;
    }

    let blacklevel = self.get_blacklevels(raw)?;
    let whitelevel = self.get_whitelevels(raw)?;

    let mut image = RawImage::new(cam, image, cpp, self.get_wb()?, blacklevel, whitelevel, dummy);
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
    let mode = String::from("dng");

    let make = self
      .tiff
      .root_ifd()
      .get_entry(TiffCommonTag::Make)
      .and_then(|x| x.as_string())
      .cloned()
      .unwrap_or_default();
    let model = self
      .tiff
      .root_ifd()
      .get_entry(TiffCommonTag::Model)
      .and_then(|x| x.as_string())
      .cloned()
      .unwrap_or_default();

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
    let real_bps = raw.get_entry(TiffCommonTag::BitsPerSample).map(|v| v.force_usize(0)).unwrap_or(16);

    Ok(Camera {
      clean_make: make.clone(),
      clean_model: model.clone(),
      make,
      model,
      mode,
      whitelevel: None,
      blacklevel: None,
      blackareah: None,
      blackareav: None,
      xyz_to_cam: Default::default(),
      color_matrix,
      cfa,
      active_area,
      crop_area,
      real_bps,
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

  fn get_blacklevels(&self, raw: &IFD) -> Result<Option<BlackLevel>> {
    // TODO: sanity checks, count checks etc.
    if let Some(levels) = raw.get_entry(TiffCommonTag::BlackLevels) {
      let data: Vec<u16> = (0..levels.count()).map(|i| levels.force_u32(i as usize) as u16).collect();
      let repeat = raw
        .get_entry(DngTag::BlackLevelRepeatDim)
        .map(|entry| (entry.force_usize(0), entry.force_usize(1)))
        .unwrap_or((1, 1));
      let cpp = raw.get_entry(TiffCommonTag::SamplesPerPixel).map(|entry| entry.force_usize(0)).unwrap_or(1);
      return Ok(Some(BlackLevel::new(&data, repeat.1, repeat.0, cpp)));
    }
    Ok(None)
  }

  fn get_whitelevels(&self, raw: &IFD) -> Result<Option<WhiteLevel>> {
    if let Some(levels) = raw.get_entry(TiffCommonTag::WhiteLevel) {
      return Ok(Some((0..levels.count()).map(|i| levels.force_u32(i as usize) as u16).collect()));
    }
    Ok(None)
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
      decode_threaded_multiline(
        width,
        height,
        tlength,
        dummy,
        &(|strip: &mut [u16], row| {
          let row = row / tlength;
          for col in 0..coltiles {
            let offset = offsets.force_usize(row * coltiles + col);
            let src = &buffer[offset..];
            let decompressor = LjpegDecompressor::new(src)?;
            // If SOF width & height matches tile dimension, we can decode directly to strip
            if decompressor.width() == twidth && decompressor.height() == tlength {
              // Calculate output width & length for current tile (right or bottom tile may have smaller dimension)
              let owidth = cmp::min(width, (col + 1) * twidth) - col * twidth;
              let olength = cmp::min(height, (row + 1) * tlength) - row * tlength;
              decompressor.decode(strip, col * twidth, width, owidth, olength, dummy)?;
            }
            // If SOF has other dimension but still same pixel count, we can decode
            // by using a temporary tile buffer. This is the case if RGGB data is encoded
            // by two input lines into one output line.
            // Encoded data is aligned like this:
            //   RGRGRGRGRGRGRGRGRGGBGBGBGBGBGBGBGBGB
            //   RGRGRGRGRGRGRGRGRGGBGBGBGBGBGBGBGBGB
            else if decompressor.width() * decompressor.height() == twidth * tlength {
              // cps is already included in all values, so just compare them.
              let mut tile = alloc_image_plain!(decompressor.width(), decompressor.height(), dummy);
              decompressor.decode(tile.pixels_mut(), 0, width, decompressor.width(), decompressor.height(), dummy)?;
              // Copy lines from temporary tile to strip buffer
              for (i, pix) in tile.pixels().chunks_exact(twidth).enumerate() {
                let start = (i * width) + (col * twidth);
                strip[start..start + twidth].copy_from_slice(pix);
              }
            } else {
              return Err(format!(
                "ljpeg92 decoding failed: tile dimensions {}x{} not matching SOF dimensions {}x{}, dng cps: {}, sof cps: {}",
                twidth,
                tlength,
                decompressor.width(),
                decompressor.height(),
                cpp,
                decompressor.components(),
              ));
            }
          }
          Ok(())
        }),
      )
      .map_err(RawlerError::DecoderFailed)
    } else {
      Err(RawlerError::DecoderFailed("DNG: didn't find tiles or strips".to_string()))
    }
  }
}
