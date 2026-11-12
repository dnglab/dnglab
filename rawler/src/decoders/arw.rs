use std::cmp;
use std::io::Cursor;

use image::DynamicImage;
use log::debug;
use rayon::prelude::*;

use crate::RawImage;
use crate::RawLoader;
use crate::RawlerError;
use crate::Result;
use crate::alloc_image_ok;
use crate::bits::*;
use crate::decompressors::arw6::decompress_arw6;
use crate::decompressors::decompress_lines_fn;
use crate::decompressors::decompress_strips_fn;
use crate::decompressors::ljpeg::LjpegDecompressor;
use crate::decompressors::packed::decompress_12le;
use crate::decompressors::packed::decompress_14be_unpacked;
use crate::decompressors::packed::decompress_16be;
use crate::decompressors::packed::decompress_16le;
use crate::exif::Exif;
use crate::formats::tiff::Entry;
use crate::formats::tiff::GenericTiffReader;
use crate::formats::tiff::IFD;
use crate::formats::tiff::Value;
use crate::formats::tiff::ifd::OffsetMode;
use crate::formats::tiff::reader::TiffReader;
use crate::imgop::Dim2;
use crate::imgop::Point;
use crate::imgop::Rect;
use crate::imgop::yuv::interpolate_yuv;
use crate::imgop::yuv::ycbcr_to_rgb;
use crate::lens::LensDescription;
use crate::lens::LensResolver;
use crate::pixarray::PixU16;
use crate::pumps::BitPump;
use crate::pumps::BitPumpLSB;
use crate::pumps::BitPumpMSB;
use crate::rawimage::BlackLevel;
use crate::rawimage::CFAConfig;
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
use super::ok_cfa_image;
use super::tag_data;

const SONY_E_MOUNT: &str = "e-mount";
const SONY_A_MOUNT: &str = "a-mount";

#[derive(Debug, Clone)]
pub struct ArwDecoder<'a> {
  #[allow(unused)]
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
  makernote: IFD,
  camera: CameraLookup,
}

impl<'a> ArwDecoder<'a> {
  pub fn new(file: &RawSource, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<ArwDecoder<'a>> {
    let camera = rawloader.lookup_camera_with_mode(tiff.root_ifd(), "")?;
    let makernote = if let Some(exif) = tiff.find_first_ifd_with_tag(ExifTag::MakerNotes) {
      exif.parse_makernote(&mut file.reader(), OffsetMode::Absolute, &[])?
    } else {
      log::warn!("ARW makernote not found");
      None
    }
    .ok_or_else(|| RawlerError::decoder_failed("File has not makernotes"))?;

    //makernote.dump::<ExifTag>(0).iter().for_each(|line| eprintln!("DUMP: {}", line));

    Ok(ArwDecoder {
      tiff,
      rawloader,
      makernote,
      camera,
    })
  }
}

impl<'a> Decoder for ArwDecoder<'a> {
  fn raw_image(&self, file: &RawSource, _params: &RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let camera = self.camera.supported()?;
    let data = self.tiff.find_ifds_with_tag(TiffCommonTag::StripOffsets);
    if data.is_empty() {
      let mut img = if camera.model == "DSLR-A100" {
        self.image_a100(camera, file, dummy)?
      } else {
        // try decoding as SRF
        self.image_srf(camera, file, dummy)?
      };
      img.orientation = Orientation::from_tiff(self.tiff.root_ifd());
      return Ok(img);
    }
    let raw = data[0];
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let mut height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let offset = fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_usize(0);
    let count = fetch_tiff_tag!(raw, TiffCommonTag::StripByteCounts).force_usize(0);
    let compression = fetch_tiff_tag!(raw, TiffCommonTag::Compression).force_u32(0);
    let bps = if let Some(forced_bps) = &camera.bps {
      *forced_bps
    } else {
      fetch_tiff_tag!(raw, TiffCommonTag::BitsPerSample).force_usize(0)
    };
    // The decoders below store up to four 16-bit samples per pixel
    image_bytes(width, height, 64)?;

    let params = self.get_params(file)?;
    debug!("Params: {:?}", params);

    //assert!(params.blacklevel.is_some());
    //assert!(params.whitelevel.is_some()); // DSC-R1 is SR2 format and has no whitelevel

    let mut white = params.whitelevel.map(|x| x[0]);
    let mut black = params.blacklevel;

    let src = file.subview_until_eof(offset as u64)?;
    let mut cpp = 1;
    // ARQ: pixel shift sequence merged by Sony software, four raw samples
    // per pixel (one per position of the 2x2 CFA block).
    let samples_per_pixel = raw.get_entry(TiffCommonTag::SamplesPerPixel).map(|e| e.value.force_usize(0)).unwrap_or(1);
    let arq = compression == 1 && samples_per_pixel == 4;

    let image = match compression {
      1 if arq => {
        cpp = 3;
        decode_arq(src, width, height, &camera.cfa, dummy)?
      }
      1 => {
        if camera.model == "DSC-R1" {
          decompress_14be_unpacked(src, width, height, dummy).map_err(RawlerError::decoder_failed)?
        } else {
          decompress_16le(src, width, height, dummy).map_err(RawlerError::decoder_failed)?
        }
      }
      7 => {
        cpp = fetch_tiff_tag!(raw, TiffCommonTag::SamplesPerPixel).force_usize(0);
        // Starting with A-1, image is compressed in tiles with LJPEG92.
        // Data is RGGB for bayer readout and YCbCr for reduced resolution files.
        ArwDecoder::decode_ljpeg(camera, file, raw, dummy)?
      }
      32766 => {
        // The ARW6 raw IFD stores the black and white levels halved (and its
        // SonyCurve tag is zeroed; the codec has a built-in curve instead).
        let halved_black = self
          .get_blacklevel(raw)
          .ok_or_else(|| RawlerError::decoder_failed("ARW6: no black level in the raw IFD"))?;
        let halved_white = fetch_tiff_tag!(raw, TiffCommonTag::WhiteLevel).force_u16(0);
        let (arw6_black, arw6_white) = arw6_levels(halved_black, halved_white)?;
        black = Some(arw6_black);
        white = Some(arw6_white);
        let strip = file.subview(offset as u64, count as u64)?;
        ArwDecoder::decode_arw6(strip, width, height, dummy)?
      }
      32767 => {
        let bits = width
          .checked_mul(height)
          .and_then(|pixels| pixels.checked_mul(bps))
          .ok_or_else(|| decoder_error!("ARW: image of {}x{} pixels with {} bps overflows", width, height, bps))?;
        if Some(bits) != count.checked_mul(8) {
          height = height.checked_add(8).ok_or_else(|| decoder_error!("ARW1: image height {} overflows", height))?;
          // ARW1 data has 12 bits, the white and black levels of the file have 14 bits
          ArwDecoder::levels_to_12bit(&mut white, &mut black);
          ArwDecoder::decode_arw1(src, width, height, dummy)?
        } else {
          match bps {
            8 => {
              let curve = ArwDecoder::get_curve(raw)?;
              ArwDecoder::decode_arw2(src, width, height, &curve, dummy)?
            }
            12 => {
              /*
                Some cameras like the A700 have an uncompressed mode where the output is 12bit and
                does not require any curve. For these all we need to do is set 12bit black and white
                points instead of the 14bit ones of the normal compressed 8bit -> 10bit -> 14bit mode.

                We set these 12bit points by shifting down the 14bit points. It might make sense to
                have a separate camera mode instead but since the values seem good we don't bother.
              */
              ArwDecoder::levels_to_12bit(&mut white, &mut black);
              decompress_12le(src, width, height, dummy).map_err(RawlerError::decoder_failed)?
            }
            _ => return Err(decoder_error!("ARW2: Don't know how to decode images with {} bps", bps)),
          }
        }
      }
      _ => return Err(decoder_error!("ARW: Don't know how to decode type {}", compression)),
    };

    let blacklevel = black
      .map(|black| match cpp {
        3 if arq => Ok(BlackLevel::new(&arq_levels(&black, &camera.cfa), 1, 1, cpp)),
        1 => Ok(BlackLevel::new(&black, camera.cfa.width, camera.cfa.height, cpp)),
        // For YUV data, the blacklevel needs to be multiplicated by 2
        3 => Ok(BlackLevel::new(&[u32::from(black[0]) * 2; 3], 1, 1, cpp)),
        _ => Err(decoder_error!("ARW: Unsupported cpp: {}", cpp)),
      })
      .transpose()?;
    let whitelevel = white.map(|white| WhiteLevel(vec![white as u32; cpp]));
    if whitelevel.is_none() && camera.whitelevel.is_none() {
      return Err(decoder_error!("ARW: no white level in the file or the camera definition"));
    }

    let photometric = match cpp {
      1 => RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(camera)),
      3 => RawPhotometricInterpretation::LinearRaw,
      _ => return Err(decoder_error!("ARW: Unsupported cpp: {}", cpp)),
    };

