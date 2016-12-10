use decoders::*;
use decoders::tiff::*;
use decoders::basics::*;
use std::f32::NAN;
use itertools::Itertools;
use std::cmp;

#[derive(Debug, Clone)]
pub struct SrwDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: TiffIFD<'a>,
}

impl<'a> SrwDecoder<'a> {
  pub fn new(buf: &'a [u8], tiff: TiffIFD<'a>, rawloader: &'a RawLoader) -> SrwDecoder<'a> {
    SrwDecoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for SrwDecoder<'a> {
  fn identify(&self) -> Result<&Camera, String> {
    let make = fetch_tag!(self.tiff, Tag::Make, "SRW: Couldn't find Make").get_str();
    let model = fetch_tag!(self.tiff, Tag::Model, "SRW: Couldn't find Model").get_str();
    self.rawloader.check_supported(make, model)
  }

  fn image(&self) -> Result<Image,String> {
    let camera = try!(self.identify());
    let data = self.tiff.find_ifds_with_tag(Tag::StripOffsets);
    let raw = data[0];
    let width = fetch_tag!(raw, Tag::ImageWidth, "SRW: Couldn't find width").get_u32(0);
    let height = fetch_tag!(raw, Tag::ImageLength, "SRW: Couldn't find height").get_u32(0);
    let offset = fetch_tag!(raw, Tag::StripOffsets, "SRW: Couldn't find offset").get_u32(0) as usize;
    let compression = fetch_tag!(raw, Tag::Compression, "SRW: Couldn't find compression").get_u32(0);
    let bits = fetch_tag!(raw, Tag::BitsPerSample, "SRW: Couldn't find bps").get_u32(0);
    let src = &self.buffer[offset..];

    let image = match compression {
      32770 => {
        match raw.find_entry(Tag::SrwSensorAreas) {
          None => match bits {
            12 => decode_12be(src, width as usize, height as usize),
            14 => decode_14le_unpacked(src, width as usize, height as usize),
             x => return Err(format!("SRW: Don't know how to handle bps {}", x).to_string()),
          },
          Some(x) => {
            let coffset = x.get_u32(0) as usize;
            let loffsets = &self.buffer[coffset..];
            SrwDecoder::decode_srw1(src, loffsets, width as usize, height as usize)
          }
        }
      }
      32772 => {
       SrwDecoder::decode_srw2(src, width as usize, height as usize)
      }
      x => return Err(format!("SRW: Don't know how to handle compression {}", x).to_string()),
    };

    ok_image(camera, width, height, try!(self.get_wb()), image)
  }
}

impl<'a> SrwDecoder<'a> {
  pub fn decode_srw1(buf: &[u8], loffsets: &[u8], width: usize, height: usize) -> Vec<u16> {
    let mut out: Vec<u16> = vec![0; (width*height) as usize];

    for row in 0..height {
      let mut len: [u32; 4] = [if row < 2 {7} else {4}; 4];
      let loffset = LEu32(loffsets, row*4) as usize;
      let mut pump = BitPumpMSB32::new(&buf[loffset..]);

      let img      = width*row;
      let img_up   = width*(cmp::max(1, row)-1);
      let img_up2  = width*(cmp::max(2, row)-2);

      // Image is arranged in groups of 16 pixels horizontally
      for col in (0..width).step(16) {
        let dir = pump.get_bits(1) == 1;

        let ops = [pump.get_bits(2), pump.get_bits(2), pump.get_bits(2), pump.get_bits(2)];
        for (i, op) in ops.iter().enumerate() {
          match *op {
            3 => {len[i] = pump.get_bits(4);},
            2 => {len[i] -= 1;},
            1 => {len[i] += 1;},
            _ => {},
          }
        }

        // First decode even pixels
        for c in (0..16).step(2) {
          let l = len[(c >> 3)];
          let adj = pump.get_ibits_sextended(l);
          let predictor = if dir { // Upward prediction
              out[img_up+col+c]
          } else { // Left to right prediction
              if col == 0 { 128 } else { out[img+col-2] }
          };
          if col+c < width { // No point in decoding pixels outside the image
            out[img+col+c] = ((predictor as i32) + adj) as u16;
          }
        }
        // Now decode odd pixels
        for c in (1..16).step(2) {
          let l = len[2 | (c >> 3)];
          let adj = pump.get_ibits_sextended(l);
          let predictor = if dir { // Upward prediction
              out[img_up2+col+c]
          } else { // Left to right prediction
              if col == 0 { 128 } else { out[img+col-1] }
          };
          if col+c < width { // No point in decoding pixels outside the image
            out[img+col+c] = ((predictor as i32) + adj) as u16;
          }
        }
      }
    }

    // SRW1 apparently has red and blue swapped, just changing the CFA pattern to
    // match causes color fringing in high contrast areas because the actual pixel
    // locations would not match the CFA pattern
    for row in (0..height).step(2) {
      for col in (0..width).step(2) {
        out.swap(row*width+col+1, (row+1)*width+col);
      }
    }

    out
  }

  pub fn decode_srw2(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
    let mut out: Vec<u16> = vec![0; (width*height) as usize];

    // This format has a variable length encoding of how many bits are needed
    // to encode the difference between pixels, we use a table to process it
    // that has two values, the first the number of bits that were used to
    // encode, the second the number of bits that come after with the difference
    // The table has 14 entries because the difference can have between 0 (no
    // difference) and 13 bits (differences between 12 bits numbers can need 13)
    let tab: [[u32;2];14] = [[3,4], [3,7], [2,6], [2,5], [4,3], [6,0], [7,9],
                             [8,10], [9,11], [10,12], [10,13], [5,1], [4,8], [4,2]];

    // We generate a 1024 entry table (to be addressed by reading 10 bits) by
    // consecutively filling in 2^(10-N) positions where N is the variable number of
    // bits of the encoding. So for example 4 is encoded with 3 bits so the first
    // 2^(10-3)=128 positions are set with 3,4 so that any time we read 000 we
    // know the next 4 bits are the difference. We read 10 bits because that is
    // the maximum number of bits used in the variable encoding (for the 12 and
    // 13 cases)
    let mut tbl: [[u32;2];1024] = [[0,0];1024];
    let mut n: usize = 0;
    for i in 0..14 {
      let mut c = 0;
      while c < (1024 >> tab[i][0]) {
        tbl[n][0] = tab[i][0];
        tbl[n][1] = tab[i][1];
        n += 1;
        c += 1;
      }
    }

    let mut vpred: [[i32;2];2] = [[0,0],[0,0]];
    let mut hpred: [i32;2] = [0,0];
    let mut pump = BitPumpMSB::new(buf);
    for row in 0..height {
      for col in 0..width {
        let diff = SrwDecoder::srw2_diff(&mut pump, &tbl);
        if col < 2 {
          vpred[row & 1][col] += diff;
          hpred[col] = vpred[row & 1][col];
        } else {
          hpred[col & 1] += diff;
        }
        out[row*width+col] = hpred[col & 1] as u16;
      }
    }

    out
  }

  pub fn srw2_diff(pump: &mut BitPumpMSB, tbl: &[[u32;2];1024]) -> i32{
    // We read 10 bits to index into our table
    let c = pump.peek_bits(10);
    // Skip the bits that were used to encode this case
    pump.consume_bits(tbl[c as usize][0]);
    // Read the number of bits the table tells me
    let len = tbl[c as usize][1];
    let mut diff = pump.get_bits(len) as i32;
    // If the first bit is 0 we need to turn this into a negative number
    if len != 0 && (diff & (1 << (len-1))) == 0 {
      diff -= (1 << len) - 1;
    }
    diff
  }


  fn get_wb(&self) -> Result<[f32;4], String> {
    let rggb_levels = fetch_tag!(self.tiff, Tag::SrwRGGBLevels, "SRW: No RGGB Levels");
    let rggb_blacks = fetch_tag!(self.tiff, Tag::SrwRGGBBlacks, "SRW: No RGGB Blacks");
    if rggb_levels.count() != 4 || rggb_blacks.count() != 4 {
      Err("SRW: RGGB Levels and Blacks don't have 4 elements".to_string())
    } else {
      let nlevels = &rggb_levels.copy_offset_from_parent(&self.buffer);
      let nblacks = &rggb_blacks.copy_offset_from_parent(&self.buffer);
      Ok([nlevels.get_u32(0) as f32 - nblacks.get_u32(0) as f32,
          nlevels.get_u32(1) as f32 - nblacks.get_u32(1) as f32,
          nlevels.get_u32(3) as f32 - nblacks.get_u32(3) as f32,
          NAN])
    }
  }
}
