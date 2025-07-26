use crate::bits::Endian;
use crate::bits::*;
use crate::buffer::PaddedBuf;
use crate::decoders::*;
use crate::decompressors::ljpeg::LjpegDecompressor;
use crate::packed::*;

pub fn decode_unwrapped(file: &RawSource) -> Result<RawImageData> {
  let buffer = file.subview_until_eof_padded(0)?;

  let decoder = LEu16(&buffer, 0);
  let width = LEu16(&buffer, 2) as usize;
  let height = LEu16(&buffer, 4) as usize;
  let data = &buffer[6..];

  if width > 64 || height > 64 {
    panic!("Trying an image larger than 64x64");
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
      let data = &data[512..];
      Ok(RawImageData::Integer(decode_8bit_wtable(data, &table, width, height, false).into_inner()))
    }
    1 => Ok(RawImageData::Integer(decode_10le_lsb16(data, width, height, false).into_inner())),
    2 => Ok(RawImageData::Integer(decode_10be(data, width, height, false).into_inner())),
    3 => Ok(RawImageData::Integer(decode_12be(data, width, height, false).into_inner())),
    4 => Ok(RawImageData::Integer(decode_12be_msb16(data, width, height, false).into_inner())),
    5 => Ok(RawImageData::Integer(decode_12le_16bitaligned(data, width, height, false).into_inner())),
    6 => Ok(RawImageData::Integer(decode_12be_msb32(data, width, height, false).into_inner())),
    7 => Ok(RawImageData::Integer(decode_12le_wcontrol(data, width, height, false).into_inner())),
    8 => Ok(RawImageData::Integer(decode_12be_wcontrol(data, width, height, false).into_inner())),
    9 => Ok(RawImageData::Integer(decode_12be_interlaced(data, width, height, false).into_inner())),
    10 => Ok(RawImageData::Integer(decode_12be_interlaced_unaligned(data, width, height, false).into_inner())),
    11 => Ok(RawImageData::Integer(decode_12le(data, width, height, false).into_inner())),
    12 => Ok(RawImageData::Integer(decode_12le_unpacked(data, width, height, false).into_inner())),
    13 => Ok(RawImageData::Integer(decode_12be_unpacked(data, width, height, false).into_inner())),
    14 => Ok(RawImageData::Integer(
      decode_12be_unpacked_left_aligned(data, width, height, false).into_inner(),
    )),
    15 => Ok(RawImageData::Integer(
      decode_12le_unpacked_left_aligned(data, width, height, false).into_inner(),
    )),
    16 => Ok(RawImageData::Integer(decode_14le_unpacked(data, width, height, false).into_inner())),
    17 => Ok(RawImageData::Integer(decode_14be_unpacked(data, width, height, false).into_inner())),
    18 => Ok(RawImageData::Integer(decode_16le(data, width, height, false).into_inner())),
    19 => Ok(RawImageData::Integer(decode_16le_skiplines(data, width, height, false).into_inner())),
    20 => Ok(RawImageData::Integer(decode_16be(data, width, height, false).into_inner())),
    21 => Ok(RawImageData::Integer(arw::ArwDecoder::decode_arw1(data, width, height, false).into_inner())),
    22 => {
      let mut curve: [usize; 6] = [0, 0, 0, 0, 0, 4095];
      for i in 0..4 {
        curve[i + 1] = (LEu16(data, i * 2) & 0xfff) as usize;
      }

      let curve = arw::ArwDecoder::calculate_curve(curve);
      let data = &data[8..];
      Ok(RawImageData::Integer(
        arw::ArwDecoder::decode_arw2(data, width, height, &curve, false).into_inner(),
      ))
    }
    23 => {
      let key = LEu32(data, 0);
      let length = LEu16(data, 4) as usize;
      let data = &data[10..];

      if length > 5000 {
        panic!("Trying an SRF style image that's too big");
      }

      let image_data = arw::ArwDecoder::sony_decrypt(data, 0, length, key)?;
      Ok(RawImageData::Integer(decode_16be(&image_data, width, height, false).into_inner()))
    }
    24 => Ok(RawImageData::Integer(
      orf::OrfDecoder::decode_compressed(&buffer, width, height, false).into_inner(),
    )),
    25 => {
      let loffsets = data;
      let data = &data[height * 4..];
      Ok(RawImageData::Integer(
        srw::SrwDecoder::decode_srw1(data, loffsets, width, height, false).into_inner(),
      ))
    }
    26 => Ok(RawImageData::Integer(srw::SrwDecoder::decode_srw2(data, width, height, false).into_inner())),
    27 => Ok(RawImageData::Integer(srw::SrwDecoder::decode_srw3(data, width, height, false).into_inner())),
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
      let data = &data[2048..];
      Ok(RawImageData::Integer(
        dcr::DcrDecoder::decode_kodak65000(data, &table, width, height, false).into_inner(),
      ))
    }
    32 => decode_ljpeg(data, width, height, false, false),
    33 => decode_ljpeg(data, width, height, false, true),
    34 => decode_ljpeg(data, width, height, true, false),
    35 => decode_ljpeg(data, width, height, true, true),
    36 => Ok(RawImageData::Integer(
      pef::PefDecoder::do_decode(data, None, width, height, false).unwrap().into_inner(),
    )),
    37 => {
      let huff = data;
      let data = &data[64..];
      Ok(RawImageData::Integer(
        pef::PefDecoder::do_decode(data, Some((huff, Endian::Little)), width, height, false)
          .unwrap()
          .into_inner(),
      ))
    }
    38 => {
      let huff = data;
      let data = &data[64..];
      Ok(RawImageData::Integer(
        pef::PefDecoder::do_decode(data, Some((huff, Endian::Big)), width, height, false)
          .unwrap()
          .into_inner(),
      ))
    }
    39 => Ok(RawImageData::Integer(
      crw::CrwDecoder::do_decode(file, false, 0, width, height, false).unwrap().into_inner(),
    )),
    40 => Ok(RawImageData::Integer(
      crw::CrwDecoder::do_decode(file, false, 1, width, height, false).unwrap().into_inner(),
    )),
    41 => Ok(RawImageData::Integer(
      crw::CrwDecoder::do_decode(file, false, 2, width, height, false).unwrap().into_inner(),
    )),
    42 => Ok(RawImageData::Integer(
      crw::CrwDecoder::do_decode(file, true, 0, width, height, false).unwrap().into_inner(),
    )),
    43 => Ok(RawImageData::Integer(
      crw::CrwDecoder::do_decode(file, true, 1, width, height, false).unwrap().into_inner(),
    )),
    44 => Ok(RawImageData::Integer(
      crw::CrwDecoder::do_decode(file, true, 2, width, height, false).unwrap().into_inner(),
    )),
    45 => Ok(RawImageData::Integer(
      mos::MosDecoder::do_decode(data, false, width, height, false).unwrap().into_inner(),
    )),
    46 => Ok(RawImageData::Integer(
      mos::MosDecoder::do_decode(data, true, width, height, false).unwrap().into_inner(),
    )),
    //47  => Ok(RawImageData::Integer(iiq::IiqDecoder::decode_compressed(data, height*4, 0, width, height, false).into_inner())),
    48 => decode_nef(data, width, height, Endian::Little, 12),
    49 => decode_nef(data, width, height, Endian::Little, 14),
    50 => decode_nef(data, width, height, Endian::Big, 12),
    51 => decode_nef(data, width, height, Endian::Big, 14),
    52 => {
      let coeffs = [LEf32(data, 0), LEf32(data, 4), LEf32(data, 8), LEf32(data, 12)];
      let data = PaddedBuf::new_owned(data[16..].to_vec(), data.len() - 16);
      Ok(RawImageData::Integer(
        nef::NefDecoder::decode_snef_compressed(&data, coeffs, width, height, false).into_inner(),
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
  let data = &data[4096..];
  Ok(RawImageData::Integer(
    nef::NefDecoder::do_decode(data, meta, endian, width, height, bps, false).unwrap().into_inner(),
  ))
}