    let mut img = RawImage::new(camera.clone(), image, cpp, params.wb, photometric, blacklevel, whitelevel, dummy);
    img.orientation = Orientation::from_tiff(self.tiff.root_ifd());

    if cpp == 3 && !arq {
      // For debayer images, we assume WB coeffs already applied
      img.wb_coeffs = [1.0, 1.0, 1.0, f32::NAN];
    }

    if let Some(raw_image_size) = self.get_raw_image_size(raw)? {
      log::debug!("Found SONYRAWIMAGESIZE tag, using as active_area");
      img.active_area = Some(raw_image_size);
    } else {
      img.active_area = camera.active_area.and_then(|area| Rect::new_with_borders(Dim2::new(width, height), &area));
    }
    img.crop_area = Rect::from_tiff(raw).or_else(|| camera.crop_area.and_then(|area| Rect::new_with_borders(Dim2::new(width, height), &area)));

    log::debug!("raw dim: {}x{}", width, height);
    log::debug!("crop_area: {:?}", img.crop_area);
    log::debug!("active_area: {:?}", img.active_area);
    Ok(img)
  }

  fn check_supported(&self) -> Result<()> {
    self.camera.supported().map(|_| ())
  }

  /// Return the embedded JPEG preview
  /// Exiftool docs says there is a tag 0x2002 including the image, but this tag
  /// exists in none of the samples?! Instead, we can use the JPEG thumbnail
  /// tags which exists for most samples.
  fn preview_image(&self, file: &RawSource, params: &RawDecodeParams) -> Result<Option<DynamicImage>> {
    if params.image_index != 0 {
      return Ok(None);
    }
    let root = self.tiff.root_ifd();
    if let Some(preview_off) = root.get_entry(ExifTag::JPEGInterchangeFormat) {
      if let Some(preview_len) = root.get_entry(ExifTag::JPEGInterchangeFormatLength) {
        let buf = file.subview(preview_off.force_u64(0), preview_len.force_u64(0))?;
        let img = image::load_from_memory_with_format(buf, image::ImageFormat::Jpeg).map_err(|err| decoder_error!("Failed to read JPEG image: {:?}", err))?;
        return Ok(Some(img));
      }
    }
    Ok(None)
  }

  fn format_dump(&self) -> crate::analyze::FormatDump {
    todo!()
  }

  fn raw_metadata(&self, _file: &RawSource, _params: &RawDecodeParams) -> Result<RawMetadata> {
    let mut exif = Exif::new(self.tiff.root_ifd())?;
    exif.extend_from_ifd(self.get_exif()?)?; // TODO: is this required?
    let mdata = RawMetadata::new_with_lens(self.camera.metadata(), exif, self.get_lens_description()?.cloned());
    Ok(mdata)
  }

  fn format_hint(&self) -> FormatHint {
    FormatHint::ARW
  }
}

impl<'a> ArwDecoder<'a> {
  fn get_exif(&self) -> Result<&IFD> {
    self
      .tiff
      .find_first_ifd_with_tag(ExifTag::MakerNotes)
      .ok_or_else(|| RawlerError::decoder_failed("EXIF IFD not found"))
  }

