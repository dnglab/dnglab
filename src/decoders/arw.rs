use decoders::*;
use decoders::tiff::*;
use decoders::basics::*;
use std::mem::transmute;
extern crate itertools;
use self::itertools::Itertools;
use std::f32;
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
  fn identify(&self) -> Result<&Camera, String> {
    let make = fetch_tag!(self.tiff, Tag::Make, "ARW: Couldn't find Make").get_str();
    let model = fetch_tag!(self.tiff, Tag::Model, "ARW: Couldn't find Model").get_str();
    self.rawloader.check_supported(make, model)
  }

  fn image(&self) -> Result<Image,String> {
    let camera = try!(self.identify());
    let data = self.tiff.find_ifds_with_tag(Tag::StripOffsets);
    if data.len() == 0 {
      if camera.model == "DSLR-A100" {
        return self.image_a100(camera)
      } else { // try decoding as SRF
        return self.image_srf(camera)
      }
    }
    let raw = data[0];
    let width = fetch_tag!(raw, Tag::ImageWidth, "ARW: Couldn't find width").get_u16(0) as u32;
    let mut height = fetch_tag!(raw, Tag::ImageLength, "ARW: Couldn't find height").get_u16(0) as u32;
    let offset = fetch_tag!(raw, Tag::StripOffsets, "ARW: Couldn't find offset").get_u32(0) as usize;
    let count = fetch_tag!(raw, Tag::StripByteCounts, "ARW: Couldn't find byte count").get_u32(0) as usize;
    let compression = fetch_tag!(raw, Tag::Compression, "ARW: Couldn't find Compression").get_u16(0);
    let bps = if camera.bps != 0 {
      camera.bps
    } else {
      fetch_tag!(raw, Tag::BitsPerSample, "ARW: Couldn't find bps").get_u16(0) as u32
    };
    let src = &self.buffer[offset .. self.buffer.len()];

    let image = match compression {
      1 => decode_16le(src, width as usize, height as usize),
      32767 => {
        if ((width*height*bps) as usize) != count*8 {
          height += 8;
          ArwDecoder::decode_arw1(src, width as usize, height as usize)
        } else {
          match bps {
            8 => {let curve = try!(ArwDecoder::get_curve(raw)); ArwDecoder::decode_arw2(src, width as usize, height as usize, &curve)},
            12 => decode_12le(src, width as usize, height as usize),
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
    let offset = fetch_tag!(raw, Tag::SubIFDs, "ARW: Couldn't find offset").get_u32(0) as usize;

    let src = &self.buffer[offset .. self.buffer.len()];
    let image = ArwDecoder::decode_arw1(src, width as usize, height as usize);

    // Get the WB the MRW way
    let priv_offset = fetch_tag!(self.tiff, Tag::DNGPrivateArea, "ARW: Couldn't find private offset").get_u32(0);
    let buf = &self.buffer[priv_offset as usize..];
    let mut currpos: usize = 8;
    let mut wb_vals: [u16;4] = [0;4];
    // At most we read 20 bytes from currpos so check we don't step outside that
    while currpos+20 < buf.len() {
      let tag: u32 = BEu32(buf,currpos);
      let len: u32 = LEu32(buf,currpos+4);
      if tag == 0x574247 { // WBG
        for i in 0..4 {
          wb_vals[i] = LEu16(buf, currpos+12+i*2);
        }
        break;
      }
      currpos += (len+8) as usize;
    }
    let wb_coeffs = [wb_vals[0] as f32, wb_vals[1] as f32, wb_vals[3] as f32, f32::NAN];

    ok_image(camera, width, height, wb_coeffs, image)
  }

  fn image_srf(&self, camera: &Camera) -> Result<Image,String> {
    let data = self.tiff.find_ifds_with_tag(Tag::ImageWidth);
    if data.len() == 0 {
      return Err("ARW: Couldn't find the data IFD!".to_string())
    }
    let raw = data[0];

    let width = fetch_tag!(raw, Tag::ImageWidth, "SRF: Couldn't find width").get_u16(0) as u32;
    let height = fetch_tag!(raw, Tag::ImageLength, "SRF: Couldn't find height").get_u16(0) as u32;
    let len = (width*height*2) as usize;

    // Constants taken from dcraw
    let off: usize = 862144;
    let key_off: usize = 200896;
    let head_off: usize = 164600;

    // Replicate the dcraw contortions to get the "decryption" key
    let offset = (self.buffer[key_off as usize] as usize)*4;
    let first_key = BEu32(self.buffer, key_off+offset);
    let head = ArwDecoder::sony_decrypt(self.buffer, head_off, 40, first_key);
    let second_key = LEu32(&head, 22);

    // "Decrypt" the whole image buffer
    let image_data = ArwDecoder::sony_decrypt(self.buffer, off, len, second_key);
    let image = decode_16be(&image_data, width as usize, height as usize);

    ok_image(camera, width, height, [f32::NAN,f32::NAN,f32::NAN,f32::NAN], image)
  }

  fn decode_arw1(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
    let mut pump = BitPump::new(buf, PumpOrder::MSB);
    let mut out: Vec<u16> = vec![0; (width*height) as usize];

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
    decode_threaded(width, height, &(|out: &mut [u16], start, width, height| {
      let mut pump = BitPump::new(&buf[(start*width)..], PumpOrder::LSB);

      for row in 0..height {
        // Process 16 pixels at a time in interleaved fashion
        let mut col = 0;
        let mut random = pump.peek_bits(24);
        while col < (width-30) {
          let max = pump.get_bits(11);
          let min = pump.get_bits(11);
          let delta = max-min;
          // Calculate the size of the data shift needed by how large the delta is
          // A delta with 11 bits requires a shift of 4, 10 bits of 3, etc
          let delta_shift: u32 = cmp::max(0, (32-(delta.leading_zeros() as i32)) - 7) as u32;
          let imax = pump.get_bits(4);
          let imin = pump.get_bits(4);

          for i in 0..16 {
            let val = if i == imax {
              max
            } else if i == imin {
              min
            } else {
              cmp::min(0x7ff,(pump.get_bits(7) << delta_shift) + min)
            };
            out[row*width+col+(i*2) as usize] = curve.dither(val as u16, &mut random);
          }
          col += if (col & 1) != 0 {31} else {1};  // Skip to next 16 pixels
        }
      }
    }))
  }

  fn get_wb(&self) -> Result<[f32;4], String> {
    let priv_offset = fetch_tag!(self.tiff, Tag::DNGPrivateArea, "ARW: Couldn't find private offset").get_u32(0);
    let priv_tiff = TiffIFD::new(self.buffer, priv_offset as usize, 0, 0, LITTLE_ENDIAN).unwrap();
    let sony_offset = fetch_tag!(priv_tiff, Tag::SonyOffset, "ARW: Couldn't find sony offset").get_u32(0) as usize;
    let sony_length = fetch_tag!(priv_tiff, Tag::SonyLength, "ARW: Couldn't find sony length").get_u32(0) as usize;
    let sony_key = fetch_tag!(priv_tiff, Tag::SonyKey, "ARW: Couldn't find sony key").get_u32(0);
    let decrypted_buf = ArwDecoder::sony_decrypt(self.buffer, sony_offset, sony_length, sony_key);
    let decrypted_tiff = TiffIFD::new(&decrypted_buf, 0, sony_offset, 0, LITTLE_ENDIAN).unwrap();
    let grgb_levels = decrypted_tiff.find_entry(Tag::SonyGRBG);
    let rggb_levels = decrypted_tiff.find_entry(Tag::SonyRGGB);
    if grgb_levels.is_some() {
      let levels = grgb_levels.unwrap();
      Ok([levels.get_u16(1) as f32, levels.get_u16(0) as f32, levels.get_u16(2) as f32, f32::NAN])
    } else if rggb_levels.is_some() {
      let levels = rggb_levels.unwrap();
      Ok([levels.get_u16(0) as f32, levels.get_u16(1) as f32, levels.get_u16(3) as f32, f32::NAN])
    } else {
      Err("ARW: Couldn't find GRGB or RGGB levels".to_string())
    }
  }

  fn get_curve(raw: &TiffIFD) -> Result<LookupTable, String> {
    let centry = fetch_tag!(raw, Tag::SonyCurve, "ARW: Couldn't find sony curve");
    let mut curve: [u32;6] = [ 0, 0, 0, 0, 0, 4095 ];

    for i in 0..4 {
      curve[i+1] = ((centry.get_u16(i) >> 2) & 0xfff) as u32;
    }

    let mut out = vec![0 as u16; (curve[5]+1) as usize];
    for i in 0..5 {
      for j in (curve[i]+1)..(curve[i+1]+1) {
        out[j as usize] = out[(j-1) as usize] + (1<<i);
      }
    }

    Ok(LookupTable::new(&out))
  }

  fn sony_decrypt(buf: &[u8], offset: usize, length: usize, key: u32) -> Vec<u8>{
    let mut pad: [u32; 128] = [0 as u32; 128];
    let mut mkey = key;
    // Make sure we always have space for the final bytes even if the buffer
    // isn't a multiple of 4
    let mut out = vec![0;length+4];
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

    for i in (0..length).step(4) {
      let p = i/4 + 127;
      pad[p & 127] = pad[(p+1) & 127] ^ pad[(p+1+64) & 127];
      let output = LEu32(buf, offset+i) ^ pad[p & 127];
      let bytes: [u8; 4] = unsafe { transmute(output.to_le()) };
      out[i]   = bytes[0];
      out[i+1] = bytes[1];
      out[i+2] = bytes[2];
      out[i+3] = bytes[3];
    }
    out
  }
}
