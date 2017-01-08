use decoders::*;
use decoders::tiff::*;
use decoders::basics::*;
use decoders::ljpeg::huffman::*;
use itertools::Itertools;
use std::f32::NAN;

const NIKON_TREE: [[u8;32];6] = [
  // 12-bit lossy
  [0,1,5,1,1,1,1,1,1,2,0,0,0,0,0,0,5,4,3,6,2,7,1,0,8,9,11,10,12,0,0,0],
  // 12-bit lossy after split
  [0,1,5,1,1,1,1,1,1,2,0,0,0,0,0,0,0x39,0x5a,0x38,0x27,0x16,5,4,3,2,1,0,11,12,12,0,0],
  // 12-bit lossless
  [0,1,4,2,3,1,2,0,0,0,0,0,0,0,0,0,5,4,6,3,7,2,8,1,9,0,10,11,12,0,0,0],
  // 14-bit lossy
  [0,1,4,3,1,1,1,1,1,2,0,0,0,0,0,0,5,6,4,7,8,3,9,2,1,0,10,11,12,13,14,0],
  // 14-bit lossy after split
  [0,1,5,1,1,1,1,1,1,1,2,0,0,0,0,0,8,0x5c,0x4b,0x3a,0x29,7,6,5,4,3,2,1,0,13,14,0],
  // 14-bit lossless
  [0,1,4,2,2,3,1,2,0,0,0,0,0,0,0,0,7,6,8,5,9,4,10,3,11,12,2,0,1,13,14,0],
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
  fn image(&self) -> Result<Image,String> {
    let camera = try!(self.rawloader.check_supported(&self.tiff));
    let raw = fetch_ifd!(&self.tiff, Tag::CFAPattern);
    let mut width = fetch_tag!(raw, Tag::ImageWidth).get_usize(0);
    let height = fetch_tag!(raw, Tag::ImageLength).get_usize(0);
    let bps = fetch_tag!(raw, Tag::BitsPerSample).get_usize(0);
    let offset = fetch_tag!(raw, Tag::StripOffsets).get_usize(0);
    let src = &self.buffer[offset..];

    let image = if camera.model == "NIKON D100" {
      width = 3040;
      decode_12be_wcontrol(src, width, height)
    } else {
      match fetch_tag!(raw, Tag::Compression).get_usize(0) {
        34713 => try!(self.decode_compressed(src, width, height, bps)),
        x => return Err(format!("Don't know how to handle compression {}", x).to_string()),
      }
    };

    ok_image(camera, width, height, try!(self.get_wb()), image)
  }
}

impl<'a> NefDecoder<'a> {
  fn get_wb(&self) -> Result<[f32;4], String> {
    if let Some(levels) = self.tiff.find_entry(Tag::NefWB0) {
      Ok([levels.get_f32(0), levels.get_f32(2), levels.get_f32(1), NAN])
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
        x => Err(format!("NEF: Don't know about WB version 0x{:x}", x).to_string()),
      }
    } else {
      Err("NEF: Don't know how to fetch WB".to_string())
    }
  }

  fn create_hufftable(&self, num: usize, bps: usize) -> Result<HuffTable,String> {
    let mut htable = HuffTable::empty(bps);

    let mut acc = 0 as usize;
    for i in 0..16 {
      htable.bits[i+1] = NIKON_TREE[num][i] as u32;
      acc += htable.bits[i+1] as usize;
    }
    htable.bits[0] = 0;

    for i in 0..acc {
      htable.huffval[i] = NIKON_TREE[num][i+16] as u32;
    }

    try!(htable.initialize(true));
    Ok(htable)
  }

  fn decode_compressed(&self, src: &[u8], width: usize, height: usize, bps: usize) -> Result<Vec<u16>, String> {
    let metaifd = fetch_ifd!(self.tiff, Tag::NefMeta1);
    let meta = if let Some(meta) = metaifd.find_entry(Tag::NefMeta2) {meta} else {
      fetch_tag!(metaifd, Tag::NefMeta1)
    };
    let mut stream = ByteStream::new(meta.get_data(), metaifd.get_endian());
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
    let mut htable = try!(self.create_hufftable(huff_select, bps));

    // Setup the predictors
    let mut pred_up1: [i32;2] = [stream.get_u16() as i32, stream.get_u16() as i32];
    let mut pred_up2: [i32;2] = [stream.get_u16() as i32, stream.get_u16() as i32];

    // Get the linearization curve
    let mut points = [0 as u16;65536];
    for i in 0..points.len() {
      points[i] = i as u16;
    }
    let mut max = (1 << bps) & 0x7fff;
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
      split = metaifd.get_endian().ru16(meta.get_data(), 562) as usize;
    } else if v0 != 70 && csize <= 0x4001 {
      for i in 0..csize {
        points[i] = stream.get_u16();
      }
      max = csize;
    }
    let curve = LookupTable::new(&points[0..max]);

    let mut out = vec![0 as u16; width * height];
    let mut pump = BitPumpMSB::new(src);
    let mut random = pump.peek_bits(24);

    for row in 0..height {
      if split > 0 && row == split {
        htable = try!(self.create_hufftable(huff_select+1, bps));
      }
      pred_up1[row&1] += try!(htable.huff_decode(&mut pump));
      pred_up2[row&1] += try!(htable.huff_decode(&mut pump));
      let mut pred_left1 = pred_up1[row&1];
      let mut pred_left2 = pred_up2[row&1];
      for col in (0..width).step(2) {
        if col > 0 {
          pred_left1 += try!(htable.huff_decode(&mut pump));
          pred_left2 += try!(htable.huff_decode(&mut pump));
        }
        out[row*width+col+0] = curve.dither(clampbits(pred_left1,15) as u16, &mut random);
        out[row*width+col+1] = curve.dither(clampbits(pred_left2,15) as u16, &mut random);
      }
    }

    Ok(out)
  }
}