  /// Get lens description by analyzing TIFF tags and makernotes
  fn get_lens_description(&self) -> Result<Option<&'static LensDescription>> {
    // Try tags 0x9416, 0x9050 and 0x940C, skipping tags that are too short
    for (tag, offset) in [(ArwMakernoteTag::Tag_9416, 0x004b), (ArwMakernoteTag::Tag_9050, 263), (ArwMakernoteTag::Tag_940C, 9)] {
      if let Some(Entry {
        value: Value::Undefined(params),
        ..
      }) = self.makernote.get_entry(tag)
        && let Some(lens_id) = deciphered_lens_id(params, offset)
      {
        debug!("Lens Id tag: {}", lens_id);

        let resolver = LensResolver::new()
          .with_camera(self.camera.metadata())
          .with_lens_id((lens_id as u32, 0))
          .with_mounts(&[SONY_E_MOUNT.into(), SONY_A_MOUNT.into()]);
        return Ok(resolver.resolve());
      }
    }
    Ok(None)
  }

  fn image_a100(&self, camera: &Camera, file: &RawSource, dummy: bool) -> Result<RawImage> {
    // We've caught the elusive A100 in the wild, a transitional format
    // between the simple sanity of the MRW custom format and the wordly
    // wonderfullness of the Tiff-based ARW format, let's shoot from the hip
    let data = self.tiff.find_ifds_with_tag(TiffCommonTag::SubIFDs);
    if data.is_empty() {
      return Err(RawlerError::decoder_failed("ARW: Couldn't find the data IFD!".to_string()));
    }
    let raw = data[0];
    let width = 3880;
    let height = 2608;
    let offset = fetch_tiff_tag!(raw, TiffCommonTag::SubIFDs).force_usize(0);

    let src = file.subview_until_eof(offset as u64)?;
    let image = ArwDecoder::decode_arw1(src, width, height, dummy)?;

    // Get the WB the MRW way
    // DNGPrivateTag contains 4 bytes forming a LE u32 offset value.
    let priv_offset = {
      let entry = fetch_tiff_tag!(self.tiff, TiffCommonTag::DNGPrivateArea);
      if entry.value_type() != 0x1 {
        return Err(decoder_error!("ARW: DNGPrivateArea has the type {} instead of BYTE", entry.value_type_name()));
      }
      LEu32(tag_data(entry, 4)?, 0)
    };
    let wb_coeffs = a100_wb(file.subview_until_eof(priv_offset as u64)?)?;

    let cpp = 1;
    ok_cfa_image(camera.clone(), cpp, normalize_wb(wb_coeffs), image, dummy)
  }

  fn image_srf(&self, camera: &Camera, file: &RawSource, dummy: bool) -> Result<RawImage> {
    let data = self.tiff.find_ifds_with_tag(TiffCommonTag::ImageWidth);
    if data.is_empty() {
      return Err(RawlerError::decoder_failed("ARW: Couldn't find the data IFD!".to_string()));
    }
    let raw = data[0];

    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);

    let len = image_bytes(width, height, 16)?;
    let image = if dummy {
      PixU16::new_uninit(width, height)
    } else {
      let buffer = file.buf();

      // Constants taken from dcraw
      let off: usize = 862144;
      let head_off: usize = 164600;

      let first_key = srf_first_key(buffer)?;
      let head = ArwDecoder::sony_decrypt(buffer, head_off, 40, first_key)?;
      let second_key = LEu32(&head, 22);

      // "Decrypt" the whole image buffer
      let image_data = ArwDecoder::sony_decrypt(buffer, off, len, second_key)?;
      decompress_16be(&image_data, width, height, dummy).map_err(RawlerError::decoder_failed)?
    };
    let cpp = 1;
    ok_cfa_image(camera.clone(), cpp, [f32::NAN, f32::NAN, f32::NAN, f32::NAN], image, dummy)
  }

  /// Shift the 14 bit white and black levels of a file down to 12 bits
  fn levels_to_12bit(white: &mut Option<u16>, black: &mut Option<[u16; 4]>) {
    *white = white.map(|x| x >> 2);
    *black = black.map(|levels| levels.map(|x| x >> 2));
  }

  pub(crate) fn decode_arw1(buf: &[u8], width: usize, height: usize, dummy: bool) -> Result<PixU16> {
    image_bytes(width, height, 16)?;
    let mut out = alloc_image_ok!(width, height, dummy);
    let mut pump = BitPumpMSB::new(buf);

    let mut sum: i32 = 0;
    for x in 0..width {
      let col = width - 1 - x;
      let mut row = 0;
      while row <= height {
        if row == height {
          row = 1;
        }

        let mut len: u32 = 4 - pump.get_bits(2)?;
        if len == 3 && pump.get_bits(1)? != 0 {
          len = 0;
        } else if len == 4 {
          let zeros = pump.peek_bits(13)?.leading_zeros() - 19;
          len += zeros;
          pump.get_bits(cmp::min(13, zeros + 1))?;
        }
        let diff: i32 = pump.get_ibits(len)?;
        // Crafted data overflows the sum, only its low 16 bits are used
        sum = sum.wrapping_add(diff);
        if len > 0 && (diff & (1 << (len - 1))) == 0 {
          sum = sum.wrapping_sub((1 << len) - 1);
        }
        out[row * width + col] = sum as u16;
        row += 2
      }
    }
    Ok(out)
  }

  pub(crate) fn decode_arw6(buf: &[u8], width: usize, height: usize, dummy: bool) -> Result<PixU16> {
    decompress_arw6(buf, width, height, dummy)
  }

  pub(crate) fn decode_arw2(buf: &[u8], width: usize, height: usize, curve: &LookupTable, dummy: bool) -> Result<PixU16> {
    Ok(
      decompress_lines_fn(
        width,
        height,
        dummy,
        &(|out: &mut [u16], row| {
          let src = buf.get(row * width..).ok_or_else(|| format!("ARW2: row {} is behind the end of the data", row))?;
          let mut pump = BitPumpLSB::new(src);

          let mut random = pump.peek_bits(16)?;
          for out in out.chunks_exact_mut(32) {
            // Process 32 pixels at a time in interleaved fashion
            for j in 0..2 {
              let max = pump.get_bits(11)?;
              let min = pump.get_bits(11)?;
              // Like dcraw, no shift for a minimum above the maximum
              let delta = max.saturating_sub(min);
              // Calculate the size of the data shift needed by how large the delta is
              // A delta with 11 bits requires a shift of 4, 10 bits of 3, etc
              let delta_shift: u32 = cmp::max(0, (32 - (delta.leading_zeros() as i32)) - 7) as u32;
              let imax = pump.get_bits(4)? as usize;
              let imin = pump.get_bits(4)? as usize;

              for i in 0..16 {
                let val = if i == imax {
                  max
                } else if i == imin {
                  min
                } else {
                  cmp::min(0x7ff, (pump.get_bits(7)? << delta_shift) + min)
                };
                out[j + (i * 2)] = curve.dither((val << 1) as u16, &mut random);
              }
            }
          }
          Ok(())
        }),
      )
      .map_err(RawlerError::decoder_failed)?,
    )
  }

  /// Some newer cameras like Alpha-1 uses LJPEG compression, but in an awkward way.
  /// The image is split into 512x512 tiles with cpp = 1, but the LJPEG stream is
  /// compressed as 256x256 with cpp = 4. So the total of bytes matches, but the dimension
  /// is wrong. Actually, the LJPEG stream is two lines packed into a single line each
  /// decompressed line has the bayer pattern: RGGBRGGBRGGB...
  /// So we need to decompress first, then unpack the bayer pattern from one line
  /// into two lines.
  /// For resolution-reduced files (cpp=3), pixels are encoded in YCbCr color space.
  pub(crate) fn decode_ljpeg(camera: &Camera, file: &RawSource, raw: &IFD, dummy: bool) -> Result<PixU16> {
    let offsets = raw
      .get_entry(TiffCommonTag::TileOffsets)
      .ok_or_else(|| RawlerError::decoder_failed("Unable to find TileOffsets"))?;
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let twidth = fetch_tiff_tag!(raw, TiffCommonTag::TileWidth).force_usize(0);
    let tlength = fetch_tiff_tag!(raw, TiffCommonTag::TileLength).force_usize(0);
    let cpp = fetch_tiff_tag!(raw, TiffCommonTag::SamplesPerPixel).force_usize(0);
    if width == 0 || height == 0 {
      return Err(decoder_error!("ARW LJPEG: invalid image size {}x{}", width, height));
    }
    // The tiles are decoded into buffers of 512x512 pixels
    if twidth != 512 || tlength != 512 {
      return Err(RawlerError::unsupported(camera, format!("ARW LJPEG: tiles of {}x{} pixels", twidth, tlength)));
    }
    let coltiles = width.div_ceil(twidth);
    let rowtiles = height.div_ceil(tlength);

    log::debug!("Sony ARW LJPEG raw: width: {}, height: {}, cpp: {}", width, height, cpp);
    log::debug!("LJPEG tile parameters: width: {}, length: {}, cpp: {}", twidth, tlength, cpp);

    if coltiles.checked_mul(rowtiles) != Some(offsets.count() as usize) {
      return Err(RawlerError::unsupported(
        camera,
        format!("ARW LJPEG: trying to decode {}x{} tiles from {} offsets", coltiles, rowtiles, offsets.count()),
      ));
    }
    let buffer = file.buf();

    if cpp == 3 {
      let mut image = decompress_strips_fn(
        width * cpp,
        height,
        tlength,
        dummy,
        &(|lines: &mut [u16], _strip, row| {
          let row = row / tlength;
          for col in 0..coltiles {
            log::debug!("Decode tile: row({}), col({})", row, col);
            let offset = offsets.force_usize(row * coltiles + col);
            let src = buffer
              .get(offset..)
              .ok_or_else(|| format!("ARW LJPEG: offset {offset} of tile ({row},{col}) is behind EOF"))?;
            let decompressor =
              LjpegDecompressor::new(src).map_err(|err| format!("Creating LJPEG decompressor for ARW LJPEG tile ({row},{col}) failed: {err}"))?;
            let cpp = 3;
            let w = 512;
            let h = 512;
            let mut data = vec![0; h * w * cpp];

            decompressor.decode_sony(&mut data, 0, w * cpp, w * cpp, h, dummy)?;
            interpolate_yuv(decompressor.super_h(), decompressor.super_v(), w * cpp, h, &mut data);
            copy_tile(lines, width * cpp, col * twidth * cpp, &data, w * cpp);
          }
          Ok(())
        }),
      )
      .map_err(RawlerError::decoder_failed)?;
      // Convert YC'bC'r data to RGB.
      ycbcr_to_rgb(&mut image.data);
      Ok(image)
    } else if cpp == 1 {
      decompress_strips_fn(
        width,
        height,
        tlength,
        dummy,
        &(|lines: &mut [u16], _strip, row| {
          let row = row / tlength;
          for col in 0..coltiles {
            let offset = offsets.force_usize(row * coltiles + col);
            let src = buffer
              .get(offset..)
              .ok_or_else(|| format!("ARW LJPEG: offset {offset} of tile ({row},{col}) is behind EOF"))?;
            let decompressor = LjpegDecompressor::new(src)?;
            let cpp = 4;
            let w = 256;
            let h = 256;
            let mut data = vec![0; h * w * cpp];

            decompressor.decode(&mut data, 0, w * cpp, w * cpp, h, dummy)?;
            copy_rggb_tile(lines, width, col * twidth, &data, w * cpp);
          }
          Ok(())
        }),
      )
      .map_err(RawlerError::decoder_failed)
    } else {
      Err(RawlerError::unsupported(
        camera,
        format!("NRW files with LJPEG compression and unsupported cpp: {}", cpp),
      ))
    }
  }

  fn get_params(&self, file: &RawSource) -> Result<ArwImageParams> {
    let priv_offset = { LEu32(tag_data(fetch_tiff_tag!(self.tiff, TiffCommonTag::DNGPrivateArea), 4)?, 0) };
    let priv_tiff = IFD::new(&mut file.reader(), priv_offset, 0, 0, Endian::Little, &[])?;

    //priv_tiff.dump::<ExifTag>(0).iter().for_each(|line| println!("DUMPXX: {}", line));

    let sony_offset = fetch_tiff_tag!(priv_tiff, TiffCommonTag::SonyOffset).force_u32(0);
    let sony_length = fetch_tiff_tag!(priv_tiff, TiffCommonTag::SonyLength).force_usize(0);
    // This tag is of type UNDEFINED and contains a 32 bit value
    let sony_key = { LEu32(tag_data(fetch_tiff_tag!(priv_tiff, TiffCommonTag::SonyKey), 4)?, 0) };
    let buffer = file.buf();
    let decrypted_buf = ArwDecoder::sony_decrypt(buffer, sony_offset as usize, sony_length, sony_key)?;

    // The offsets of the decrypted IFD are relative to the file
    let corr = i32::try_from(sony_offset).map_err(|_| decoder_error!("ARW: SR2 offset {} is out of range", sony_offset))?;
    let decrypted_tiff = IFD::new(&mut Cursor::new(decrypted_buf), 0, 0, -corr, Endian::Little, &[])?;

    let wb = self.get_wb(&decrypted_tiff)?;

    let blacklevel = self.get_blacklevel(&decrypted_tiff);
    let whitelevel = self.get_whitelevel(&decrypted_tiff);

    Ok(ArwImageParams { wb, blacklevel, whitelevel })
  }

  fn get_blacklevel(&self, sr2: &IFD) -> Option<[u16; 4]> {
    if let Some(entry) = sr2.get_entry(SR2SubIFD::BlackLevel2) {
      if entry.count() == 4 {
        return Some([entry.force_u16(0), entry.force_u16(1), entry.force_u16(2), entry.force_u16(3)]);
      } else {
        return Some([entry.force_u16(0), entry.force_u16(0), entry.force_u16(0), entry.force_u16(0)]);
      }
    }
    if let Some(entry) = sr2.get_entry(SR2SubIFD::BlackLevel1) {
      if entry.count() == 4 {
        return Some([entry.force_u16(0), entry.force_u16(1), entry.force_u16(2), entry.force_u16(3)]);
      } else {
        return Some([entry.force_u16(0), entry.force_u16(0), entry.force_u16(0), entry.force_u16(0)]);
      }
    }
    None
  }

  fn get_whitelevel(&self, sr2: &IFD) -> Option<[u16; 4]> {
    if let Some(entry) = sr2.get_entry(SR2SubIFD::WhiteLevel) {
      if entry.count() == 4 {
        return Some([entry.force_u16(0), entry.force_u16(1), entry.force_u16(2), entry.force_u16(3)]);
      } else {
        return Some([entry.force_u16(0), entry.force_u16(0), entry.force_u16(0), entry.force_u16(0)]);
      }
    }
    None
  }

  fn get_wb(&self, sr2: &IFD) -> Result<[f32; 4]> {
    let grbg_levels = sr2.get_entry(SR2SubIFD::SonyGRBG);
    let rggb_levels = sr2.get_entry(SR2SubIFD::SonyRGGB);
    if let Some(levels) = grbg_levels {
      Ok(normalize_wb([
        levels.force_u32(1) as f32,
        levels.force_u32(0) as f32,
        levels.force_u32(3) as f32,
        levels.force_u32(2) as f32,
      ]))
    } else if let Some(levels) = rggb_levels {
      Ok(normalize_wb([
        levels.force_u32(0) as f32,
        levels.force_u32(1) as f32,
        levels.force_u32(2) as f32,
        levels.force_u32(3) as f32,
      ]))
    } else {
      Err(RawlerError::decoder_failed("ARW: Couldn't find GRGB or RGGB levels".to_string()))
    }
  }

  fn get_curve(raw: &IFD) -> Result<LookupTable> {
    let centry = fetch_tiff_tag!(raw, TiffCommonTag::SonyCurve);
    let mut curve: [usize; 6] = [0, 0, 0, 0, 0, 4095];

    for i in 0..4 {
      curve[i + 1] = ((centry.force_u32(i) >> 2) & 0xfff) as usize;
    }

    Ok(Self::calculate_curve(curve))
  }

  pub(crate) fn calculate_curve(curve: [usize; 6]) -> LookupTable {
    let mut out = vec![0_u16; curve[5] + 1];
    for i in 0..5 {
      for j in (curve[i] + 1)..(curve[i + 1] + 1) {
        out[j] = out[j - 1] + (1 << i);
      }
    }

    LookupTable::new(&out)
  }

  pub(crate) fn sony_decrypt(buf: &[u8], offset: usize, length: usize, key: u32) -> crate::Result<Vec<u8>> {
    if offset.checked_add(4 * (length / 4)).is_none_or(|end| buf.len() < end) {
      return Err(RawlerError::decoder_failed("sony_decrypt() failed: buffer to short"));
    }
    let mut pad: [u32; 128] = [0_u32; 128];
    let mut mkey = key;
    // Initialize the decryption pad from the key
    for p in 0..4 {
      mkey = mkey.wrapping_mul(48828125).wrapping_add(1);
      pad[p] = mkey;
    }
    pad[3] = (pad[3] << 1) | ((pad[0] ^ pad[2]) >> 31);
    for p in 4..127 {
      pad[p] = ((pad[p - 4] ^ pad[p - 2]) << 1) | ((pad[p - 3] ^ pad[p - 1]) >> 31);
    }
    for p in 0..127 {
      pad[p] = u32::from_be(pad[p]);
    }

    let mut out = Vec::with_capacity(length + 4);
    //for i in 0..(length / 4 + 1) {
    for i in 0..(length / 4) {
      let p = i + 127;
      pad[p & 127] = pad[(p + 1) & 127] ^ pad[(p + 1 + 64) & 127];
      let output = LEu32(buf, offset + i * 4) ^ pad[p & 127];
      out.push(((output >> 0) & 0xff) as u8);
      out.push(((output >> 8) & 0xff) as u8);
      out.push(((output >> 16) & 0xff) as u8);
      out.push(((output >> 24) & 0xff) as u8);
    }
    Ok(out)
  }

  fn get_raw_image_size(&self, raw_ifd: &IFD) -> Result<Option<Rect>> {
    if let Some(entry) = raw_ifd.get_entry(ExifTag::SonyRawImageSize) {
      Ok(Some(Rect::new(Point::default(), Dim2::new(entry.force_usize(0), entry.force_usize(1)))))
    } else {
      Ok(None)
    }
  }
}

