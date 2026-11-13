use crate::bits::Endian;
use crate::bits::*;
use crate::buffer::PaddedBuf;
use crate::decoders::*;
use crate::decompressors::ljpeg::LjpegDecompressor;
use crate::decompressors::packed::*;

pub fn decode_unwrapped(file: &RawSource) -> Result<RawImageData> {
  let buffer = file.subview_until_eof_padded(0)?;

  let decoder = LEu16(&buffer, 0);
  let width = LEu16(&buffer, 2) as usize;
  let height = LEu16(&buffer, 4) as usize;
  let data = &buffer[6..];

  if width > 64 || height > 64 {
    return Err(decoder_error!("Unwrapped: image {}x{} exceeds 64x64 limit", width, height));
  }
  // An image without pixels panics in the decompressors
  image_bytes("Unwrapped", width, height, 16)?;

  match decoder {
    0 => {
      let (curve, data) = prefix(data, 512, decoder)?;
      let table = {
        let mut t: [u16; 256] = [0; 256];
        for i in 0..256 {
          t[i] = LEu16(curve, i * 2);
        }
        LookupTable::new(&t)
      };
      Ok(RawImageData::Integer(
        decompress_8bit_wtable(data, &table, width, height, false)
          .map_err(RawlerError::decoder_failed)?
          .into_inner(),
      ))
    }
    1 => Ok(RawImageData::Integer(
      decompress_10le_lsb16(data, width, height, false)
        .map_err(RawlerError::decoder_failed)?
        .into_inner(),
    )),
    2 => Ok(RawImageData::Integer(
      decompress_10be(data, width, height, false).map_err(RawlerError::decoder_failed)?.into_inner(),
    )),
    3 => Ok(RawImageData::Integer(
      decompress_12be(data, width, height, false).map_err(RawlerError::decoder_failed)?.into_inner(),
    )),
    4 => Ok(RawImageData::Integer(
      decompress_12be_msb16(data, width, height, false)
        .map_err(RawlerError::decoder_failed)?
        .into_inner(),
    )),
    5 => Ok(RawImageData::Integer(
      decompress_12le_16bitaligned(data, width, height, false)
        .map_err(RawlerError::decoder_failed)?
        .into_inner(),
    )),
    6 => Ok(RawImageData::Integer(
      decompress_12be_msb32(data, width, height, false)
        .map_err(RawlerError::decoder_failed)?
        .into_inner(),
    )),
    7 => Ok(RawImageData::Integer(
      decompress_12le_wcontrol(data, width, height, false)
        .map_err(RawlerError::decoder_failed)?
        .into_inner(),
    )),
    8 => Ok(RawImageData::Integer(
      decompress_12be_wcontrol(data, width, height, false)
        .map_err(RawlerError::decoder_failed)?
        .into_inner(),
    )),
    9 => Ok(RawImageData::Integer(
      decompress_12be_interlaced(data, width, height, false)
        .map_err(RawlerError::decoder_failed)?
        .into_inner(),
    )),
    10 => Ok(RawImageData::Integer(
      decompress_12be_interlaced_unaligned(data, width, height, false)
        .map_err(RawlerError::decoder_failed)?
        .into_inner(),
    )),
    11 => Ok(RawImageData::Integer(
      decompress_12le(data, width, height, false).map_err(RawlerError::decoder_failed)?.into_inner(),
    )),
    12 => Ok(RawImageData::Integer(
      decompress_12le_unpacked(data, width, height, false)
        .map_err(RawlerError::decoder_failed)?
        .into_inner(),
    )),
    13 => Ok(RawImageData::Integer(
      decompress_12be_unpacked(data, width, height, false)
        .map_err(RawlerError::decoder_failed)?
        .into_inner(),
    )),
    14 => Ok(RawImageData::Integer(
      decompress_12be_unpacked_left_aligned(data, width, height, false)
        .map_err(RawlerError::decoder_failed)?
        .into_inner(),
    )),
    15 => Ok(RawImageData::Integer(
      decompress_12le_unpacked_left_aligned(data, width, height, false)
        .map_err(RawlerError::decoder_failed)?
        .into_inner(),
    )),
    16 => Ok(RawImageData::Integer(
      decompress_14le_unpacked(data, width, height, false)
        .map_err(RawlerError::decoder_failed)?
        .into_inner(),
    )),
    17 => Ok(RawImageData::Integer(
      decompress_14be_unpacked(data, width, height, false)
        .map_err(RawlerError::decoder_failed)?
        .into_inner(),
    )),
    18 => Ok(RawImageData::Integer(
      decompress_16le(data, width, height, false).map_err(RawlerError::decoder_failed)?.into_inner(),
    )),
    19 => Ok(RawImageData::Integer(
      decompress_16le_skiplines(data, width, height, false)
        .map_err(RawlerError::decoder_failed)?
        .into_inner(),
    )),
    20 => Ok(RawImageData::Integer(
      decompress_16be(data, width, height, false).map_err(RawlerError::decoder_failed)?.into_inner(),
    )),
    21 => Ok(RawImageData::Integer(arw::ArwDecoder::decode_arw1(data, width, height, false)?.into_inner())),
    22 => {
      let (points, data) = prefix(data, 8, decoder)?;
      let mut curve: [usize; 6] = [0, 0, 0, 0, 0, 4095];
      for i in 0..4 {
        curve[i + 1] = (LEu16(points, i * 2) & 0xfff) as usize;
      }

      let curve = arw::ArwDecoder::calculate_curve(curve);
      Ok(RawImageData::Integer(
        arw::ArwDecoder::decode_arw2(data, width, height, &curve, false)?.into_inner(),
      ))
    }
    23 => {
      let (header, data) = prefix(data, 10, decoder)?;
      let key = LEu32(header, 0);
      let length = LEu16(header, 4) as usize;

      if length > 5000 {
        return Err(decoder_error!("Unwrapped: SRF image length {} exceeds 5000 limit", length));
      }

      let image_data = arw::ArwDecoder::sony_decrypt(data, 0, length, key)?;
      Ok(RawImageData::Integer(
        decompress_16be(&image_data, width, height, false)
          .map_err(RawlerError::decoder_failed)?
          .into_inner(),
      ))
    }
    24 => Ok(RawImageData::Integer(
      orf::OrfDecoder::decode_compressed(&buffer, width, height, 12, false)?.into_inner(),
    )),
    25 => {
      let loffsets = data;
      let (_, data) = prefix(data, height * 4, decoder)?;
      Ok(RawImageData::Integer(
        srw::SrwDecoder::decode_srw1(data, loffsets, width, height, false)?.into_inner(),
      ))
    }
    26 => Ok(RawImageData::Integer(srw::SrwDecoder::decode_srw2(data, width, height, false)?.into_inner())),
    27 => Ok(RawImageData::Integer(srw::SrwDecoder::decode_srw3(data, width, height, false)?.into_inner())),
    28 => Ok(RawImageData::Integer(kdc::KdcDecoder::decode_dc120(data, width, height, false)?.into_inner())),
    29 => Ok(RawImageData::Integer(
      rw2::v4decompressor::decode_panasonic_v4(data, width, height, false, false)
        .map_err(RawlerError::decoder_failed)?
        .into_inner(),
    )),
    30 => Ok(RawImageData::Integer(
      rw2::v4decompressor::decode_panasonic_v4(data, width, height, true, false)
        .map_err(RawlerError::decoder_failed)?
        .into_inner(),
    )),
    31 => {
      let (curve, data) = prefix(data, 2048, decoder)?;
      let table = {
        let mut t = [0u16; 1024];
        for i in 0..1024 {
          t[i] = LEu16(curve, i * 2);
        }
        LookupTable::new(&t)
      };
      Ok(RawImageData::Integer(
        dcr::DcrDecoder::decode_kodak65000(data, &table, width, height, false)?.into_inner(),
      ))
    }
    32 => decode_ljpeg(data, width, height, false, false),
    33 => decode_ljpeg(data, width, height, false, true),
    34 => decode_ljpeg(data, width, height, true, false),
    35 => decode_ljpeg(data, width, height, true, true),
    36 => Ok(RawImageData::Integer(
      pef::PefDecoder::do_decode(data, None, width, height, false)?.into_inner(),
    )),
    37 => {
      let huff = data;
      let (_, data) = prefix(data, 64, decoder)?;
      Ok(RawImageData::Integer(
        pef::PefDecoder::do_decode(data, Some((huff, Endian::Little)), width, height, false)?.into_inner(),
      ))
    }
    38 => {
      let huff = data;
      let (_, data) = prefix(data, 64, decoder)?;
      Ok(RawImageData::Integer(
        pef::PefDecoder::do_decode(data, Some((huff, Endian::Big)), width, height, false)?.into_inner(),
      ))
    }
    39 => Ok(RawImageData::Integer(
      crw::CrwDecoder::do_decode(file, false, 0, width, height, false)?.into_inner(),
    )),
    40 => Ok(RawImageData::Integer(
      crw::CrwDecoder::do_decode(file, false, 1, width, height, false)?.into_inner(),
    )),
    41 => Ok(RawImageData::Integer(
      crw::CrwDecoder::do_decode(file, false, 2, width, height, false)?.into_inner(),
    )),
    42 => Ok(RawImageData::Integer(
      crw::CrwDecoder::do_decode(file, true, 0, width, height, false)?.into_inner(),
    )),
    43 => Ok(RawImageData::Integer(
      crw::CrwDecoder::do_decode(file, true, 1, width, height, false)?.into_inner(),
    )),
    44 => Ok(RawImageData::Integer(
      crw::CrwDecoder::do_decode(file, true, 2, width, height, false)?.into_inner(),
    )),
    45 => Ok(RawImageData::Integer(
      mos::MosDecoder::do_decode(data, false, width, height, false)?.into_inner(),
    )),
    46 => Ok(RawImageData::Integer(
      mos::MosDecoder::do_decode(data, true, width, height, false)?.into_inner(),
    )),
    //47  => Ok(RawImageData::Integer(iiq::IiqDecoder::decode_compressed(data, height*4, 0, width, height, false).into_inner())),
    48 => decode_nef(decoder, data, width, height, Endian::Little, 12),
    49 => decode_nef(decoder, data, width, height, Endian::Little, 14),
    50 => decode_nef(decoder, data, width, height, Endian::Big, 12),
    51 => decode_nef(decoder, data, width, height, Endian::Big, 14),
    52 => {
      let (coeffs, data) = prefix(data, 16, decoder)?;
      let coeffs = [LEf32(coeffs, 0), LEf32(coeffs, 4), LEf32(coeffs, 8), LEf32(coeffs, 12)];
      let data = PaddedBuf::new_owned(data.to_vec(), data.len());
      Ok(RawImageData::Integer(
        nef::NefDecoder::decode_snef_compressed(&data, coeffs, width, height, false)
          .map_err(RawlerError::decoder_failed)?
          .into_inner(),
      ))
    }
    _ => Err(RawlerError::decoder_failed("No such decoder")),
  }
}

