use decoders::*;
use decoders::tiff::*;
use decoders::basics::*;
use std::mem::transmute;
extern crate itertools;
use self::itertools::Itertools;
extern crate crossbeam;
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
      return Err("ARW: Couldn't find the data IFD!".to_string())
    }
    let raw = data[0];
    let width = fetch_tag!(raw, Tag::ImageWidth, "ARW: Couldn't find width").get_u16(0) as u32;
    let height = fetch_tag!(raw, Tag::ImageLength, "ARW: Couldn't find height").get_u16(0) as u32;
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
          ArwDecoder::decode_arw1(src, width, height)
        } else {
          match bps {
            8 => {let curve = try!(ArwDecoder::get_curve(raw)); ArwDecoder::decode_arw2(src, width, height, &curve)},
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
  fn decode_arw1(buf: &[u8], width: u32, height: u32) -> Vec<u16> {
    let mut buffer: Vec<u16> = vec![0; (width*height) as usize];

    buffer[0] = buf[0] as u16; // Shut up the warnings for now

    buffer
  }

  fn decode_arw2(buf: &[u8], width: u32, height: u32, curve: &LookupTable) -> Vec<u16> {
    // Speedup doesn't currently seem worthwhile above 2 threads, probably because
    // of too much copying
    let threads = 2;
    if threads < 2 || height < threads {
      return ArwDecoder::decode_arw2_slice(buf, width, height, &curve);
    }

    let mut heights = Vec::new();
    let split = height/threads;
    let mut height_split = 0;
    for i in 0..threads {
      let start = height_split;
      height_split += split;
      let end = if i == threads-1 {
        height
      } else {
        height_split -1
      };
      heights.push((start, end));
    }

    crossbeam::scope(|scope| {
      let mut handles = Vec::new();
      for (start, end) in heights {
        let src = &buf[((start*width) as usize)..buf.len()];
        let handle = scope.spawn(move || {
          ArwDecoder::decode_arw2_slice(src, width, end-start+1, &curve)
        });
        handles.push(handle);
      }

      let mut out = Vec::new();
      for h in handles {
        let mut other = h.join();
        out.append(&mut other);
      }
      out
    })
  }

  fn decode_arw2_slice(buf: &[u8], width: u32, height: u32, curve: &LookupTable) -> Vec<u16> {
    let mut buffer: Vec<u16> = vec![0; (width*height) as usize];
    let mut pump = BitPump::new(buf);

    for row in 0..height {
      // Process 16 pixels at a time in interleaved fashion
      let mut col = 0;
      let mut random = pump.peek_bits(24);
      while col < (width-30) {
        let max = pump.get_bits(11);
        let min = pump.get_bits(11);
        let imax = pump.get_bits(4);
        let imin = pump.get_bits(4);
        let mut sh = 0;
        while sh<4 && (0x80 << sh) <= (max - min) {sh = sh + 1;}
        for i in 0..16 {
          let val = if i == imax {
            max
          } else if i == imin {
            min
          } else {
            cmp::min(0x7ff,(pump.get_bits(7) << sh) + min)
          };
          buffer[(row*width+col+i*2) as usize] = curve.dither(val as u16, &mut random);
        }
        col += if (col & 1) != 0 {31} else {1};  // Skip to next 16 pixels
      }
    }

    buffer
  }

  fn get_wb(&self) -> Result<[f32;4], String> {
    let priv_offset = fetch_tag!(self.tiff, Tag::DNGPrivateArea, "ARW: Couldn't find private offset").get_u32(0);
    let priv_tiff = TiffIFD::new(self.buffer, priv_offset as usize, 0, 0, LITTLE_ENDIAN);
    let sony_offset = fetch_tag!(priv_tiff, Tag::SonyOffset, "ARW: Couldn't find sony offset").get_u32(0) as usize;
    let sony_length = fetch_tag!(priv_tiff, Tag::SonyLength, "ARW: Couldn't find sony length").get_u32(0) as usize;
    let sony_key = fetch_tag!(priv_tiff, Tag::SonyKey, "ARW: Couldn't find sony key").get_u32(0);
    let decrypted_buf = ArwDecoder::sony_decrypt(self.buffer, sony_offset, sony_length, sony_key);
    let decrypted_tiff = TiffIFD::new(&decrypted_buf, 0, sony_offset, 0, LITTLE_ENDIAN);
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
