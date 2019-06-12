use decoders::*;
use decoders::tiff::*;
use decoders::basics::*;
use decoders::ljpeg::huffman::*;
use std::f32::NAN;

#[derive(Debug, Clone)]
pub struct PefDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: TiffIFD<'a>,
}

impl<'a> PefDecoder<'a> {
  pub fn new(buf: &'a [u8], tiff: TiffIFD<'a>, rawloader: &'a RawLoader) -> PefDecoder<'a> {
    PefDecoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for PefDecoder<'a> {
  fn image(&self, dummy: bool) -> Result<RawImage,String> {
    let camera = try!(self.rawloader.check_supported(&self.tiff));
    let raw = fetch_ifd!(&self.tiff, Tag::StripOffsets);
    let width = fetch_tag!(raw, Tag::ImageWidth).get_usize(0);
    let height = fetch_tag!(raw, Tag::ImageLength).get_usize(0);
    let offset = fetch_tag!(raw, Tag::StripOffsets).get_usize(0);
    let src = &self.buffer[offset..];

    let image = match fetch_tag!(raw, Tag::Compression).get_u32(0) {
      1 => decode_16be(src, width, height, dummy),
      32773 => decode_12be(src, width, height, dummy),
      65535 => try!(self.decode_compressed(src, width, height, dummy)),
      c => return Err(format!("PEF: Don't know how to read compression {}", c).to_string()),
    };

    let blacklevels = self.get_blacklevels().unwrap_or(camera.blacklevels);
    ok_image_with_blacklevels(camera, width, height, try!(self.get_wb()), blacklevels, image)
  }
}

impl<'a> PefDecoder<'a> {
  fn get_wb(&self) -> Result<[f32;4], String> {
    let levels = fetch_tag!(self.tiff, Tag::PefWB);
    Ok([levels.get_f32(0), levels.get_f32(1), levels.get_f32(3), NAN])
  }

  fn get_blacklevels(&self) -> Option<[u16;4]> {
    match self.tiff.find_entry(Tag::PefBlackLevels) {
      Some(levels) => {
        Some([levels.get_f32(0) as u16,levels.get_f32(1) as u16,
             levels.get_f32(2) as u16,levels.get_f32(3) as u16])
      },
      None => None,
    }
  }

  fn decode_compressed(&self, src: &[u8], width: usize, height: usize, dummy: bool) -> Result<Vec<u16>,String> {
    if let Some(huff) = self.tiff.find_entry(Tag::PefHuffman) {
      Self::do_decode(src, Some((huff.get_data(), self.tiff.get_endian())), width, height, dummy)
    } else {
      Self::do_decode(src, None, width, height, dummy)
    }
  }

  pub(crate) fn do_decode(src: &[u8], huff: Option<(&[u8], Endian)>, width: usize, height: usize, dummy: bool) -> Result<Vec<u16>,String> {
    let mut out = alloc_image_ok!(width, height, dummy);
    let mut htable = HuffTable::empty(16);

    /* Attempt to read huffman table, if found in makernote */
    if let Some((huff, endian)) = huff {
      let mut stream = ByteStream::new(huff, endian);

      let depth: usize = (stream.get_u16() as usize + 12) & 0xf;
      stream.consume_bytes(12);

      let mut v0: [u32;16] = [0;16];
      for i in 0..depth {
        v0[i] = stream.get_u16() as u32;
      }

      let mut v1: [u32;16] = [0;16];
      for i in 0..depth {
        v1[i] = stream.get_u8() as u32;
      }

      // Calculate codes and store bitcounts
      let mut v2: [u32;16] = [0;16];
      for c in 0..depth {
        v2[c] = v0[c] >> (12 - v1[c]);
        htable.bits[v1[c] as usize] += 1;
      }

      // Find smallest
      for i in 0..depth {
        let mut sm_val: u32 = 0xfffffff;
        let mut sm_num: u32 = 0xff;
        for j in 0..depth {
          if v2[j] <= sm_val {
            sm_num = j as u32;
            sm_val = v2[j];
          }
        }
        htable.huffval[i] = sm_num;
        v2[sm_num as usize]=0xffffffff;
      }
    } else {
      // Initialize with legacy data
      let pentax_tree: [u8; 29] = [ 0, 2, 3, 1, 1, 1, 1, 1, 1, 2, 0, 0, 0, 0, 0, 0,
                                    3, 4, 2, 5, 1, 6, 0, 7, 8, 9, 10, 11, 12 ];
      let mut acc: usize = 0;
      for i in 0..16 {
        htable.bits[i+1] = pentax_tree[i] as u32;
        acc += htable.bits[i+1] as usize;
      }
      for i in 0..acc {
        htable.huffval[i] = pentax_tree[i+16] as u32;
      }
    }

    try!(htable.initialize(true));

    let mut pump = BitPumpMSB::new(src);
    let mut pred_up1: [i32;2] = [0, 0];
    let mut pred_up2: [i32;2] = [0, 0];
    let mut pred_left1: i32;
    let mut pred_left2: i32;

    for row in 0..height {
      pred_up1[row & 1] += try!(htable.huff_decode(&mut pump));
      pred_up2[row & 1] += try!(htable.huff_decode(&mut pump));
      pred_left1 = pred_up1[row & 1];
      pred_left2 = pred_up2[row & 1];
      out[row*width+0] = pred_left1 as u16;
      out[row*width+1] = pred_left2 as u16;
      for col in (2..width).step_by(2) {
        pred_left1 += try!(htable.huff_decode(&mut pump));
        pred_left2 += try!(htable.huff_decode(&mut pump));
        out[row*width+col+0] = pred_left1 as u16;
        out[row*width+col+1] = pred_left2 as u16;
      }
    }
    Ok(out)
  }
}
