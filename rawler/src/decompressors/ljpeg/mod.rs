use crate::bits::Endian;
use crate::decoders::decode_threaded_multiline;
use crate::decompressors::ljpeg::decompressors::*;
use crate::decompressors::ljpeg::huffman::*;
use crate::pixarray::PixU16;
use crate::pumps::ByteStream;

mod decompressors;
pub mod huffman;

enum Marker {
  Stuff = 0x00,
  SOF3 = 0xc3, // lossless
  DHT = 0xc4,  // huffman tables
  SOI = 0xd8,  // start of image
  EOI = 0xd9,  // end of image
  SOS = 0xda,  // start of scan
  DQT = 0xdb,  // quantization tables
  Fill = 0xff,
}

fn m(marker: Marker) -> u8 {
  marker as u8
}

#[derive(Debug, Copy, Clone)]
struct JpegComponentInfo {
  // These values are fixed over the whole image, read from the SOF marker.
  id: usize, // identifier for this component (0..255)
  #[allow(dead_code)]
  index: usize, // its index in SOF or cPtr->compInfo[]

  // Huffman table selector (0..3). The value may vary between scans.
  // It is read from the SOS marker.
  dc_tbl_num: usize,
  super_h: usize, // Horizontal Supersampling
  super_v: usize, // Vertical Supersampling
}

#[derive(Debug, Clone)]
struct SOFInfo {
  width: usize,
  height: usize,
  cps: usize,
  precision: usize,
  components: Vec<JpegComponentInfo>,
  csfix: bool,
}

impl SOFInfo {
  fn empty(csfix: bool) -> SOFInfo {
    SOFInfo {
      width: 0,
      height: 0,
      cps: 0,
      precision: 0,
      components: Vec::new(),
      csfix,
    }
  }

  fn parse_sof(&mut self, input: &mut ByteStream) -> Result<(), String> {
    let header_length = input.get_u16() as usize;
    self.precision = input.get_u8() as usize;
    self.height = input.get_u16() as usize;
    self.width = input.get_u16() as usize;
    self.cps = input.get_u8() as usize;

    if self.precision > 16 {
      return Err("ljpeg: More than 16 bits per channel is not supported.".to_string());
    }
    if self.cps > 4 || self.cps < 1 {
      return Err("ljpeg: Only from 1 to 4 components are supported.".to_string());
    }
    if header_length != 8 + self.cps * 3 {
      return Err("ljpeg: Header size mismatch.".to_string());
    }

    for i in 0..self.cps {
      let id = input.get_u8() as usize;
      let subs = input.get_u8() as usize;
      input.get_u8(); // Skip info about quantized

      self.components.push(JpegComponentInfo {
        id,
        index: i,
        dc_tbl_num: 0,
        super_v: subs & 0xf,
        super_h: subs >> 4,
      });
    }
    Ok(())
  }

  fn parse_sos(&mut self, input: &mut ByteStream) -> Result<(usize, usize), String> {
    if self.width == 0 {
      return Err("ljpeg: Trying to parse SOS before SOF".to_string());
    }
    input.get_u16(); //skip header length
    let soscps = input.get_u8() as usize;
    if self.cps != soscps {
      return Err("ljpeg: component number mismatch in SOS".to_string());
    }
    for cs in 0..self.cps {
      // At least some MOS cameras have this broken
      let readcs = input.get_u8() as usize;
      let cs = if self.csfix { cs } else { readcs };
      let component = match self.components.iter_mut().find(|&&mut c| c.id == cs) {
        Some(val) => val,
        None => return Err(format!("ljpeg: invalid component selector {}", cs)),
      };
      let td = (input.get_u8() as usize) >> 4;
      if td > 3 {
        return Err("ljpeg: Invalid Huffman table selection".to_string());
      }
      component.dc_tbl_num = td;
    }
    let pred = input.get_u8() as usize;
    input.get_u8(); // Se + Ah Not used in LJPEG
    let pt = (input.get_u8() as usize) & 0xf; // Point Transform
    Ok((pred, pt))
  }
}

#[derive(Debug)]
pub struct LjpegDecompressor<'a> {
  buffer: &'a [u8],
  sof: SOFInfo,
  predictor: usize,
  point_transform: usize,
  dhts: Vec<HuffTable>,
}

