use decoders::*;
use decoders::tiff::*;
use decoders::basics::*;
use std::f32::NAN;
use std::cmp;

#[derive(Debug, Clone)]
pub struct OrfDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: TiffIFD<'a>,
}

impl<'a> OrfDecoder<'a> {
  pub fn new(buf: &'a [u8], tiff: TiffIFD<'a>, rawloader: &'a RawLoader) -> OrfDecoder<'a> {
    OrfDecoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for OrfDecoder<'a> {
  fn image(&self) -> Result<Image,String> {
    let camera = try!(self.rawloader.check_supported(&self.tiff));
    let raw = fetch_ifd!(&self.tiff, Tag::StripOffsets);
    let width = fetch_tag!(raw, Tag::ImageWidth).get_u32(0);
    let height = fetch_tag!(raw, Tag::ImageLength).get_u32(0);
    let offset = fetch_tag!(raw, Tag::StripOffsets).get_u32(0) as usize;
    let counts = fetch_tag!(raw, Tag::StripByteCounts);
    let mut size: usize = 0;
    for i in 0..counts.count() {
      size += counts.get_u32(i as usize) as usize;
    }

    let src = &self.buffer[offset .. self.buffer.len()];

    let image = if size >= ((width*height*2) as usize) {
      if self.tiff.little_endian() {
        decode_12le_unpacked_left_aligned(src, width as usize, height as usize)
      } else {
        decode_12be_unpacked_left_aligned(src, width as usize, height as usize)
      }
    } else if size >= ((width*height/10*16) as usize) {
      decode_12le_wcontrol(src, width as usize, height as usize)
    } else if size >= ((width*height*12/8) as usize) {
      if width < 3500 { // The interlaced stuff is all old and smaller
        decode_12be_interlaced(src, width as usize, height as usize)
      } else {
        decode_12be_msb32(src, width as usize, height as usize)
      }
    } else {
      OrfDecoder::decode_compressed(src, width as usize, height as usize)
    };
    ok_image(camera, width, height, try!(self.get_wb()), image)
  }
}


impl<'a> OrfDecoder<'a> {
  /* This is probably the slowest decoder of them all.
   * I cannot see any way to effectively speed up the prediction
   * phase, which is by far the slowest part of this algorithm.
   * Also there is no way to multithread this code, since prediction
   * is based on the output of all previous pixel (bar the first four)
   */

  pub fn decode_compressed(buf: &'a [u8], width: usize, height: usize) -> Vec<u16> {
    let mut out: Vec<u16> = vec![0; (width*height) as usize];

    /* Build a table to quickly look up "high" value */
    let mut bittable: [u8; 4096] = [0; 4096];
    for i in 0..4096 {
      let mut b = 12;
      for high in 0..12 {
        if ((i >> (11-high))&1) != 0 { b = high; break }
      }
      bittable[i] = b;
    }

    let mut left: [i32; 2] = [0; 2];
    let mut nw: [i32; 2] = [0; 2];
    let mut pump = BitPumpMSB::new(&buf[7..]);

    for row in 0..height {
      let mut acarry: [[i32; 3];2] = [[0; 3];2];

      for c in 0..width/2 {
        let col: usize = c * 2;
        for s in 0..2 { // Run twice for odd and even pixels
          let i = if acarry[s][2] < 3 {2} else {0};
          let mut nbits = 2 + i;
          while ((acarry[s][0] >> (nbits + i)) & 0xffff) > 0 { nbits += 1 }
          nbits = cmp::min(nbits, 16);
          let b = pump.peek_ibits(15);

          let sign: i32 = (b >> 14) * -1;
          let low: i32  = (b >> 12) &  3;
          let mut high: i32 = bittable[(b&4095) as usize] as i32;

          // Skip bytes used above or read bits
          if high == 12 {
            pump.consume_bits(15);
            high = pump.get_ibits(16 - nbits) >> 1;
          } else {
            pump.consume_bits((high + 4) as u32);
          }

          acarry[s][0] = ((high << nbits) | pump.get_ibits(nbits)) as i32;
          let diff = (acarry[s][0] ^ sign) + acarry[s][1];
          acarry[s][1] = (diff * 3 + acarry[s][1]) >> 5;
          acarry[s][2] = if acarry[s][0] > 16 { 0 } else { acarry[s][2] + 1 };

          if row < 2 || col < 2 { // We're in a border, special care is needed
            let pred = if row < 2 && col < 2 { // We're in the top left corner
              0
            } else if row < 2 { // We're going along the top border
              left[s]
            } else { // col < 2, we're at the start of a line
              nw[s] = out[(row-2) * width + (col+s)] as i32;
              nw[s]
            };
            left[s] = pred + ((diff << 2) | low);
            out[row*width + (col+s)] = left[s] as u16;
          } else {
            let up: i32 = out[(row-2) * width + (col+s)] as i32;
            let left_minus_nw: i32 = left[s] - nw[s];
            let up_minus_nw: i32 = up - nw[s];
            // Check if sign is different, and one is not zero
            let pred = if left_minus_nw * up_minus_nw < 0 {
              if left_minus_nw.abs() > 32 || up_minus_nw.abs() > 32 {
                left[s] + up_minus_nw
              } else {
                (left[s] + up) >> 1
              }
            } else {
              if left_minus_nw.abs() > up_minus_nw.abs() { left[s] } else { up }
            };

            left[s] = pred + ((diff << 2) | low);
            nw[s] = up;
            out[(row*width + (col+s)) as usize] = left[s] as u16;
          }
        }
      }
    }
    out
  }

  fn get_wb(&self) -> Result<[f32;4],String> {
    let redmul = self.tiff.find_entry(Tag::OlympusRedMul);
    let bluemul = self.tiff.find_entry(Tag::OlympusBlueMul);

    if redmul.is_some() && bluemul.is_some() {
      Ok([redmul.unwrap().get_u32(0) as f32,256.0,bluemul.unwrap().get_u32(0) as f32,NAN])
    } else {
      let iproc = fetch_tag!(self.tiff,Tag::OlympusImgProc);
      let poff = iproc.parent_offset() - 12;
      let off = (iproc.get_u32(0) as usize) + poff;
      let ifd = try!(TiffIFD::new(self.buffer, off, 0, 0, 0, self.tiff.get_endian()));
      let wbs = fetch_tag!(ifd, Tag::ImageWidth);
      if wbs.count() == 4 {
        let off = poff + wbs.doffset();
        let nwbs = &wbs.copy_with_new_data(&self.buffer[off..]);
        Ok([nwbs.get_u32(0) as f32,256.0,nwbs.get_u32(1) as f32,NAN])
      } else {
        Ok([wbs.get_u32(0) as f32,256.0,wbs.get_u32(1) as f32,NAN])
      }
    }
  }
}
