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
  fn image(&self) -> Result<Image,String> {
    let camera = try!(self.rawloader.check_supported(&self.tiff));
    let data = self.tiff.find_ifds_with_tag(Tag::StripOffsets);
    let raw = data[0];
    let width = fetch_tag!(raw, Tag::ImageWidth).get_u32(0);
    let height = fetch_tag!(raw, Tag::ImageLength).get_u32(0);
    let offset = fetch_tag!(raw, Tag::StripOffsets).get_u32(0) as usize;
    let compression = fetch_tag!(raw, Tag::Compression).get_u32(0);
    let bits = fetch_tag!(raw, Tag::BitsPerSample).get_u32(0);
    let src = &self.buffer[offset..];

    let image = match compression {
      32769 => match bits {
        12 => decode_12le_unpacked(src, width as usize, height as usize),
        14 => decode_14le_unpacked(src, width as usize, height as usize),
         x => return Err(format!("SRW: Don't know how to handle bps {}", x).to_string()),
      },
      32770 => {
        match raw.find_entry(Tag::SrwSensorAreas) {
          None => match bits {
            12 => {
              if camera.find_hint("little_endian") {
                decode_12le(src, width as usize, height as usize)
              } else {
                decode_12be(src, width as usize, height as usize)
              }
            },
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
      32773 => {
       SrwDecoder::decode_srw3(src, width as usize, height as usize)
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

  pub fn decode_srw3(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
    // Decoder for third generation compressed SRW files (NX1)
    // Seriously Samsung just use lossless jpeg already, it compresses better too :)

    // Thanks to Michael Reichmann (Luminous Landscape) for putting me in contact
    // and Loring von Palleske (Samsung) for pointing to the open-source code of
    // Samsung's DNG converter at http://opensource.samsung.com/

    let mut out: Vec<u16> = vec![0; (width*height) as usize];
    let mut pump = BitPumpMSB32::new(buf);

    // Process the initial metadata bits, we only really use initVal, width and
    // height (the last two match the TIFF values anyway)
    pump.get_bits(16); // NLCVersion
    pump.get_bits(4);  // ImgFormat
    let bit_depth = pump.get_bits(4)+1;
    pump.get_bits(4);  // NumBlkInRCUnit
    pump.get_bits(4);  // CompressionRatio
    pump.get_bits(16);  // Width;
    pump.get_bits(16);  // Height;
    pump.get_bits(16); // TileWidth
    pump.get_bits(4);  // reserved

    // The format includes an optimization code that sets 3 flags to change the
    // decoding parameters
    let optflags = pump.get_bits(4);
    static OPT_SKIP: u32 = 1; // Skip checking if we need differences from previous line
    static OPT_MV  : u32 = 2; // Simplify motion vector definition
    static OPT_QP  : u32 = 4; // Don't scale the diff values

    pump.get_bits(8);  // OverlapWidth
    pump.get_bits(8);  // reserved
    pump.get_bits(8);  // Inc
    pump.get_bits(2);  // reserved
    let init_val = pump.get_bits(14) as u16;

    // The format is relatively straightforward. Each line gets encoded as a set
    // of differences from pixels from another line. Pixels are grouped in blocks
    // of 16 (8 green, 8 red or blue). Each block is encoded in three sections.
    // First 1 or 4 bits to specify which reference pixels to use, then a section
    // that specifies for each pixel the number of bits in the difference, then
    // the actual difference bits
    let mut line_offset = 0;
    for row in 0..height {
      line_offset += pump.get_pos();
      // Align pump to 16byte boundary
      if (line_offset & 0x0f) != 0 {
        line_offset += 16 - (line_offset & 0xf);
      }
      pump = BitPumpMSB32::new(&buf[line_offset..]);

      let img = width*row;
      let img_up   = width*(cmp::max(1, row)-1);
      let img_up2  = width*(cmp::max(2, row)-2);

      // Initialize the motion and diff modes at the start of the line
      let mut motion: usize = 7;
      // By default we are not scaling values at all
      let mut scale: i32 = 0;
      let mut diff_bits_mode: [[u32;2];3] = [[0;2];3];
      for i in 0..3 {
        let init: u32 = if row < 2 {7} else {4};
        diff_bits_mode[i][0] = init;
        diff_bits_mode[i][1] = init;
      }

      for col in (0..width).step(16) {
        // Calculate how much scaling the final values will need
        scale = if (optflags & OPT_QP) == 0 && (col & 63) == 0 {
          let scalevals: [i32;3] = [0,-2,2];
          let i = pump.get_bits(2) as usize;
          if i < 3 {
            scale+scalevals[i]
          } else {
            pump.get_bits(12) as i32
          }
        } else {
          0
        };

        // First we figure out which reference pixels mode we're in
        if (optflags & OPT_MV) != 0 {
          motion = if pump.get_bits(1) != 0 {3} else {7};
        } else if pump.get_bits(1) == 0 {
          motion = pump.get_bits(3) as usize;
        }

        if row < 2 && motion != 7 {
          panic!("SRW Decoder: At start of image and motion isn't 7. File corrupted?")
        }

        if motion == 7 {
          // The base case, just set all pixels to the previous ones on the same line
          // If we're at the left edge we just start at the initial value
          for i in 0..16 {
            out[img+col+i] = if col == 0 {init_val} else {out[img+col+i-2]};
          }
        } else {
          // The complex case, we now need to actually lookup one or two lines above
          if row < 2 {
            panic!("SRW: Got a previous line lookup on first two lines. File corrupted?");
          }
          let motion_offset: [isize;7]  = [-4,-2,-2,0,0,2,4];
          let motion_average: [i32;7] = [ 0, 0, 1,0,1,0,0];
          let slide_offset = motion_offset[motion];

          for i in 0..16 {
            let refpixel: usize = if ((row+i) & 0x1) != 0 {
              // Red or blue pixels use same color two lines up
              ((img_up2 + col + i) as isize + slide_offset) as usize
            } else {
              // Green pixel N uses Green pixel N from row above (top left or top right)
              if (i % 2) != 0 {
                ((img_up + col + i - 1) as isize + slide_offset) as usize
              } else {
                ((img_up + col + i + 1) as isize + slide_offset) as usize
              }
            };
            // In some cases we use as reference interpolation of this pixel and the next
            out[img+col+i] = if motion_average[motion] != 0 {
              (out[refpixel] + out[refpixel+2] + 1) >> 1
            } else {
              out[refpixel]
            }
          }
        }

        // Figure out how many difference bits we have to read for each pixel
        let mut diff_bits: [u32; 4] = [0;4];
        if (optflags & OPT_SKIP) != 0 || pump.get_bits(1) == 0 {
          let flags: [u32; 4] = [pump.get_bits(2), pump.get_bits(2), pump.get_bits(2), pump.get_bits(2)];
          for i in 0..4 {
            // The color is 0-Green 1-Blue 2-Red
            let colornum: usize = if row % 2 != 0 {i>>1} else {((i>>1)+2) % 3};
            match flags[i] {
              0 => {diff_bits[i] = diff_bits_mode[colornum][0];},
              1 => {diff_bits[i] = diff_bits_mode[colornum][0]+1;},
              2 => {diff_bits[i] = diff_bits_mode[colornum][0]-1;},
              3 => {diff_bits[i] = pump.get_bits(4);},
              _ => {},
            }
            diff_bits_mode[colornum][0] = diff_bits_mode[colornum][1];
            diff_bits_mode[colornum][1] = diff_bits[i];
            if diff_bits[i] > bit_depth+1 {
              panic!("SRW Decoder: Too many difference bits. File corrupted?");
            }
          }
        }

        // Actually read the differences and write them to the pixels
        for i in 0..16 {
          let len = diff_bits[i>>2];
          let mut diff = pump.get_ibits_sextended(len);
          diff = diff * (scale*2+1) + scale;

          // Apply the diff to pixels 0 2 4 6 8 10 12 14 1 3 5 7 9 11 13 15
          let pos = if row % 2 != 0 {
            ((i&0x7) << 1) + 1 - (i>>3)
          } else {
            ((i&0x7) << 1) + (i>>3)
          } + img + col;
          out[pos] = clampbits((out[pos] as i32) + diff, bit_depth) as u16;
        }
      }
    }

    out
  }

  fn get_wb(&self) -> Result<[f32;4], String> {
    let rggb_levels = fetch_tag!(self.tiff, Tag::SrwRGGBLevels);
    let rggb_blacks = fetch_tag!(self.tiff, Tag::SrwRGGBBlacks);
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