fn decode_ljpeg(src: &[u8], width: usize, height: usize, dng_bug: bool, csfix: bool) -> Result<RawImageData> {
  let mut out = vec![0u16; width * height];
  let decompressor = LjpegDecompressor::new_full(src, dng_bug, csfix).map_err(RawlerError::decoder_failed)?;
  decompressor
    .decode(&mut out, 0, width, width, height, false)
    .map_err(RawlerError::decoder_failed)?;
  Ok(RawImageData::Integer(out))
}

/// The first `len` bytes of `data` and the rest, an error for shorter data
fn prefix(data: &[u8], len: usize, decoder: u16) -> Result<(&[u8], &[u8])> {
  if data.len() < len {
    return Err(decoder_error!(
      "Unwrapped: decoder {} needs {} bytes of data, there are {}",
      decoder,
      len,
      data.len()
    ));
  }
  Ok(data.split_at(len))
}

fn decode_nef(decoder: u16, data: &[u8], width: usize, height: usize, endian: Endian, bps: usize) -> Result<RawImageData> {
  let meta = data;
  let (_, data) = prefix(data, 4096, decoder)?;
  Ok(RawImageData::Integer(
    nef::NefDecoder::do_decode(data, meta, endian, width, height, bps, false)
      .map_err(RawlerError::decoder_failed)?
      .into_inner(),
  ))
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Unwrapped file of `decoder` for an image of `width` × `height` pixels
  fn unwrapped(decoder: u16, width: u16, height: u16, data: &[u8]) -> RawSource {
    let mut file = Vec::new();
    for value in [decoder, width, height] {
      file.extend_from_slice(&value.to_le_bytes());
    }
    file.extend_from_slice(data);
    RawSource::new_from_slice(&file)
  }

  #[test]
  fn short_data_is_an_error() {
    // Prefix length of each decoder for an image of 4x64 pixels, the data
    // has 16 bytes of padding
    for (decoder, len) in [(0, 512), (25, 64 * 4), (31, 2048), (37, 64), (38, 64), (48, 4096), (51, 4096)] {
      let data = vec![0; len - 17];
      assert!(decode_unwrapped(&unwrapped(decoder, 4, 64, &data)).is_err(), "decoder {decoder}");
    }
    // A file of 5 bytes, the high byte of the height is padding. Its 15
    // bytes of data are less than the 16 bytes of the coefficients.
    assert!(decode_unwrapped(&RawSource::new_from_slice(&[52, 0, 4, 0, 4])).is_err());
  }

  #[test]
  fn empty_image_is_an_error() {
    for decoder in [1, 18, 22, 29] {
      for (width, height) in [(0, 4), (4, 0)] {
        assert!(decode_unwrapped(&unwrapped(decoder, width, height, &[0; 600])).is_err(), "decoder {decoder}");
      }
    }
  }

  #[test]
  fn tiny_images_decode() -> Result<()> {
    // 16-bit little-endian values
    let data: Vec<u8> = (1..=4_u16).flat_map(|v| v.to_le_bytes()).collect();
    let RawImageData::Integer(pixels) = decode_unwrapped(&unwrapped(18, 2, 2, &data))? else {
      panic!("integer data expected");
    };
    assert_eq!(pixels, [1, 2, 3, 4]);
    // An 8-bit table that maps every value to 100, then 4 pixels
    let mut data: Vec<u8> = [100_u16; 256].iter().flat_map(|v| v.to_le_bytes()).collect();
    data.extend_from_slice(&[1, 2, 3, 4]);
    let RawImageData::Integer(pixels) = decode_unwrapped(&unwrapped(0, 2, 2, &data))? else {
      panic!("integer data expected");
    };
    assert_eq!(pixels, [100; 4]);
    Ok(())
  }
}