impl<'a> LjpegDecompressor<'a> {
  pub fn new(src: &'a [u8]) -> Result<LjpegDecompressor, String> {
    LjpegDecompressor::new_full(src, false, false)
  }

  pub fn new_full(src: &'a [u8], dng_bug: bool, csfix: bool) -> Result<LjpegDecompressor, String> {
    let mut input = ByteStream::new(src, Endian::Big);
    if LjpegDecompressor::get_next_marker(&mut input, false)? != m(Marker::SOI) {
      return Err("ljpeg: Image did not start with SOI. Probably not LJPEG".to_string());
    }

    let mut sof = SOFInfo::empty(csfix);
    let mut dht_init = [false; 4];
    let mut dht_bits = [[0_u32; 17]; 4];
    let mut dht_huffval = [[0_u32; 256]; 4];
    let pred;
    let pt;
    loop {
      let marker = LjpegDecompressor::get_next_marker(&mut input, true)?;
      if marker == m(Marker::SOF3) {
        // Start of the frame, giving us the basic info
        sof.parse_sof(&mut input)?;
        if sof.precision > 16 || sof.precision < 12 {
          return Err(format!("ljpeg: sof.precision {}", sof.precision));
        }
      } else if marker == m(Marker::DHT) {
        // Huffman table settings
        LjpegDecompressor::parse_dht(&mut input, &mut dht_init, &mut dht_bits, &mut dht_huffval)?;
      } else if marker == m(Marker::SOS) {
        // Start of the actual stream, we can decode after this
        let (a, b) = sof.parse_sos(&mut input)?;
        pred = a;
        pt = b;
        break;
      } else if marker == m(Marker::EOI) {
        // Should never be reached as we stop at SOS
        return Err("ljpeg: reached EOI before SOS".to_string());
      } else if marker == m(Marker::DQT) {
        return Err("ljpeg: not a valid raw file, found DQT".to_string());
      }
    }

    let mut dhts = Vec::new();
    for i in 0..4 {
      dhts.push(if dht_init[i] {
        HuffTable::new(dht_bits[i], dht_huffval[i], dng_bug)?
      } else {
        HuffTable::empty()
      });
    }

    log::debug!(
      "LJPEGDecompressor: super_h: {}, super_v: {}, pred: {}, pt: {}, prec: {}",
      sof.components[0].super_h,
      sof.components[0].super_v,
      pred,
      pt,
      sof.precision
    );

    if sof.components[0].super_h == 2 && sof.components[0].super_v == 2 {
      log::debug!("LJPEG with YUV 4:2:0 encoding");
    } else if sof.components[0].super_h == 2 && sof.components[0].super_v == 1 {
      log::debug!("LJPEG with YUV 4:2:2 encoding");
    }

    let offset = input.get_pos();
    Ok(LjpegDecompressor {
      buffer: &src[offset..],
      sof,
      predictor: pred,
      point_transform: pt,
      dhts,
    })
  }

  fn get_next_marker(input: &mut ByteStream, allowskip: bool) -> Result<u8, String> {
    if !allowskip {
      if input.get_u8() != 0xff {
        return Err("ljpeg: (noskip) expected marker not found".to_string());
      }
      let mark = input.get_u8();
      if mark == m(Marker::Stuff) || mark == m(Marker::Fill) {
        return Err("ljpeg: (noskip) expected marker but found stuff or fill".to_string());
      }
      return Ok(mark);
    }
    input.skip_to_marker()?;

    Ok(input.get_u8())
  }

  fn parse_dht(input: &mut ByteStream, init: &mut [bool; 4], bits: &mut [[u32; 17]; 4], huffval: &mut [[u32; 256]; 4]) -> Result<(), String> {
    let mut length = (input.get_u16() as usize) - 2;

    while length > 0 {
      let b = input.get_u8() as usize;
      let tc = b >> 4;
      let th = b & 0xf;

      if tc != 0 {
        return Err("ljpeg: unsuported table class in DHT".to_string());
      }
      if th > 3 {
        return Err(format!("ljpeg: unsuported table id {}", th));
      }

      let mut acc: usize = 0;
      for i in 0..16 {
        bits[th][i + 1] = input.get_u8() as u32;
        acc += bits[th][i + 1] as usize;
      }
      bits[th][0] = 0;

      if acc > 256 {
        return Err("ljpeg: invalid DHT table".to_string());
      }

      if length < 1 + 16 + acc {
        return Err("ljpeg: invalid DHT table length".to_string());
      }

      for i in 0..acc {
        huffval[th][i] = input.get_u8() as u32;
      }

      init[th] = true;
      length -= 1 + 16 + acc;
    }

    Ok(())
  }

