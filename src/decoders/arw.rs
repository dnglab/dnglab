use decoders::*;
use decoders::tiff::*;
use decoders::basics::*;
use std::mem::transmute;
use std::f32::NAN;
use std::cmp;

#[derive(Debug, Clone)]
pub struct ArwDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: TiffIFD<'a>,
}

impl<'a> ArwDecoder<'a> {
  pub fn new(buf: &'a [u8], tiff: TiffIFD<'a>, rawloader: &'a RawLoader) -> ArwDecoder<'a> {
    ArwDecoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for ArwDecoder<'a> {
  fn image(&self) -> Result<Image,String> {
    let camera = try!(self.rawloader.check_supported(&self.tiff));
    let data = self.tiff.find_ifds_with_tag(Tag::StripOffsets);
    if data.len() == 0 {
      if camera.model == "DSLR-A100" {
        return self.image_a100(camera)
      } else { // try decoding as SRF
        return self.image_srf(camera)
      }
    }
    let raw = data[0];
    let width = fetch_tag!(raw, Tag::ImageWidth).get_usize(0);
    let mut height = fetch_tag!(raw, Tag::ImageLength).get_usize(0);
    let offset = fetch_tag!(raw, Tag::StripOffsets).get_usize(0);
    let count = fetch_tag!(raw, Tag::StripByteCounts).get_usize(0);
    let compression = fetch_tag!(raw, Tag::Compression).get_u32(0);
    let bps = if camera.bps != 0 {
      camera.bps
    } else {
      fetch_tag!(raw, Tag::BitsPerSample).get_usize(0)
    };
    let src = &self.buffer[offset..];

    let image = match compression {
      1 => {
        if camera.model == "DSC-R1" {
          decode_14be_unpacked(src, width, height)
        } else {
          decode_16le(src, width, height)
        }
      }
      32767 => {
        if (width*height*bps) != count*8 {
          height += 8;
          ArwDecoder::decode_arw1(src, width, height)
        } else {
          match bps {
            8 => {let curve = try!(ArwDecoder::get_curve(raw)); ArwDecoder::decode_arw2(src, width, height, &curve)},
            12 => decode_12le(src, width, height),
            _ => return Err(format!("ARW2: Don't know how to decode images with {} bps", bps)),
          }
        }
      },
      _ => return Err(format!("ARW: Don't know how to decode type {}", compression).to_string()),
    };

    ok_image(camera, width, height, try!(self.get_wb()), image)
  }
}

impl<'a> ArwDecoder<'a> {
  fn image_a100(&self, camera: &Camera) -> Result<Image,String> {
    // We've caught the elusive A100 in the wild, a transitional format
    // between the simple sanity of the MRW custom format and the wordly
    // wonderfullness of the Tiff-based ARW format, let's shoot from the hip
    let data = self.tiff.find_ifds_with_tag(Tag::SubIFDs);
    if data.len() == 0 {
      return Err("ARW: Couldn't find the data IFD!".to_string())
    }
    let raw = data[0];
    let width = 3881;
    let height = 2608;
    let offset = fetch_tag!(raw, Tag::SubIFDs).get_usize(0);

    let src = &self.buffer[offset..];
    let image = ArwDecoder::decode_arw1(src, width, height);

    // Get the WB the MRW way
    let priv_offset = fetch_tag!(self.tiff, Tag::DNGPrivateArea).get_force_u32(0) as usize;
    let buf = &self.buffer[priv_offset..];
    let mut currpos: usize = 8;
    let mut wb_coeffs: [f32;4] = [0.0, 0.0, 0.0, NAN];
    // At most we read 20 bytes from currpos so check we don't step outside that
    while currpos+20 < buf.len() {
      let tag: u32 = BEu32(buf,currpos);
      let len: usize = LEu32(buf,currpos+4) as usize;
      if tag == 0x574247 { // WBG
        wb_coeffs[0] = LEu16(buf, currpos+12) as f32;
        wb_coeffs[1] = LEu16(buf, currpos+14) as f32;
        wb_coeffs[2] = LEu16(buf, currpos+18) as f32;
        break;
      }
      currpos += len+8;
    }

    ok_image(camera, width, height, wb_coeffs, image)
  }

  fn image_srf(&self, camera: &Camera) -> Result<Image,String> {
    let data = self.tiff.find_ifds_with_tag(Tag::ImageWidth);
    if data.len() == 0 {
      return Err("ARW: Couldn't find the data IFD!".to_string())
    }
    let raw = data[0];

    let width = fetch_tag!(raw, Tag::ImageWidth).get_usize(0);
    let height = fetch_tag!(raw, Tag::ImageLength).get_usize(0);
    let len = width*height*2;

    // Constants taken from dcraw
    let off: usize = 862144;
    let key_off: usize = 200896;
    let head_off: usize = 164600;

    // Replicate the dcraw contortions to get the "decryption" key
    let offset = (self.buffer[key_off] as usize)*4;
    let first_key = BEu32(self.buffer, key_off+offset);
    let head = ArwDecoder::sony_decrypt(self.buffer, head_off, 40, first_key);
    let second_key = LEu32(&head, 22);

    // "Decrypt" the whole image buffer
    let image_data = ArwDecoder::sony_decrypt(self.buffer, off, len, second_key);
    let image = decode_16be(&image_data, width, height);

    ok_image(camera, width, height, [NAN,NAN,NAN,NAN], image)
  }

  fn decode_arw1(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
    let mut pump = BitPumpMSB::new(buf);
    let mut out: Vec<u16> = vec![0; width*height];

    let mut sum: i32 = 0;
    for x in 0..width {
      let col = width-1-x;
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
          pump.get_bits(cmp::min(13, zeros+1));
        }
        let diff: i32 = pump.get_ibits(len);
        sum += diff;
        if len > 0 && (diff & (1 << (len - 1))) == 0 {
          sum -= (1 << len) - 1;
        }
        out[row*width+col] = sum as u16;
        row += 2
      }
    }
    out
  }

