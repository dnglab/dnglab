use std::f32::NAN;

use crate::decoders::*;
use crate::decoders::tiff::*;
use crate::decoders::basics::*;
use crate::decompressors::ljpeg::huffman::*;

// NEF Huffman tables in order. First two are the normal huffman definitions.
// Third one are weird shifts that are used in the lossy split encodings only
// Values are extracted from dcraw with the shifts unmangled out.
const NIKON_TREE: [[[u8;16];3];6] = [
  [ // 12-bit lossy
    [0,0,1,5,1,1,1,1,1,1,2,0,0,0,0,0],
    [5,4,3,6,2,7,1,0,8,9,11,10,12,0,0,0],
    [0,0,0,0,0,0,0,0,0,0, 0, 0, 0,0,0,0],
  ],
  [ // 12-bit lossy after split
    [0,0,1,5,1,1,1,1,1,1,2,0,0,0,0,0],
    [6,5,5,5,5,5,4,3,2,1,0,11,12,12,0,0],
    [3,5,3,2,1,0,0,0,0,0, 0, 0, 0,0,0,0],
  ],
  [ // 12-bit lossless
    [0,0,1,4,2,3,1,2,0,0,0,0,0,0,0,0],
    [5,4,6,3,7,2,8,1,9,0,10,11,12,0,0,0],
    [0,0,0,0,0,0,0,0,0,0, 0, 0, 0,0,0,0],
  ],
  [ // 14-bit lossy
    [0,0,1,4,3,1,1,1,1,1,2,0,0,0,0,0],
    [5,6,4,7,8,3,9,2,1,0,10,11,12,13,14,0],
    [0,0,0,0,0,0,0,0,0,0, 0, 0, 0, 0, 0,0],
  ],
  [ // 14-bit lossy after split
    [0,0,1,5,1,1,1,1,1,1,1,2,0,0,0,0],
    [8,7,7,7,7,7,6,5,4,3,2,1,0,13,14,0],
    [0,5,4,3,2,0,0,0,0,0,0,0,0, 0, 0,0],
  ],
  [ // 14-bit lossless
    [0,0,1,4,2,2,3,1,2,0,0,0,0,0,0,0],
    [7,6,8,5,9,4,10,3,11,12,2,0,1,13,14,0],
    [0,0,0,0,0,0, 0,0, 0, 0,0,0,0, 0, 0,0],
  ],
];

// We use this for the D50 and D2X whacky WB "encryption"
const WB_SERIALMAP: [u8;256] = [
  0xc1,0xbf,0x6d,0x0d,0x59,0xc5,0x13,0x9d,0x83,0x61,0x6b,0x4f,0xc7,0x7f,0x3d,0x3d,
  0x53,0x59,0xe3,0xc7,0xe9,0x2f,0x95,0xa7,0x95,0x1f,0xdf,0x7f,0x2b,0x29,0xc7,0x0d,
  0xdf,0x07,0xef,0x71,0x89,0x3d,0x13,0x3d,0x3b,0x13,0xfb,0x0d,0x89,0xc1,0x65,0x1f,
  0xb3,0x0d,0x6b,0x29,0xe3,0xfb,0xef,0xa3,0x6b,0x47,0x7f,0x95,0x35,0xa7,0x47,0x4f,
  0xc7,0xf1,0x59,0x95,0x35,0x11,0x29,0x61,0xf1,0x3d,0xb3,0x2b,0x0d,0x43,0x89,0xc1,
  0x9d,0x9d,0x89,0x65,0xf1,0xe9,0xdf,0xbf,0x3d,0x7f,0x53,0x97,0xe5,0xe9,0x95,0x17,
  0x1d,0x3d,0x8b,0xfb,0xc7,0xe3,0x67,0xa7,0x07,0xf1,0x71,0xa7,0x53,0xb5,0x29,0x89,
  0xe5,0x2b,0xa7,0x17,0x29,0xe9,0x4f,0xc5,0x65,0x6d,0x6b,0xef,0x0d,0x89,0x49,0x2f,
  0xb3,0x43,0x53,0x65,0x1d,0x49,0xa3,0x13,0x89,0x59,0xef,0x6b,0xef,0x65,0x1d,0x0b,
  0x59,0x13,0xe3,0x4f,0x9d,0xb3,0x29,0x43,0x2b,0x07,0x1d,0x95,0x59,0x59,0x47,0xfb,
  0xe5,0xe9,0x61,0x47,0x2f,0x35,0x7f,0x17,0x7f,0xef,0x7f,0x95,0x95,0x71,0xd3,0xa3,
  0x0b,0x71,0xa3,0xad,0x0b,0x3b,0xb5,0xfb,0xa3,0xbf,0x4f,0x83,0x1d,0xad,0xe9,0x2f,
  0x71,0x65,0xa3,0xe5,0x07,0x35,0x3d,0x0d,0xb5,0xe9,0xe5,0x47,0x3b,0x9d,0xef,0x35,
  0xa3,0xbf,0xb3,0xdf,0x53,0xd3,0x97,0x53,0x49,0x71,0x07,0x35,0x61,0x71,0x2f,0x43,
  0x2f,0x11,0xdf,0x17,0x97,0xfb,0x95,0x3b,0x7f,0x6b,0xd3,0x25,0xbf,0xad,0xc7,0xc5,
  0xc5,0xb5,0x8b,0xef,0x2f,0xd3,0x07,0x6b,0x25,0x49,0x95,0x25,0x49,0x6d,0x71,0xc7
];