  /// Handle special SONY YUV 4:2:0 encoding in ILCE-7RM5
  pub fn decode_sony(&self, out: &mut [u16], x: usize, stripwidth: usize, width: usize, height: usize, dummy: bool) -> Result<(), String> {
    if dummy {
      return Ok(());
    }
    log::debug!("LJPEG decode with special Sony mode");
    if self.sof.components[0].super_h == 2 && self.sof.components[0].super_v == 2 {
      decode_sony_ljpeg_420(self, out, width, height)
    } else if self.sof.components[0].super_h == 2 && self.sof.components[0].super_v == 1 {
      decode_ljpeg_422(self, out, width, height)
    } else if self.sof.components[0].super_h == 1 && self.sof.components[0].super_v == 1 {
      match self.predictor {
        1 | 2 | 3 | 4 | 5 | 6 | 7 => decode_ljpeg(self, out, x, stripwidth, width, height),
        8 => decode_hasselblad(self, out, width),
        p => Err(format!("ljpeg: predictor {} not supported", p)),
      }
    } else {
      Err(format!(
        "ljpeg: unsupported interleave configuration, super_h: {}, super_v: {}",
        self.sof.components[0].super_h, self.sof.components[0].super_v
      ))
    }
  }

  pub fn decode(&self, out: &mut [u16], x: usize, stripwidth: usize, width: usize, height: usize, dummy: bool) -> Result<(), String> {
    if dummy {
      return Ok(());
    }

    if self.sof.components[0].super_h == 2 && self.sof.components[0].super_v == 2 {
      return decode_ljpeg_420(self, out, width, height);
    } else if self.sof.components[0].super_h == 2 && self.sof.components[0].super_v == 1 {
      return decode_ljpeg_422(self, out, width, height);
    } else if self.sof.components[0].super_h == 1 && self.sof.components[0].super_v == 1 {
      match self.predictor {
        1 | 2 | 3 | 4 | 5 | 6 | 7 => decode_ljpeg(self, out, x, stripwidth, width, height),
        8 => decode_hasselblad(self, out, width),
        p => Err(format!("ljpeg: predictor {} not supported", p)),
      }
    } else {
      Err(format!(
        "ljpeg: unsupported interleave configuration, super_h: {}, super_v: {}",
        self.sof.components[0].super_h, self.sof.components[0].super_v
      ))
    }
  }

  pub fn decode_leaf(&self, width: usize, height: usize) -> Result<PixU16, String> {
    let mut offsets = vec![0_usize; 1];
    let mut input = ByteStream::new(self.buffer, Endian::Big);

    while let Ok(marker) = LjpegDecompressor::get_next_marker(&mut input, true) {
      if marker == m(Marker::EOI) {
        break;
      }
      offsets.push(input.get_pos());
    }
    let nstrips = (height - 1) / 8 + 1;
    if offsets.len() != nstrips {
      return Err(format!("MOS: expecting {} strips found {}", nstrips, offsets.len()));
    }

    let htable1 = &self.dhts[self.sof.components[0].dc_tbl_num];
    let htable2 = &self.dhts[self.sof.components[1].dc_tbl_num];
    let bpred = 1 << (self.sof.precision - self.point_transform - 1);
    Ok(decode_threaded_multiline(
      width,
      height,
      8,
      false,
      &(|strip: &mut [u16], block| {
        let block = block / 8;
        let offset = offsets[block];
        let nlines = strip.len() / width;
        decode_leaf_strip(&self.buffer[offset..], strip, width, nlines, htable1, htable2, bpred).unwrap();
      }),
    ))
  }

  pub fn width(&self) -> usize {
    self.sof.width * self.sof.cps
  }
  pub fn height(&self) -> usize {
    self.sof.height
  }
  pub fn super_v(&self) -> usize {
    self.sof.components[0].super_v
  }
  pub fn super_h(&self) -> usize {
    self.sof.components[0].super_h
  }
  pub fn components(&self) -> usize {
    self.sof.components.len()
  }
}
