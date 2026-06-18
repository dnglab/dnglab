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
  // This is a fuzzing/test-only entry point; the 6-byte header (decoder, width,
  // height) is followed by the payload. Guard the slice so a sub-6-byte input
  // errors instead of panicking. The `LEu*` readers above are already EOF-safe.
  let data = buffer.get(6..).ok_or("Unwrapped: input too small for header")?;

  if width > 64 || height > 64 {
    return Err(RawlerError::DecoderFailed(format!("Unwrapped: image {}x{} exceeds 64x64 limit", width, height)));
  }
  // Several unwrapped decoders below call into helpers that `.expect()` a
  // successful decompress (e.g. Panasonic v4), and a 0-width/0-height image makes
  // those helpers fail (zero chunk size). A real image has non-zero dimensions,
  // so rejecting them here is invisible to valid input and keeps the decoders
  // from hitting an internal panic.
  if width == 0 || height == 0 {
    return Err(RawlerError::DecoderFailed(format!(
      "Unwrapped: image {}x{} has a zero dimension",
      width, height
    )));
  }

  match decoder {
    0 => {
      let table = {
        let mut t: [u16; 256] = [0; 256];
        for i in 0..256 {
          t[i] = LEu16(data, i * 2);
        }
        LookupTable::new(&t)
      };
      let data = data.get(512..).ok_or("Unwrapped: input too small for 8bit table data")?;
      Ok(RawImageData::Integer(decompress_8bit_wtable(data, &table, width, height, false)?.into_inner()))
    }
    1 => Ok(RawImageData::Integer(decompress_10le_lsb16(data, width, height, false)?.into_inner())),
    2 => Ok(RawImageData::Integer(decompress_10be(data, width, height, false)?.into_inner())),
    3 => Ok(RawImageData::Integer(decompress_12be(data, width, height, false)?.into_inner())),
    4 => Ok(RawImageData::Integer(decompress_12be_msb16(data, width, height, false)?.into_inner())),
    5 => Ok(RawImageData::Integer(decompress_12le_16bitaligned(data, width, height, false)?.into_inner())),
    6 => Ok(RawImageData::Integer(decompress_12be_msb32(data, width, height, false)?.into_inner())),
    7 => Ok(RawImageData::Integer(decompress_12le_wcontrol(data, width, height, false)?.into_inner())),
    8 => Ok(RawImageData::Integer(decompress_12be_wcontrol(data, width, height, false)?.into_inner())),
    9 => Ok(RawImageData::Integer(decompress_12be_interlaced(data, width, height, false)?.into_inner())),
    10 => Ok(RawImageData::Integer(
      decompress_12be_interlaced_unaligned(data, width, height, false)?.into_inner(),
    )),
    11 => Ok(RawImageData::Integer(decompress_12le(data, width, height, false)?.into_inner())),
    12 => Ok(RawImageData::Integer(decompress_12le_unpacked(data, width, height, false)?.into_inner())),
    13 => Ok(RawImageData::Integer(decompress_12be_unpacked(data, width, height, false)?.into_inner())),
    14 => Ok(RawImageData::Integer(
      decompress_12be_unpacked_left_aligned(data, width, height, false)?.into_inner(),
    )),
    15 => Ok(RawImageData::Integer(
      decompress_12le_unpacked_left_aligned(data, width, height, false)?.into_inner(),
    )),
    16 => Ok(RawImageData::Integer(decompress_14le_unpacked(data, width, height, false)?.into_inner())),
    17 => Ok(RawImageData::Integer(decompress_14be_unpacked(data, width, height, false)?.into_inner())),
    18 => Ok(RawImageData::Integer(decompress_16le(data, width, height, false)?.into_inner())),
    19 => Ok(RawImageData::Integer(decompress_16le_skiplines(data, width, height, false)?.into_inner())),
    20 => Ok(RawImageData::Integer(decompress_16be(data, width, height, false)?.into_inner())),
    21 => Ok(RawImageData::Integer(arw::ArwDecoder::decode_arw1(data, width, height, false)?.into_inner())),
    22 => {
      let mut curve: [usize; 6] = [0, 0, 0, 0, 0, 4095];
      for i in 0..4 {
        curve[i + 1] = (LEu16(data, i * 2) & 0xfff) as usize;
      }

      let curve = arw::ArwDecoder::calculate_curve(curve);
      let data = data.get(8..).ok_or("Unwrapped: input too small for arw2 curve")?;
      Ok(RawImageData::Integer(
        arw::ArwDecoder::decode_arw2(data, width, height, &curve, false)?.into_inner(),
      ))
    }
    23 => {
      let key = LEu32(data, 0);
      let length = LEu16(data, 4) as usize;
      let data = data.get(10..).ok_or("Unwrapped: input too small for SRF header")?;

      if length > 5000 {
        return Err(RawlerError::DecoderFailed(format!("Unwrapped: SRF image length {} exceeds 5000 limit", length)));
      }

      let image_data = arw::ArwDecoder::sony_decrypt(data, 0, length, key)?;
      Ok(RawImageData::Integer(decompress_16be(&image_data, width, height, false)?.into_inner()))
    }
    24 => Ok(RawImageData::Integer(
      orf::OrfDecoder::decode_compressed(&buffer, width, height, 12, false).into_inner(),
    )),
    25 => {
      let loffsets = data;
      // height <= 64 (checked above), so `height * 4` cannot overflow; guard the
      // slice for inputs shorter than the line-offset table.
      let data = data.get(height * 4..).ok_or("Unwrapped: input too small for srw1 line offsets")?;
      Ok(RawImageData::Integer(
        srw::SrwDecoder::decode_srw1(data, loffsets, width, height, false).into_inner(),
      ))
    }
    26 => Ok(RawImageData::Integer(srw::SrwDecoder::decode_srw2(data, width, height, false).into_inner())),
    27 => Ok(RawImageData::Integer(srw::SrwDecoder::decode_srw3(data, width, height, false)?.into_inner())),
    28 => Ok(RawImageData::Integer(kdc::KdcDecoder::decode_dc120(data, width, height, false).into_inner())),
    29 => Ok(RawImageData::Integer(
      rw2::v4decompressor::decode_panasonic_v4(data, width, height, false, false).into_inner(),
    )),
    30 => Ok(RawImageData::Integer(
      rw2::v4decompressor::decode_panasonic_v4(data, width, height, true, false).into_inner(),
    )),
    31 => {
      let table = {
        let mut t = [0u16; 1024];
        for i in 0..1024 {
          t[i] = LEu16(data, i * 2);
        }
        LookupTable::new(&t)
      };
      let data = data.get(2048..).ok_or("Unwrapped: input too small for kodak65000 table data")?;
      Ok(RawImageData::Integer(
        dcr::DcrDecoder::decode_kodak65000(data, &table, width, height, false).into_inner(),
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
      let data = data.get(64..).ok_or("Unwrapped: input too small for PEF huffman table")?;
      Ok(RawImageData::Integer(
        pef::PefDecoder::do_decode(data, Some((huff, Endian::Little)), width, height, false)?.into_inner(),
      ))
    }
    38 => {
      let huff = data;
      let data = data.get(64..).ok_or("Unwrapped: input too small for PEF huffman table")?;
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
    48 => decode_nef(data, width, height, Endian::Little, 12),
    49 => decode_nef(data, width, height, Endian::Little, 14),
    50 => decode_nef(data, width, height, Endian::Big, 12),
    51 => decode_nef(data, width, height, Endian::Big, 14),
    52 => {
      let coeffs = [LEf32(data, 0), LEf32(data, 4), LEf32(data, 8), LEf32(data, 12)];
      let payload = data.get(16..).ok_or("Unwrapped: input too small for snef coeffs")?;
      let data = PaddedBuf::new_owned(payload.to_vec(), payload.len());
      Ok(RawImageData::Integer(
        nef::NefDecoder::decode_snef_compressed(&data, coeffs, width, height, false)?.into_inner(),
      ))
    }
    _ => Err("No such decoder".into()),
  }
}

fn decode_ljpeg(src: &[u8], width: usize, height: usize, dng_bug: bool, csfix: bool) -> Result<RawImageData> {
  let mut out = vec![0u16; width * height];
  let decompressor = LjpegDecompressor::new_full(src, dng_bug, csfix)?;
  decompressor.decode(&mut out, 0, width, width, height, false)?;
  Ok(RawImageData::Integer(out))
}

fn decode_nef(data: &[u8], width: usize, height: usize, endian: Endian, bps: usize) -> Result<RawImageData> {
  let meta = data;
  let data = data.get(4096..).ok_or("Unwrapped: input too small for NEF metadata block")?;
  Ok(RawImageData::Integer(
    nef::NefDecoder::do_decode(data, meta, endian, width, height, bps, false)?.into_inner(),
  ))
}