const WB_KEYMAP: [u8;256] = [
  0xa7,0xbc,0xc9,0xad,0x91,0xdf,0x85,0xe5,0xd4,0x78,0xd5,0x17,0x46,0x7c,0x29,0x4c,
  0x4d,0x03,0xe9,0x25,0x68,0x11,0x86,0xb3,0xbd,0xf7,0x6f,0x61,0x22,0xa2,0x26,0x34,
  0x2a,0xbe,0x1e,0x46,0x14,0x68,0x9d,0x44,0x18,0xc2,0x40,0xf4,0x7e,0x5f,0x1b,0xad,
  0x0b,0x94,0xb6,0x67,0xb4,0x0b,0xe1,0xea,0x95,0x9c,0x66,0xdc,0xe7,0x5d,0x6c,0x05,
  0xda,0xd5,0xdf,0x7a,0xef,0xf6,0xdb,0x1f,0x82,0x4c,0xc0,0x68,0x47,0xa1,0xbd,0xee,
  0x39,0x50,0x56,0x4a,0xdd,0xdf,0xa5,0xf8,0xc6,0xda,0xca,0x90,0xca,0x01,0x42,0x9d,
  0x8b,0x0c,0x73,0x43,0x75,0x05,0x94,0xde,0x24,0xb3,0x80,0x34,0xe5,0x2c,0xdc,0x9b,
  0x3f,0xca,0x33,0x45,0xd0,0xdb,0x5f,0xf5,0x52,0xc3,0x21,0xda,0xe2,0x22,0x72,0x6b,
  0x3e,0xd0,0x5b,0xa8,0x87,0x8c,0x06,0x5d,0x0f,0xdd,0x09,0x19,0x93,0xd0,0xb9,0xfc,
  0x8b,0x0f,0x84,0x60,0x33,0x1c,0x9b,0x45,0xf1,0xf0,0xa3,0x94,0x3a,0x12,0x77,0x33,
  0x4d,0x44,0x78,0x28,0x3c,0x9e,0xfd,0x65,0x57,0x16,0x94,0x6b,0xfb,0x59,0xd0,0xc8,
  0x22,0x36,0xdb,0xd2,0x63,0x98,0x43,0xa1,0x04,0x87,0x86,0xf7,0xa6,0x26,0xbb,0xd6,
  0x59,0x4d,0xbf,0x6a,0x2e,0xaa,0x2b,0xef,0xe6,0x78,0xb6,0x4e,0xe0,0x2f,0xdc,0x7c,
  0xbe,0x57,0x19,0x32,0x7e,0x2a,0xd0,0xb8,0xba,0x29,0x00,0x3c,0x52,0x7d,0xa8,0x49,
  0x3b,0x2d,0xeb,0x25,0x49,0xfa,0xa3,0xaa,0x39,0xa7,0xc5,0xa7,0x50,0x11,0x36,0xfb,
  0xc6,0x67,0x4a,0xf5,0xa5,0x12,0x65,0x7e,0xb0,0xdf,0xaf,0x4e,0xb3,0x61,0x7f,0x2f
];