/// Copy the lines of a decoded tile of `tile_width` samples per line into
/// the lines of `strip`, which have `strip_width` samples, starting at sample
/// `x`. The parts of the tile outside of the strip are dropped.
fn copy_tile(strip: &mut [u16], strip_width: usize, x: usize, tile: &[u16], tile_width: usize) {
  for (dst, src) in strip.chunks_exact_mut(strip_width).zip(tile.chunks_exact(tile_width)) {
    if let Some(dst) = dst.get_mut(x..) {
      let len = dst.len().min(src.len());
      dst[..len].copy_from_slice(&src[..len]);
    }
  }
}

/// Copy a decoded tile of RGGB blocks, in lines of `tile_width` samples,
/// into the lines of `strip`, which have `width` pixels, starting at pixel
/// `x`. Each line of the tile becomes two lines of the strip, the first one
/// is RGRG… and the second one GBGB…. The parts of the tile outside of the
/// strip are dropped.
fn copy_rggb_tile(strip: &mut [u16], width: usize, x: usize, tile: &[u16], tile_width: usize) {
  for (lines, src) in strip.chunks_exact_mut(2 * width).zip(tile.chunks_exact(tile_width)) {
    let (upper, lower) = lines.split_at_mut(width);
    let (Some(upper), Some(lower)) = (upper.get_mut(x..), lower.get_mut(x..)) else {
      continue;
    };
    for ((upper, lower), block) in upper.chunks_mut(2).zip(lower.chunks_mut(2)).zip(src.as_chunks::<4>().0) {
      upper.iter_mut().zip(&block[..2]).for_each(|(dst, src)| *dst = *src);
      lower.iter_mut().zip(&block[2..]).for_each(|(dst, src)| *dst = *src);
    }
  }
}

