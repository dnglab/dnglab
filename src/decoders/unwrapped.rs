use decoders::*;
use decoders::basics::*;

pub fn decode_unwrapped(buffer: &Buffer) -> Result<RawImageData,String> {
  let decoder = LEu16(&buffer.buf, 0);
  let width   = LEu16(&buffer.buf, 2) as usize;
  let height  = LEu16(&buffer.buf, 4) as usize;
  let data    = &buffer.buf[6..];

  if width > 64 || height > 64 {
    panic!("Trying an image larger than 64x64");
  }

  match decoder {
    0   => {
      let table = {
        let mut t: [u16;256] = [0;256];
        for i in 0..256 {
          t[i] = LEu16(data, i*2);
        }
        LookupTable::new(&t)
      };
      let data = &data[512..];
      Ok(RawImageData::Integer(decode_8bit_wtable(data, &table, width, height)))
    },
    1   => Ok(RawImageData::Integer(decode_10le_lsb16(data, width, height))),
    2   => Ok(RawImageData::Integer(decode_10le(data, width, height))),
    3   => Ok(RawImageData::Integer(decode_12be(data, width, height))),
    4   => Ok(RawImageData::Integer(decode_12be_msb16(data, width, height))),
    5   => Ok(RawImageData::Integer(decode_12le_16bitaligned(data, width, height))),
    6   => Ok(RawImageData::Integer(decode_12be_msb32(data, width, height))),
    7   => Ok(RawImageData::Integer(decode_12le_wcontrol(data, width, height))),
    8   => Ok(RawImageData::Integer(decode_12be_wcontrol(data, width, height))),
    9   => Ok(RawImageData::Integer(decode_12be_interlaced(data, width, height))),
    10  => Ok(RawImageData::Integer(decode_12be_interlaced_unaligned(data, width, height))),
    11  => Ok(RawImageData::Integer(decode_12le(data, width, height))),
    12  => Ok(RawImageData::Integer(decode_12le_unpacked(data, width, height))),
    13  => Ok(RawImageData::Integer(decode_12be_unpacked(data, width, height))),
    14  => Ok(RawImageData::Integer(decode_12be_unpacked_left_aligned(data, width, height))),
    15  => Ok(RawImageData::Integer(decode_12le_unpacked_left_aligned(data, width, height))),
    16  => Ok(RawImageData::Integer(decode_14le_unpacked(data, width, height))),
    17  => Ok(RawImageData::Integer(decode_14be_unpacked(data, width, height))),
    18  => Ok(RawImageData::Integer(decode_16le(data, width, height))),
    19  => Ok(RawImageData::Integer(decode_16le_skiplines(data, width, height))),
    20  => Ok(RawImageData::Integer(decode_16be(data, width, height))),
    21  => Ok(RawImageData::Integer(arw::ArwDecoder::decode_arw1(data, width, height))),
    22  => {
      let mut curve: [usize;6] = [ 0, 0, 0, 0, 0, 4095 ];
      for i in 0..4 {
        curve[i+1] = (LEu16(data, i*2) & 0xfff) as usize;
      }

      let curve = arw::ArwDecoder::calculate_curve(curve);
      let data = &data[8..];
      Ok(RawImageData::Integer(arw::ArwDecoder::decode_arw2(data, width, height, &curve)))
    },
    23  => {
      let key    = LEu32(data, 0);
      let length = LEu16(data, 4) as usize;
      let data   = &data[10..];

      if length > 5000 {
        panic!("Trying an SRF style image that's too big");
      }

      let image_data = arw::ArwDecoder::sony_decrypt(data, 0, length, key);
      Ok(RawImageData::Integer(decode_16be(&image_data, width, height)))
    },
    24  => Ok(RawImageData::Integer(orf::OrfDecoder::decode_compressed(data, width, height))),
    25  => {
      let loffsets = data;
      let data = &data[height*4..];
      Ok(RawImageData::Integer(srw::SrwDecoder::decode_srw1(data, loffsets, width, height)))
    },
    26  => Ok(RawImageData::Integer(srw::SrwDecoder::decode_srw2(data, width, height))),
    27  => Ok(RawImageData::Integer(srw::SrwDecoder::decode_srw3(data, width, height))),
    28  => Ok(RawImageData::Integer(kdc::KdcDecoder::decode_dc120(data, width, height))),
    29  => Ok(RawImageData::Integer(rw2::Rw2Decoder::decode_panasonic(data, width, height, false))),
    30  => Ok(RawImageData::Integer(rw2::Rw2Decoder::decode_panasonic(data, width, height, true))),
    31  => {
      let table = {
        let mut t = [0u16;1024];
        for i in 0..1024 {
          t[i] = LEu16(data, i*2);
        }
        LookupTable::new(&t)
      };
      let data = &data[2048..];
      Ok(RawImageData::Integer(dcr::DcrDecoder::decode_kodak65000(data, &table, width, height)))
    },
    32  => decode_ljpeg(data, width, height, false, false),
    33  => decode_ljpeg(data, width, height, false, true),
    34  => decode_ljpeg(data, width, height, true, false),
    35  => decode_ljpeg(data, width, height, true, true),
    36  => Ok(RawImageData::Integer(pef::PefDecoder::do_decode(data, None, width, height).unwrap())),
    37  => {
      let huff = data;
      let data = &data[64..];
      Ok(RawImageData::Integer(
        pef::PefDecoder::do_decode(data, Some((huff, LITTLE_ENDIAN)), width, height).unwrap()
      ))
    },
    38  => {
      let huff = data;
      let data = &data[64..];
      Ok(RawImageData::Integer(
        pef::PefDecoder::do_decode(data, Some((huff, BIG_ENDIAN)), width, height).unwrap()
      ))
    },
    39  => Ok(RawImageData::Integer(crw::CrwDecoder::do_decode(data, false, 0, width, height))),
    40  => Ok(RawImageData::Integer(crw::CrwDecoder::do_decode(data, false, 1, width, height))),
    41 => Ok(RawImageData::Integer(crw::CrwDecoder::do_decode(data, false, 2, width, height))),
    42  => Ok(RawImageData::Integer(crw::CrwDecoder::do_decode(data, true, 0, width, height))),
    43  => Ok(RawImageData::Integer(crw::CrwDecoder::do_decode(data, true, 1, width, height))),
    44  => Ok(RawImageData::Integer(crw::CrwDecoder::do_decode(data, true, 2, width, height))),
    45  => Ok(RawImageData::Integer(mos::MosDecoder::do_decode(data, false, width, height).unwrap())),
    46  => Ok(RawImageData::Integer(mos::MosDecoder::do_decode(data, true, width, height).unwrap())),
    47  => Ok(RawImageData::Integer(iiq::IiqDecoder::decode_compressed(data, height*4, 0, width, height))),
    48  => decode_nef(data, width, height, LITTLE_ENDIAN, 12),
    49  => decode_nef(data, width, height, LITTLE_ENDIAN, 14),
    50  => decode_nef(data, width, height, BIG_ENDIAN, 12),
    51  => decode_nef(data, width, height, BIG_ENDIAN, 14),
    52  => {
      let coeffs = [LEf32(data,0), LEf32(data,4), LEf32(data,8), LEf32(data,12)];
      let data = &data[16..];
      Ok(RawImageData::Integer(nef::NefDecoder::decode_snef_compressed(data, coeffs, width, height)))
    },
    _   => Err("No such decoder".to_string()),
  }
}

fn decode_ljpeg(src: &[u8], width: usize, height: usize, dng_bug: bool, csfix: bool) -> Result<RawImageData,String> {
  let mut out = vec![0u16; width*height];
  let decompressor = try!(ljpeg::LjpegDecompressor::new_full(src, dng_bug, csfix));
  try!(decompressor.decode(&mut out, 0, width, width, height));
  Ok(RawImageData::Integer(out))
}

fn decode_nef(data: &[u8], width: usize, height: usize, endian: Endian, bps: usize) -> Result<RawImageData,String> {
  let meta = data;
  let data = &data[4096..];
  Ok(RawImageData::Integer(nef::NefDecoder::do_decode(data, meta, endian, width, height, bps).unwrap()))
}