#[derive(Debug, Clone)]
pub struct NefDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: TiffIFD<'a>,
}

impl<'a> NefDecoder<'a> {
  pub fn new(buf: &'a [u8], tiff: TiffIFD<'a>, rawloader: &'a RawLoader) -> NefDecoder<'a> {
    NefDecoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for NefDecoder<'a> {
  fn image(&self, dummy: bool) -> Result<RawImage,String> {
    let raw = fetch_ifd!(&self.tiff, Tag::CFAPattern);
    let mut width = fetch_tag!(raw, Tag::ImageWidth).get_usize(0);
    let height = fetch_tag!(raw, Tag::ImageLength).get_usize(0);
    let bps = fetch_tag!(raw, Tag::BitsPerSample).get_usize(0);
    let compression = fetch_tag!(raw, Tag::Compression).get_usize(0);

    // Make sure we always use a 12/14 bit mode to get correct white/blackpoints
    let mode = format!("{}bit", bps).to_string();
    let camera = self.rawloader.check_supported_with_mode(&self.tiff, &mode)?;

    let offset = fetch_tag!(raw, Tag::StripOffsets).get_usize(0);
    let size = fetch_tag!(raw, Tag::StripByteCounts).get_usize(0);
    let src = &self.buffer[offset..];
    let mut cpp = 1;
    let coeffs = self.get_wb()?;

    let image = if camera.model == "NIKON D100" {
      width = 3040;
      decode_12be_wcontrol(src, width, height, dummy)
    } else {
      if compression == 1 || size == width*height*bps/8 {
        match bps {
          14 => if self.tiff.little_endian() {
            decode_14le_unpacked(src, width, height, dummy)
          } else {
            decode_14be_unpacked(src, width, height, dummy)
          },
          12 => if self.tiff.little_endian() {
            decode_12le(src, width, height, dummy)
          } else {
            decode_12be(src, width, height, dummy)
          },
          x => return Err(format!("Don't know uncompressed bps {}", x).to_string()),
        }
      } else if size == width*height*3 {
        cpp = 3;
        Self::decode_snef_compressed(src, coeffs, width, height, dummy)
      } else if compression == 34713 {
        self.decode_compressed(src, width, height, bps, dummy)?
      } else {
        return Err(format!("NEF: Don't know compression {}", compression).to_string())
      }
    };

    let mut img = RawImage::new(camera, width, height, coeffs, image, false);
    if cpp == 3 {
      img.cpp = 3;
      img.blacklevels = [0,0,0,0];
      img.whitelevels = [65535,65535,65535,65535];
    }
    Ok(img)
  }
}

impl<'a> NefDecoder<'a> {
  fn get_wb(&self) -> Result<[f32;4], String> {
    if let Some(levels) = self.tiff.find_entry(Tag::NefWB0) {
      Ok([levels.get_f32(0), 1.0, levels.get_f32(1), NAN])
    } else if let Some(levels) = self.tiff.find_entry(Tag::NefWB1) {
      let mut version: u32 = 0;
      for i in 0..4 {
        version = (version << 4) + (levels.get_data()[i]-b'0') as u32;
      }
      match version {
        0x100 =>  Ok([levels.get_force_u16(36) as f32, levels.get_force_u16(38) as f32,
                      levels.get_force_u16(37) as f32, NAN]),
        0x103 =>  Ok([levels.get_force_u16(10) as f32, levels.get_force_u16(11) as f32,
                      levels.get_force_u16(12) as f32, NAN]),
        0x204 | 0x205 => {
          let serial = fetch_tag!(self.tiff, Tag::NefSerial);
          let data = serial.get_data();
          let mut serialno = 0 as usize;
          for i in 0..serial.count() {
            if data[i] == 0 { break }
            serialno = serialno*10 + if data[i] >= 48 && data[i] <= 57 { // "0" to "9"
              (data[i]-48) as usize
            } else {
              (data[i]%10) as usize
            };
          }

          // Get the "decryption" key
          let keydata = fetch_tag!(self.tiff, Tag::NefKey).get_data();
          let keyno = (keydata[0]^keydata[1]^keydata[2]^keydata[3]) as usize;

          let src = if version == 0x204 {
            &levels.get_data()[284..]
          } else {
            &levels.get_data()[4..]
          };

          let ci = WB_SERIALMAP[serialno & 0xff] as u32;
          let mut cj = WB_KEYMAP[keyno & 0xff] as u32;
          let mut ck = 0x60 as u32;
          let mut buf = [0 as u8; 280];
          for i in 0..280 {
            cj += ci * ck;
            ck += 1;
            buf[i] = src[i] ^ (cj as u8);
          }

          let off = if version == 0x204 { 6 } else { 14 };
          Ok([BEu16(&buf, off) as f32, BEu16(&buf, off+2) as f32,
              BEu16(&buf, off+6) as f32, NAN])
        },
        x => Err(format!("NEF: Don't know about WB version 0x{:x}", x).to_string()),
      }
    } else {
      Err("NEF: Don't know how to fetch WB".to_string())
    }
  }

  fn create_hufftable(num: usize) -> Result<HuffTable,String> {
    let mut htable = HuffTable::empty();

    for i in 0..15 {
      htable.bits[i] = NIKON_TREE[num][0][i] as u32;
      htable.huffval[i] = NIKON_TREE[num][1][i] as u32;
      htable.shiftval[i] = NIKON_TREE[num][2][i] as u32;
    }

    htable.initialize()?;
    Ok(htable)
  }

  fn decode_compressed(&self, src: &[u8], width: usize, height: usize, bps: usize, dummy: bool) -> Result<Vec<u16>, String> {
    let metaifd = fetch_ifd!(self.tiff, Tag::NefMeta1);
    let meta = if let Some(meta) = metaifd.find_entry(Tag::NefMeta2) {meta} else {
      fetch_tag!(metaifd, Tag::NefMeta1)
    };
    Self::do_decode(src, meta.get_data(), metaifd.get_endian(), width, height, bps, dummy)
  }

  pub(crate) fn do_decode(src: &[u8], meta: &[u8], endian: Endian, width: usize, height: usize, bps: usize, dummy: bool) -> Result<Vec<u16>, String> {
    let mut out = alloc_image_ok!(width, height, dummy);
    let mut stream = ByteStream::new(meta, endian);
    let v0 = stream.get_u8();
    let v1 = stream.get_u8();
    //println!("Nef version v0:{}, v1:{}", v0, v1);

    let mut huff_select = 0;
    if v0 == 73 || v1 == 88 {
      stream.consume_bytes(2110);
    }
    if v0 == 70 {
      huff_select = 2;
    }
    if bps == 14 {
      huff_select += 3;
    }

    // Create the huffman table used to decode
    let mut htable = Self::create_hufftable(huff_select)?;

    // Setup the predictors
    let mut pred_up1: [i32;2] = [stream.get_u16() as i32, stream.get_u16() as i32];
    let mut pred_up2: [i32;2] = [stream.get_u16() as i32, stream.get_u16() as i32];

    // Get the linearization curve
    let mut points = [0 as u16; 1<<16];
    for i in 0..points.len() {
      points[i] = i as u16;
    }
    let mut max = 1 << bps;
    let csize = stream.get_u16() as usize;
    let mut split = 0 as usize;
    let step = if csize > 1 {
      max / (csize - 1)
    } else {
      0
    };
    if v0 == 68 && v1 == 32 && step > 0 {
      for i in 0..csize {
        points[i*step] = stream.get_u16();
      }
      for i in 0..max {
        points[i] = ((points[i-i%step] as usize * (step - i % step) +
                     points[i-i%step+step] as usize * (i%step)) / step) as u16;
      }
      split = endian.ru16(meta, 562) as usize;
    } else if v0 != 70 && csize <= 0x4001 {
      for i in 0..csize {
        points[i] = stream.get_u16();
      }
      max = csize;
    }
    let curve = LookupTable::new(&points[0..max]);

    let mut pump = BitPumpMSB::new(src);
    let mut random = pump.peek_bits(24);

    let bps: u32 = bps as u32;
    for row in 0..height {
      if split > 0 && row == split {
        htable = Self::create_hufftable(huff_select+1)?;
      }
      pred_up1[row&1] += htable.huff_decode(&mut pump)?;
      pred_up2[row&1] += htable.huff_decode(&mut pump)?;
      let mut pred_left1 = pred_up1[row&1];
      let mut pred_left2 = pred_up2[row&1];
      for col in (0..width).step_by(2) {
        if col > 0 {
          pred_left1 += htable.huff_decode(&mut pump)?;
          pred_left2 += htable.huff_decode(&mut pump)?;
        }
        out[row*width+col+0] = curve.dither(clampbits(pred_left1,bps), &mut random);
        out[row*width+col+1] = curve.dither(clampbits(pred_left2,bps), &mut random);
      }
    }

    Ok(out)
  }

  // Decodes 12 bit data in an YUY2-like pattern (2 Luma, 1 Chroma per 2 pixels).
  // We un-apply the whitebalance, so output matches lossless.
  pub(crate) fn decode_snef_compressed(src: &[u8], coeffs: [f32; 4], width: usize, height: usize, dummy: bool) -> Vec<u16> {
    let inv_wb_r = (1024.0 / coeffs[0]) as i32;
    let inv_wb_b = (1024.0 / coeffs[2]) as i32;

    //println!("Got invwb {} {}", inv_wb_r, inv_wb_b);

    let snef_curve = {
      let g: f32 = 2.4;
      let f: f32 = 0.055;
      let min: f32 = 0.04045;
      let mul: f32 = 12.92;
      let curve = (0..4096).map(|i| {
        let v = (i as f32) / 4095.0;
        let res = if v <= min {
          v / mul
        } else {
          ((v+f)/(1.0+f)).powf(g)
        };
        clampbits((res*65535.0*4.0) as i32, 16)
      }).collect::<Vec<u16>>();
      LookupTable::new(&curve)
    };

    decode_threaded(width*3, height, dummy, &(|out: &mut [u16], row| {
      let inb = &src[row*width*3..];
      let mut random = BEu32(inb, 0);
      for (o, i) in out.chunks_exact_mut(6).zip(inb.chunks_exact(6)) {
        let g1: u16 = i[0] as u16;
        let g2: u16 = i[1] as u16;
        let g3: u16 = i[2] as u16;
        let g4: u16 = i[3] as u16;
        let g5: u16 = i[4] as u16;
        let g6: u16 = i[5] as u16;

        let y1  = (g1 | ((g2 & 0x0f) << 8)) as f32;
        let y2  = ((g2 >> 4) | (g3 << 4)) as f32;
        let cb = (g4 | ((g5 & 0x0f) << 8)) as f32 - 2048.0;
        let cr = ((g5 >> 4) | (g6 << 4)) as f32 - 2048.0;

        let r = snef_curve.dither(clampbits((y1 + 1.370705 * cr) as i32, 12), &mut random);
        let g = snef_curve.dither(clampbits((y1 - 0.337633 * cb - 0.698001 * cr) as i32, 12), &mut random);
        let b = snef_curve.dither(clampbits((y1 + 1.732446 * cb) as i32, 12), &mut random);
        // invert the white balance
        o[0] = clampbits((inv_wb_r * r as i32 + (1<<9)) >> 10, 15);
        o[1] = g;
        o[2] = clampbits((inv_wb_b * b as i32 + (1<<9)) >> 10, 15);

        let r = snef_curve.dither(clampbits((y2 + 1.370705 * cr) as i32, 12), &mut random);
        let g = snef_curve.dither(clampbits((y2 - 0.337633 * cb - 0.698001 * cr) as i32, 12), &mut random);
        let b = snef_curve.dither(clampbits((y2 + 1.732446 * cb) as i32, 12), &mut random);
        // invert the white balance
        o[3] = clampbits((inv_wb_r * r as i32 + (1<<9)) >> 10, 15);
        o[4] = g;
        o[5] = clampbits((inv_wb_b * b as i32 + (1<<9)) >> 10, 15);
      }
    }))
  }
}