/// Key of the header of SRF files, replicating the dcraw contortions to get
/// the "decryption" key
fn srf_first_key(buffer: &[u8]) -> Result<u32> {
  // Constant taken from dcraw
  const KEY_OFF: usize = 200896;
  buffer
    .get(KEY_OFF)
    .map(|&offset| KEY_OFF + offset as usize * 4)
    .and_then(|pos| buffer.get(pos..pos + 4))
    .map(|key| BEu32(key, 0))
    .ok_or_else(|| decoder_error!("SRF: file of {} bytes is too short for the key", buffer.len()))
}

/// White balance of the A100 from the MRW blocks of its DNGPrivateData
fn a100_wb(buf: &[u8]) -> Result<[f32; 4]> {
  match buf.get(0..4).map(|magic| BEu32(magic, 0)) {
    // MRI
    Some(0x4D5249) => {}
    Some(magic) => return Err(decoder_error!("Invalid DNGPRIVATEDATA tag: 0x{:X}, expected 0x4D5249 ", magic)),
    None => return Err(decoder_error!("DNGPRIVATEDATA has {} bytes, the tag needs 4", buf.len())),
  }
  let mut currpos: usize = 8;
  let mut wb_coeffs: [f32; 4] = [1.0, 1.0, 1.0, f32::NAN];
  // At most we read 20 bytes from currpos so check we don't step outside that
  while currpos.saturating_add(20) < buf.len() {
    let tag: u32 = BEu32(buf, currpos);
    let len: usize = LEu32(buf, currpos + 4) as usize;
    if tag == 0x574247 {
      // WBG
      wb_coeffs[0] = LEu16(buf, currpos + 12) as f32;
      wb_coeffs[1] = LEu16(buf, currpos + 14) as f32;
      wb_coeffs[2] = LEu16(buf, currpos + 16) as f32;
      wb_coeffs[3] = LEu16(buf, currpos + 18) as f32;
      break;
    }
    currpos = currpos.saturating_add(len).saturating_add(8);
  }
  Ok(wb_coeffs)
}

