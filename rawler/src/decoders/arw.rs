use std::cmp;
use std::io::Cursor;

use image::DynamicImage;
use log::debug;

use crate::RawImage;
use crate::RawLoader;
use crate::RawlerError;
use crate::Result;
use crate::alloc_image;
use crate::bits::*;
use crate::decoders::decode_threaded;
use crate::decoders::decode_threaded_multiline;
use crate::decompressors::ljpeg::LjpegDecompressor;
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
use crate::packed::decode_12le;
use crate::packed::decode_14be_unpacked;
use crate::packed::decode_16be;
use crate::packed::decode_16le;
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
use super::Decoder;
use super::FormatHint;
use super::RawDecodeParams;
use super::RawMetadata;
use super::ok_cfa_image;

const SONY_E_MOUNT: &str = "e-mount";
const SONY_A_MOUNT: &str = "a-mount";

#[derive(Debug, Clone)]
pub struct ArwDecoder<'a> {
  #[allow(unused)]
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
  makernote: IFD,
  camera: Camera,
}

impl<'a> ArwDecoder<'a> {
  pub fn new(file: &RawSource, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<ArwDecoder<'a>> {
    let camera = rawloader.check_supported(tiff.root_ifd())?;

    let makernote = if let Some(exif) = tiff.find_first_ifd_with_tag(ExifTag::MakerNotes) {
      exif.parse_makernote(&mut file.reader(), OffsetMode::Absolute, &[])?
    } else {
      log::warn!("ARW makernote not found");
      None
    }
    .ok_or("File has not makernotes")?;

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
    let data = self.tiff.find_ifds_with_tag(TiffCommonTag::StripOffsets);
    if data.is_empty() {
      if self.camera.model == "DSLR-A100" {
        return self.image_a100(file, dummy);
      } else {
        // try decoding as SRF
        return self.image_srf(file, dummy);
      }
    }
    let raw = data[0];
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let mut height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let offset = fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_usize(0);
    let count = fetch_tiff_tag!(raw, TiffCommonTag::StripByteCounts).force_usize(0);
    let compression = fetch_tiff_tag!(raw, TiffCommonTag::Compression).force_u32(0);
    let bps = if let Some(forced_bps) = &self.camera.bps {
      *forced_bps
    } else {
      fetch_tiff_tag!(raw, TiffCommonTag::BitsPerSample).force_usize(0)
    };

    let params = self.get_params(file)?;
    debug!("Params: {:?}", params);

    //assert!(params.blacklevel.is_some());
    //assert!(params.whitelevel.is_some()); // DSC-R1 is SR2 format and has no whitelevel

    let mut white = params.whitelevel.map(|x| x[0]);
    let mut black = params.blacklevel;

    let src = file.subview_until_eof(offset as u64)?;
    let mut cpp = 1;

    let image = match compression {
      1 => {
        if self.camera.model == "DSC-R1" {
          decode_14be_unpacked(src, width, height, dummy)
        } else {
          decode_16le(src, width, height, dummy)
        }
      }
      7 => {
        cpp = fetch_tiff_tag!(raw, TiffCommonTag::SamplesPerPixel).force_usize(0);
        // Starting with A-1, image is compressed in tiles with LJPEG92.
        // Data is RGGB for bayer readout and YCbCr for reduced resolution files.
        ArwDecoder::decode_ljpeg(&self.camera, file, raw, dummy)?
      }
      32767 => {
        if (width * height * bps) != count * 8 {
          height += 8;
          ArwDecoder::decode_arw1(src, width, height, dummy)
        } else {
          match bps {
            8 => {
              let curve = ArwDecoder::get_curve(raw)?;
              ArwDecoder::decode_arw2(src, width, height, &curve, dummy)
            }
            12 => {
              /*
                Some cameras like the A700 have an uncompressed mode where the output is 12bit and
                does not require any curve. For these all we need to do is set 12bit black and white
                points instead of the 14bit ones of the normal compressed 8bit -> 10bit -> 14bit mode.

                We set these 12bit points by shifting down the 14bit points. It might make sense to
                have a separate camera mode instead but since the values seem good we don't bother.
              */
              white = white.map(|x| x >> 2);
              black = black.map(|mut x| {
                x.iter_mut().for_each(|x| *x >>= 2);
                x
              });
              decode_12le(src, width, height, dummy)
            }
            _ => return Err(RawlerError::DecoderFailed(format!("ARW2: Don't know how to decode images with {} bps", bps))),
          }
        }
      }
      _ => return Err(RawlerError::DecoderFailed(format!("ARW: Don't know how to decode type {}", compression))),
    };

    let blacklevel = black.map(|black| match cpp {
      1 => BlackLevel::new(&black, self.camera.cfa.width, self.camera.cfa.height, cpp),
      // For YUV data, the blacklevel needs to be multiplicated by 2
      3 => BlackLevel::new(&[black[0] * 2, black[0] * 2, black[0] * 2], 1, 1, cpp),
      _ => panic!("Unsupported cpp == {}", cpp),
    });
    let whitelevel = white.map(|white| WhiteLevel(vec![white as u32; cpp]));

    let photometric = match cpp {
      1 => RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&self.camera)),
      3 => RawPhotometricInterpretation::LinearRaw,
      _ => todo!(),
    };

    let mut img = RawImage::new(self.camera.clone(), image, cpp, params.wb, photometric, blacklevel, whitelevel, dummy);

    if cpp == 3 {
      // For debayer images, we assume WB coeffs already applied
      img.wb_coeffs = [1.0, 1.0, 1.0, f32::NAN];
    }

    if let Some(raw_image_size) = self.get_raw_image_size(raw)? {
      log::debug!("Found SONYRAWIMAGESIZE tag, using as active_area");
      img.active_area = Some(raw_image_size);
    } else {
      img.active_area = self.camera.active_area.map(|area| Rect::new_with_borders(Dim2::new(width, height), &area));
    }
    img.crop_area = Rect::from_tiff(raw).or_else(|| self.camera.crop_area.map(|area| Rect::new_with_borders(Dim2::new(width, height), &area)));

    log::debug!("raw dim: {}x{}", width, height);
    log::debug!("crop_area: {:?}", img.crop_area);
    log::debug!("active_area: {:?}", img.active_area);
    Ok(img)
  }

  /// Return the embedded JPEG preview
  /// Exiftool docs says there is a tag 0x2002 including the image, but this tag
  /// exists in none of the samples?! Instead, we can use the JPEG thumbnail
  /// tags which exists for most samples.
  fn full_image(&self, file: &RawSource, params: &RawDecodeParams) -> Result<Option<DynamicImage>> {
    if params.image_index != 0 {
      return Ok(None);
    }
    let root = self.tiff.root_ifd();
    if let Some(preview_off) = root.get_entry(ExifTag::JPEGInterchangeFormat) {
      if let Some(preview_len) = root.get_entry(ExifTag::JPEGInterchangeFormatLength) {
        let buf = file.subview(preview_off.force_u64(0), preview_len.force_u64(0))?;
        let img = image::load_from_memory_with_format(buf, image::ImageFormat::Jpeg)
          .map_err(|err| RawlerError::DecoderFailed(format!("Failed to read JPEG image: {:?}", err)))?;
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
    let mdata = RawMetadata::new_with_lens(&self.camera, exif, self.get_lens_description()?.cloned());
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
      .ok_or_else(|| "EXIF IFD not found".into())
  }

  /// Get lens description by analyzing TIFF tags and makernotes
  fn get_lens_description(&self) -> Result<Option<&'static LensDescription>> {
    // Try tag 0x9416
    if let Some(Entry {
      value: Value::Undefined(params),
      ..
    }) = self.makernote.get_entry(ArwMakernoteTag::Tag_9416)
    {
      let dechiphered_9416 = sony_tag9cxx_decipher(params);
      let lens_id = LEu16(&dechiphered_9416, 0x004b);
      debug!("Lens Id tag: {}", lens_id);

      let resolver = LensResolver::new()
        .with_camera(&self.camera)
        .with_lens_id((lens_id as u32, 0))
        .with_mounts(&[SONY_E_MOUNT.into(), SONY_A_MOUNT.into()]);
      return Ok(resolver.resolve());
    }

    // Try tag 0x9050
    if let Some(Entry {
      value: Value::Undefined(params),
      ..
    }) = self.makernote.get_entry(ArwMakernoteTag::Tag_9050)
    {
      if params.len() >= 263 + 2 {
        let dechiphered_9050 = sony_tag9cxx_decipher(params);
        let lens_id = LEu16(&dechiphered_9050, 263);
        debug!("Lens Id tag: {}", lens_id);

        let resolver = LensResolver::new()
          .with_camera(&self.camera)
          .with_lens_id((lens_id as u32, 0))
          .with_mounts(&[SONY_E_MOUNT.into(), SONY_A_MOUNT.into()]);
        return Ok(resolver.resolve());
      }
    }

    // Try tag 0x940C
    if let Some(Entry {
      value: Value::Undefined(params),
      ..
    }) = self.makernote.get_entry(ArwMakernoteTag::Tag_940C)
    {
      let dechiphered_940c = sony_tag9cxx_decipher(params);
      let lens_id = LEu16(&dechiphered_940c, 9);
      debug!("Lens Id tag: {}", lens_id);

      let resolver = LensResolver::new()
        .with_camera(&self.camera)
        .with_lens_id((lens_id as u32, 0))
        .with_mounts(&[SONY_E_MOUNT.into(), SONY_A_MOUNT.into()]);
      return Ok(resolver.resolve());
    }
    Ok(None)
  }

  fn image_a100(&self, file: &RawSource, dummy: bool) -> Result<RawImage> {
    // We've caught the elusive A100 in the wild, a transitional format
    // between the simple sanity of the MRW custom format and the wordly
    // wonderfullness of the Tiff-based ARW format, let's shoot from the hip
    let data = self.tiff.find_ifds_with_tag(TiffCommonTag::SubIFDs);
    if data.is_empty() {
      return Err(RawlerError::DecoderFailed("ARW: Couldn't find the data IFD!".to_string()));
    }
    let raw = data[0];
    let width = 3880;
    let height = 2608;
    let offset = fetch_tiff_tag!(raw, TiffCommonTag::SubIFDs).force_usize(0);

    let src = file.subview_until_eof(offset as u64)?;
    let image = ArwDecoder::decode_arw1(src, width, height, dummy);

    // Get the WB the MRW way
    // DNGPrivateTag contains 4 bytes forming a LE u32 offset value.
    let priv_offset = {
      let entry = fetch_tiff_tag!(self.tiff, TiffCommonTag::DNGPrivateArea);
      assert_eq!(entry.value_type(), 0x1);
      LEu32(entry.get_data(), 0)
    };
    let buf = file.subview_until_eof(priv_offset as u64)?;
    if BEu32(buf, 0) != 0x4D5249 {
      // MRI
      return Err(format!("Invalid DNGPRIVATEDATA tag: 0x{:X}, expected 0x4D5249 ", BEu32(buf, 0)).into());
    }
    let mut currpos: usize = 8;
    let mut wb_coeffs: [f32; 4] = [1.0, 1.0, 1.0, f32::NAN];
    // At most we read 20 bytes from currpos so check we don't step outside that
    while currpos + 20 < buf.len() {
      let tag: u32 = BEu32(buf, currpos);
      let len: usize = LEu32(buf, currpos + 4) as usize;
      if tag == 0x574247 {
        // WBG
        wb_coeffs[0] = LEu16(buf, currpos + 12) as f32;
        wb_coeffs[1] = LEu16(buf, currpos + 14) as f32;
        wb_coeffs[2] = LEu16(buf, currpos + 14) as f32;
        wb_coeffs[3] = LEu16(buf, currpos + 18) as f32;
        break;
      }
      currpos += len + 8;
    }

    let cpp = 1;
    ok_cfa_image(self.camera.clone(), cpp, normalize_wb(wb_coeffs), image, dummy)
  }

  fn image_srf(&self, file: &RawSource, dummy: bool) -> Result<RawImage> {
    let data = self.tiff.find_ifds_with_tag(TiffCommonTag::ImageWidth);
    if data.is_empty() {
      return Err(RawlerError::DecoderFailed("ARW: Couldn't find the data IFD!".to_string()));
    }
    let raw = data[0];

    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);

    let image = if dummy {
      PixU16::new_uninit(width, height)
    } else {
      let buffer = file.as_vec()?;
      let len = width * height * 2;

      // Constants taken from dcraw
      let off: usize = 862144;
      let key_off: usize = 200896;
      let head_off: usize = 164600;

      // Replicate the dcraw contortions to get the "decryption" key
      let offset = (buffer[key_off] as usize) * 4;
      let first_key = BEu32(&buffer, key_off + offset);
      let head = ArwDecoder::sony_decrypt(&buffer, head_off, 40, first_key)?;
      let second_key = LEu32(&head, 22);

      // "Decrypt" the whole image buffer
      let image_data = ArwDecoder::sony_decrypt(&buffer, off, len, second_key)?;
      decode_16be(&image_data, width, height, dummy)
    };
    let cpp = 1;
    ok_cfa_image(self.camera.clone(), cpp, [f32::NAN, f32::NAN, f32::NAN, f32::NAN], image, dummy)
  }

  pub(crate) fn decode_arw1(buf: &[u8], width: usize, height: usize, dummy: bool) -> PixU16 {
    let mut out = alloc_image!(width, height, dummy);
    let mut pump = BitPumpMSB::new(buf);

    let mut sum: i32 = 0;
    for x in 0..width {
      let col = width - 1 - x;
      let mut row = 0;
      while row <= height {
        if row == height {
          row = 1;
        }

        let mut len: u32 = 4 - pump.get_bits(2);
        if len == 3 && pump.get_bits(1) != 0 {
          len = 0;
        } else if len == 4 {
          let zeros = pump.peek_bits(13).leading_zeros() - 19;
          len += zeros;
          pump.get_bits(cmp::min(13, zeros + 1));
        }
        let diff: i32 = pump.get_ibits(len);
        sum += diff;
        if len > 0 && (diff & (1 << (len - 1))) == 0 {
          sum -= (1 << len) - 1;
        }
        out[row * width + col] = sum as u16;
        row += 2
      }
    }
    out
  }

  pub(crate) fn decode_arw2(buf: &[u8], width: usize, height: usize, curve: &LookupTable, dummy: bool) -> PixU16 {
    decode_threaded(
      width,
      height,
      dummy,
      &(|out: &mut [u16], row| {
        let mut pump = BitPumpLSB::new(&buf[(row * width)..]);

        let mut random = pump.peek_bits(16);
        for out in out.chunks_exact_mut(32) {
          // Process 32 pixels at a time in interleaved fashion
          for j in 0..2 {
            let max = pump.get_bits(11);
            let min = pump.get_bits(11);
            let delta = max - min;
            // Calculate the size of the data shift needed by how large the delta is
            // A delta with 11 bits requires a shift of 4, 10 bits of 3, etc
            let delta_shift: u32 = cmp::max(0, (32 - (delta.leading_zeros() as i32)) - 7) as u32;
            let imax = pump.get_bits(4) as usize;
            let imin = pump.get_bits(4) as usize;

            for i in 0..16 {
              let val = if i == imax {
                max
              } else if i == imin {
                min
              } else {
                cmp::min(0x7ff, (pump.get_bits(7) << delta_shift) + min)
              };
              out[j + (i * 2)] = curve.dither((val << 1) as u16, &mut random);
            }
          }
        }
      }),
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
    let offsets = raw.get_entry(TiffCommonTag::TileOffsets).ok_or("Unable to find TileOffsets")?;
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let twidth = fetch_tiff_tag!(raw, TiffCommonTag::TileWidth).force_usize(0);
    let tlength = fetch_tiff_tag!(raw, TiffCommonTag::TileLength).force_usize(0);
    let cpp = fetch_tiff_tag!(raw, TiffCommonTag::SamplesPerPixel).force_usize(0);
    let coltiles = (width - 1) / twidth + 1;
    let rowtiles = (height - 1) / tlength + 1;

    log::debug!("Sony ARW LJPEG raw: width: {}, height: {}, cpp: {}", width, height, cpp);
    log::debug!("LJPEG tile parameters: width: {}, length: {}, cpp: {}", twidth, tlength, cpp);

    if coltiles * rowtiles != offsets.count() as usize {
      return Err(RawlerError::unsupported(
        camera,
        format!("ARW LJPEG: trying to decode {} tiles from {} offsets", coltiles * rowtiles, offsets.count()),
      ));
    }
    let buffer = file.as_vec()?;

    if cpp == 3 {
      let mut image = decode_threaded_multiline(
        width * cpp,
        height,
        tlength,
        dummy,
        &(|strip: &mut [u16], row| {
          let row = row / tlength;
          for col in 0..coltiles {
            log::debug!("Decode tile: row({}), col({})", row, col);
            let offset = offsets.force_usize(row * coltiles + col);
            let src = &buffer[offset..];
            let decompressor =
              LjpegDecompressor::new(src).map_err(|err| format!("Creating LJPEG decompressor for ARW LJPEG tile ({row},{col}) failed: {err}"))?;
            let cpp = 3;
            let w = 512;
            let h = 512;
            let mut data = vec![0; h * w * cpp];

            decompressor.decode_sony(&mut data, 0, w * cpp, w * cpp, h, dummy)?;
            interpolate_yuv(decompressor.super_h(), decompressor.super_v(), w * cpp, h, &mut data);

            let mut strip = &mut *strip;

            for line in data.chunks_exact(w * cpp) {
              let base = col * twidth * cpp;
              strip[base..base + w * cpp].copy_from_slice(line);

              // Now move output strip by one row.
              strip = &mut strip[width * cpp..];
            }
          }
          Ok(())
        }),
      )?;
      // Convert YC'bC'r data to RGB.
      ycbcr_to_rgb(&mut image.data);
      Ok(image)
    } else if cpp == 1 {
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
            let cpp = 4;
            let w = 256;
            let h = 256;
            let mut data = vec![0; h * w * cpp];

            decompressor.decode(&mut data, 0, w * cpp, w * cpp, h, dummy)?;

            let mut strip = &mut *strip;
            for line in data.chunks_exact(1024) {
              for (i, chunk) in line.chunks_exact(4).enumerate() {
                // Unpack chunks of RGGB pixel data into two output lines
                // so the first line is RGRGRG and the second one is GBGBGB.
                strip[col * twidth + i * 2 + 0] = chunk[0];
                strip[col * twidth + i * 2 + 1] = chunk[1];
                strip[width + col * twidth + i * 2 + 0] = chunk[2];
                strip[width + col * twidth + i * 2 + 1] = chunk[3];
              }
              // Now move output strip by two rows.
              strip = &mut strip[width * 2..];
            }
          }
          Ok(())
        }),
      )
      .map_err(RawlerError::DecoderFailed)
    } else {
      Err(RawlerError::unsupported(
        camera,
        format!("NRW files with LJPEG compression and unsupported cpp: {}", cpp),
      ))
    }
  }

  fn get_params(&self, file: &RawSource) -> Result<ArwImageParams> {
    let priv_offset = {
      let tag = fetch_tiff_tag!(self.tiff, TiffCommonTag::DNGPrivateArea).get_data();
      LEu32(tag, 0)
    };
    let priv_tiff = IFD::new(&mut file.reader(), priv_offset, 0, 0, Endian::Little, &[])?;

    //priv_tiff.dump::<ExifTag>(0).iter().for_each(|line| println!("DUMPXX: {}", line));

    let sony_offset = fetch_tiff_tag!(priv_tiff, TiffCommonTag::SonyOffset).force_u32(0);
    let sony_length = fetch_tiff_tag!(priv_tiff, TiffCommonTag::SonyLength).force_usize(0);
    // This tag is of type UNDEFINED and contains a 32 bit value
    let sony_key = {
      let tag = fetch_tiff_tag!(priv_tiff, TiffCommonTag::SonyKey).get_data();
      LEu32(tag, 0)
    };
    let buffer = file.as_vec()?;
    let decrypted_buf = ArwDecoder::sony_decrypt(&buffer, sony_offset as usize, sony_length, sony_key)?;

    let decrypted_tiff = IFD::new(&mut Cursor::new(decrypted_buf), 0, 0, -(sony_offset as i32), Endian::Little, &[])?;

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
      Err(RawlerError::DecoderFailed("ARW: Couldn't find GRGB or RGGB levels".to_string()))
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
    if buf.len() < offset + 4 * (length / 4) {
      return Err(RawlerError::DecoderFailed("sony_decrypt() failed: buffer to short".into()));
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

#[derive(Debug)]
struct ArwImageParams {
  wb: [f32; 4],
  blacklevel: Option<[u16; 4]>,
  whitelevel: Option<[u16; 4]>,
}

crate::tags::tiff_tag_enum!(SR2SubIFD);

/// Specific Canon CR2 Makernotes tags.
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