  fn decode_arw2(buf: &[u8], width: usize, height: usize, curve: &LookupTable) -> Vec<u16> {
    decode_threaded(width, height, &(|out: &mut [u16], row| {
      let mut pump = BitPumpLSB::new(&buf[(row*width)..]);

      let mut random = pump.peek_bits(24);
      for out in out.chunks_mut(32) {
        // Process 32 pixels at a time in interleaved fashion
        for j in 0..2 {
          let max = pump.get_bits(11);
          let min = pump.get_bits(11);
          let delta = max-min;
          // Calculate the size of the data shift needed by how large the delta is
          // A delta with 11 bits requires a shift of 4, 10 bits of 3, etc
          let delta_shift: u32 = cmp::max(0, (32-(delta.leading_zeros() as i32)) - 7) as u32;
          let imax = pump.get_bits(4) as usize;
          let imin = pump.get_bits(4) as usize;

          for i in 0..16 {
            let val = if i == imax {
              max
            } else if i == imin {
              min
            } else {
              cmp::min(0x7ff,(pump.get_bits(7) << delta_shift) + min)
            };
            out[j+(i*2)] = curve.dither((val<<1) as u16, &mut random);
          }
        }
      }
    }))
  }

  fn get_wb(&self) -> Result<[f32;4], String> {
    let priv_offset = fetch_tag!(self.tiff, Tag::DNGPrivateArea).get_force_u32(0) as usize;
    let priv_tiff = try!(TiffIFD::new(self.buffer, priv_offset, 0, 0, 0, LITTLE_ENDIAN));
    let sony_offset = fetch_tag!(priv_tiff, Tag::SonyOffset).get_usize(0);
    let sony_length = fetch_tag!(priv_tiff, Tag::SonyLength).get_usize(0);
    let sony_key = fetch_tag!(priv_tiff, Tag::SonyKey).get_u32(0);
    let decrypted_buf = ArwDecoder::sony_decrypt(self.buffer, sony_offset, sony_length, sony_key);
    let decrypted_tiff = TiffIFD::new(&decrypted_buf, 0, sony_offset, 0, 0, LITTLE_ENDIAN).unwrap();
    let grgb_levels = decrypted_tiff.find_entry(Tag::SonyGRBG);
    let rggb_levels = decrypted_tiff.find_entry(Tag::SonyRGGB);
    if grgb_levels.is_some() {
      let levels = grgb_levels.unwrap();
      Ok([levels.get_u32(1) as f32, levels.get_u32(0) as f32, levels.get_u32(2) as f32, NAN])
    } else if rggb_levels.is_some() {
      let levels = rggb_levels.unwrap();
      Ok([levels.get_u32(0) as f32, levels.get_u32(1) as f32, levels.get_u32(3) as f32, NAN])
    } else {
      Err("ARW: Couldn't find GRGB or RGGB levels".to_string())
    }
  }

  fn get_curve(raw: &TiffIFD) -> Result<LookupTable, String> {
    let centry = fetch_tag!(raw, Tag::SonyCurve);
    let mut curve: [usize;6] = [ 0, 0, 0, 0, 0, 4095 ];

    for i in 0..4 {
      curve[i+1] = ((centry.get_u32(i) >> 2) & 0xfff) as usize;
    }

    let mut out = vec![0 as u16; curve[5]+1];
    for i in 0..5 {
      for j in (curve[i]+1)..(curve[i+1]+1) {
        out[j] = out[(j-1)] + (1<<i);
      }
    }

    Ok(LookupTable::new(&out))
  }

  fn sony_decrypt(buf: &[u8], offset: usize, length: usize, key: u32) -> Vec<u8>{
    let mut pad: [u32; 128] = [0 as u32; 128];
    let mut mkey = key;
    // Initialize the decryption pad from the key
    for p in 0..4 {
      mkey = mkey.wrapping_mul(48828125).wrapping_add(1);
      pad[p] = mkey;
    }
    pad[3] = pad[3] << 1 | (pad[0]^pad[2]) >> 31;
    for p in 4..127 {
      pad[p] = (pad[p-4]^pad[p-2]) << 1 | (pad[p-3]^pad[p-1]) >> 31;
    }
    for p in 0..127 {
      pad[p] = u32::from_be(pad[p]);
    }

    let mut out = Vec::with_capacity(length+4);
    for i in 0..(length/4+1) {
      let p = i + 127;
      pad[p & 127] = pad[(p+1) & 127] ^ pad[(p+1+64) & 127];
      let output = LEu32(buf, offset+i*4) ^ pad[p & 127];
      let bytes: [u8;4] = unsafe { transmute(output.to_le()) };
      out.push(bytes[0]);
      out.push(bytes[1]);
      out.push(bytes[2]);
      out.push(bytes[3]);
    }
    out
  }
}