/// Black and white levels of ARW6 data from the halved levels of its raw IFD
fn arw6_levels(black: [u16; 4], white: u16) -> Result<([u16; 4], u16)> {
  let double = |level: u16| level.checked_mul(2).ok_or_else(|| decoder_error!("ARW6: level {} exceeds 16 bits when doubled", level));
  Ok(([double(black[0])?, double(black[1])?, double(black[2])?, double(black[3])?], double(white)?))
}

/// Bytes of `width` × `height` pixels of `bits` bits, an error for an image
/// without pixels or a size that overflows
fn image_bytes(width: usize, height: usize, bits: usize) -> Result<usize> {
  width
    .checked_mul(height)
    .and_then(|pixels| pixels.checked_mul(bits))
    .filter(|_| width > 0 && height > 0)
    .map(|bits| bits.div_ceil(8))
    .ok_or_else(|| decoder_error!("ARW: invalid image size {}x{}", width, height))
}

fn normalize_wb(raw_wb: [f32; 4]) -> [f32; 4] {
  debug!("ARW raw wb: {:?}", raw_wb);
  // We never have more then RGB colors so far (no RGBE etc.)
  // So we combine G1 and G2 to get RGB wb.
  let div = raw_wb[1]; // G1 should be 1024 and we use this as divisor
  let mut norm = raw_wb;
  norm.iter_mut().for_each(|v| {
    if v.is_normal() {
      *v /= div
    }
  });
  [norm[0], (norm[1] + norm[2]) / 2.0, norm[3], f32::NAN]
}

crate::tags::tiff_tag_enum!(ArwMakernoteTag);

/// Specific Makernotes tags.
/// These are only related to the Makernote IFD.
#[derive(Debug, Copy, Clone, PartialEq, enumn::N)]
#[repr(u16)]
#[allow(non_camel_case_types)]
pub enum ArwMakernoteTag {
  CameraInfo = 0x0010,
  Tag_940C = 0x940C,
  Tag_9050 = 0x9050,
  Tag_9405 = 0x9405,
  Tag_9416 = 0x9416, // replaces 0x9405 for the Sony ILCE-7SM3, from July 2020
}

/// Decode uncompressed ARQ data (four 16-bit samples per pixel, channel =
/// position in the 2x2 CFA block) into RGB, averaging the two green samples.
fn decode_arq(src: &[u8], width: usize, height: usize, cfa: &crate::cfa::CFA, dummy: bool) -> Result<PixU16> {
  let need = image_bytes(width, height, 64)?;
  if src.len() < need {
    return Err(decoder_error!("ARQ: buffer too short ({} < {})", src.len(), need));
  }
  // Colour (0 = R, 1 = G, 2 = B) of each of the four channels.
  let colors: Vec<usize> = (0..4).map(|ch| cfa.color_at(ch / 2, ch % 2)).collect();
  if colors.iter().filter(|c| **c == 1).count() != 2 || !colors.contains(&0) || !colors.contains(&2) {
    return Err(decoder_error!("ARQ: unsupported CFA pattern {}", cfa.name));
  }
  if dummy {
    return Ok(PixU16::new_uninit(width * 3, height));
  }
  let mut out = vec![0u16; width * height * 3];
  out.par_chunks_exact_mut(width * 3).enumerate().for_each(|(row, line)| {
    let inb = &src[row * width * 8..(row + 1) * width * 8];
    for (dst, s) in line.as_chunks_mut::<3>().0.iter_mut().zip(inb.as_chunks::<8>().0) {
      let mut g = 0u32;
      for (ch, &color) in colors.iter().enumerate() {
        let v = u16::from_le_bytes([s[2 * ch], s[2 * ch + 1]]);
        match color {
          1 => g += v as u32,
          c => dst[c] = v,
        }
      }
      dst[1] = g.div_ceil(2) as u16;
    }
  });
  Ok(PixU16::new_with(out, width * 3, height))
}

/// Per-colour (R, G, B) levels from the four CFA-position levels.
fn arq_levels(levels: &[u16; 4], cfa: &crate::cfa::CFA) -> [u16; 3] {
  let mut sum = [0u32; 3];
  let mut n = [0u32; 3];
  for (pos, &lvl) in levels.iter().enumerate() {
    let c = cfa.color_at(pos / 2, pos % 2).min(2);
    sum[c] += lvl as u32;
    n[c] += 1;
  }
  [0, 1, 2].map(|c| (sum[c] / n[c].max(1)) as u16)
}

/// Decipher/encipher Sony tag 0x2010, 0x900b, 0x9050 and 0x940x data
/// Extracted from exiftool, comment from PH:
/// This is a simple substitution cipher, so use a hardcoded translation table for speed.
/// The formula is: $c = ($b*$b*$b) % 249, where $c is the enciphered data byte
/// note that bytes with values 249-255 are not translated, and 0-1, 82-84,
/// 165-167 and 248 have the same enciphered value)
const fn sony_tag9cxx_decipher_table() -> [u8; 256] {
  let mut tbl = [0; 256];

  let mut i = 0;
  loop {
    if i >= 249 {
      tbl[i] = i as u8;
    } else {
      tbl[i * i * i % 249] = i as u8;
    }
    i += 1;
    if i >= tbl.len() {
      break;
    }
  }
  tbl
}

const SONY_TAG_940X_DECIPHER_TABLE: [u8; 256] = sony_tag9cxx_decipher_table();

fn sony_tag9cxx_decipher(data: &[u8]) -> Vec<u8> {
  let mut buf = Vec::from(data);
  buf.iter_mut().for_each(|v| *v = SONY_TAG_940X_DECIPHER_TABLE[*v as usize]);
  buf
}

/// Lens ID at `offset` of the enciphered Sony tag `data`, `None` if the tag
/// is too short
fn deciphered_lens_id(data: &[u8], offset: usize) -> Option<u16> {
  let id = sony_tag9cxx_decipher(data.get(offset..offset + 2)?);
  Some(LEu16(&id, 0))
}

#[derive(Debug)]
struct ArwImageParams {
  wb: [f32; 4],
  blacklevel: Option<[u16; 4]>,
  whitelevel: Option<[u16; 4]>,
}

crate::tags::tiff_tag_enum!(SR2SubIFD);

/// Specific Sony SR2 sub-IFD tags.
/// These are only related to the Makernote IFD.
#[derive(Debug, Copy, Clone, PartialEq, enumn::N)]
#[repr(u16)]
#[allow(non_camel_case_types)]
pub enum SR2SubIFD {
  SonyGRBG = 0x7303,
  SonyRGGB = 0x7313,
  BlackLevel1 = 0x7300,
  BlackLevel2 = 0x7310,
  WhiteLevel = 0x787f,
}

#[cfg(test)]
mod arq_tests {
  use super::*;

  #[test]
  fn arq_samples_become_rgb_with_averaged_green() {
    let cfa = crate::cfa::CFA::new("RGGB");
    // Two pixels; channels are the CFA positions R, G, G, B.
    let samples: [u16; 8] = [100, 200, 203, 300, 1, 10, 11, 65535];
    let src: Vec<u8> = samples.iter().flat_map(|v| v.to_le_bytes()).collect();
    let img = decode_arq(&src, 2, 1, &cfa, false).expect("decode");
    assert_eq!(img.pixels(), &[100, 202, 300, 1, 11, 65535]);
    assert_eq!(arq_levels(&[512, 510, 514, 600], &cfa), [512, 512, 600]);
  }

  #[test]
  fn arq_rejects_short_buffer() {
    let cfa = crate::cfa::CFA::new("RGGB");
    assert!(decode_arq(&[0u8; 10], 2, 1, &cfa, false).is_err());
  }

  #[test]
  fn arq_rejects_invalid_sizes() {
    let cfa = crate::cfa::CFA::new("RGGB");
    assert!(decode_arq(&[], 0, 1, &cfa, false).is_err());
    assert!(decode_arq(&[], 1, 0, &cfa, false).is_err());
    assert!(decode_arq(&[], usize::MAX, 2, &cfa, false).is_err());
  }

  #[test]
  fn arw1_sum_wraps_around() -> Result<()> {
    // Each pixel of zeros is a difference of 17 bits that subtracts
    // 2^17 - 1 from the sum, which overflows i32 after 16384 pixels.
    ArwDecoder::decode_arw1(&vec![0; 70000], 128, 130, false)?;
    Ok(())
  }

  #[test]
  fn arw1_rejects_an_empty_image() {
    assert!(ArwDecoder::decode_arw1(&[0; 16], 4, 0, false).is_err());
  }

  #[test]
  fn arw2_minimum_above_maximum() -> Result<()> {
    let curve = ArwDecoder::calculate_curve([0, 1024, 2048, 3072, 3584, 4095]);
    // A block of 16 pixels with a maximum of 0 and a minimum of 0x7ff, the
    // other pixels are 0x7ff
    let mut buf = vec![0; 64];
    buf[1] = 0xf8;
    buf[2] = 0x3f;
    let image = ArwDecoder::decode_arw2(&buf, 32, 1, &curve, false)?;
    assert!(image.pixels()[2] > image.pixels()[0]);
    Ok(())
  }

  #[test]
  fn arw2_rejects_short_data() {
    let curve = ArwDecoder::calculate_curve([0, 1024, 2048, 3072, 3584, 4095]);
    assert!(ArwDecoder::decode_arw2(&[0; 10], 32, 4, &curve, false).is_err());
  }

  #[test]
  fn tiles_are_clipped_to_the_strip() {
    // Tile of 3 lines with 4 samples, at sample 3 of a strip of 2 lines
    // with 5 samples
    let tile: Vec<u16> = (1..=12).collect();
    let mut strip = [0; 10];
    copy_tile(&mut strip, 5, 3, &tile, 4);
    assert_eq!(strip, [0, 0, 0, 1, 2, 0, 0, 0, 5, 6]);
    // A full tile
    let mut strip = [0; 8];
    copy_tile(&mut strip, 4, 0, &tile, 4);
    assert_eq!(strip, [1, 2, 3, 4, 5, 6, 7, 8]);
    // A tile beyond the strip
    let mut strip = [0; 8];
    copy_tile(&mut strip, 4, 6, &tile, 4);
    assert_eq!(strip, [0; 8]);
  }

  #[test]
  fn rggb_tiles_are_clipped_to_the_strip() {
    // Tile of 2 lines with 2 RGGB blocks each
    let tile: Vec<u16> = (1..=16).collect();
    // A full tile in a strip of 4 lines with 6 pixels, at pixel 2
    let mut strip = [0; 24];
    copy_rggb_tile(&mut strip, 6, 2, &tile, 8);
    #[rustfmt::skip]
    assert_eq!(strip, [
      0, 0, 1, 2, 5, 6,
      0, 0, 3, 4, 7, 8,
      0, 0, 9, 10, 13, 14,
      0, 0, 11, 12, 15, 16,
    ]);
    // At pixel 3 of a strip of 3 lines with 4 pixels: one column, one line pair
    let mut strip = [0; 12];
    copy_rggb_tile(&mut strip, 4, 3, &tile, 8);
    assert_eq!(strip, [0, 0, 0, 1, 0, 0, 0, 3, 0, 0, 0, 0]);
  }

  #[test]
  fn arw6_levels_are_doubled() {
    assert_eq!(arw6_levels([256, 257, 258, 259], 8191).ok(), Some(([512, 514, 516, 518], 16382)));
    assert!(arw6_levels([32768, 0, 0, 0], 8191).is_err());
    assert!(arw6_levels([0; 4], 40000).is_err());
  }

  #[test]
  fn a100_wb_of_short_and_long_blocks() -> Result<()> {
    assert!(a100_wb(&[]).is_err());
    assert!(a100_wb(b"\0MR").is_err());
    assert!(a100_wb(b"\0MRX").is_err());
    let default = [1.0, 1.0, 1.0];
    assert_eq!(a100_wb(b"\0MRI")?[..3], default);
    // A block that is longer than the data, followed by a WBG block
    let mut buf = b"\0MRI\0\0\0\0\0PRD".to_vec();
    buf.extend_from_slice(&u32::MAX.to_le_bytes());
    buf.extend_from_slice(&[0; 16]);
    assert_eq!(a100_wb(&buf)?[..3], default);
    // A WBG block
    let mut buf = b"\0MRI\0\0\0\0\0WBG".to_vec();
    buf.extend_from_slice(&12_u32.to_le_bytes());
    buf.extend_from_slice(&[0; 4]);
    for coeff in [512_u16, 256, 257, 384] {
      buf.extend_from_slice(&coeff.to_le_bytes());
    }
    buf.push(0);
    assert_eq!(a100_wb(&buf)?, [512.0, 256.0, 257.0, 384.0]);
    Ok(())
  }

  #[test]
  fn srf_key_and_decryption_of_short_data() {
    assert!(srf_first_key(&[0; 1000]).is_err());
    // The key is at 200896 + 4 * the byte at 200896
    let mut buffer = vec![0; 200896 + 8];
    buffer[200896] = 2;
    assert!(srf_first_key(&buffer).is_err());
    buffer.extend_from_slice(&[0x12, 0x34, 0x56, 0x78]);
    assert_eq!(srf_first_key(&buffer).ok(), Some(0x12345678));
    assert!(ArwDecoder::sony_decrypt(&[0; 16], usize::MAX - 2, 8, 0).is_err());
    assert!(ArwDecoder::sony_decrypt(&[0; 16], 8, 12, 0).is_err());
    assert_eq!(ArwDecoder::sony_decrypt(&[0; 16], 8, 8, 0).map(|out| out.len()).ok(), Some(8));
  }

  #[test]
  fn lens_id_of_short_and_long_tags() {
    let data = [0, 2, 8, 27];
    // The cipher maps x^3 % 249 to x
    assert_eq!(deciphered_lens_id(&data, 2), Some(0x0302));
    assert_eq!(deciphered_lens_id(&data, 3), None);
    assert_eq!(deciphered_lens_id(&data, 4), None);
    assert_eq!(deciphered_lens_id(&[], 0x4b), None);
  }

  #[test]
  fn image_bytes_of_valid_and_invalid_sizes() {
    assert_eq!(image_bytes(3, 2, 12).ok(), Some(9));
    assert_eq!(image_bytes(3, 1, 12).ok(), Some(5));
    assert!(image_bytes(0, 2, 16).is_err());
    assert!(image_bytes(2, 0, 16).is_err());
    assert!(image_bytes(u32::MAX as usize, u32::MAX as usize, 64).is_err());
  }
}
