use decoders::basics::*;

mod huffman;
mod decompressors;
use decoders::ljpeg::huffman::*;
use decoders::ljpeg::decompressors::*;

enum Marker {
  Stuff        = 0x00,
  SOF3         = 0xc3, // lossless
  DHT          = 0xc4, // huffman tables
  SOI          = 0xd8, // start of image
  EOI          = 0xd9, // end of image
  SOS          = 0xda, // start of scan
  DQT          = 0xdb, // quantization tables
  Fill         = 0xff,
}

fn m(marker: Marker) -> u8 {
  marker as u8
}

#[derive(Debug, Copy, Clone)]
struct JpegComponentInfo {
  // These values are fixed over the whole image, read from the SOF marker.
  id: usize,    // identifier for this component (0..255)
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
}

impl SOFInfo {
  fn empty() -> SOFInfo {
    SOFInfo {
      width: 0,
      height: 0,
      cps: 0,
      precision: 0,
      components: Vec::new(),
    }
  }

  fn parse_sof(&mut self, input: &mut ByteStream) -> Result<(), String> {
    let header_length = input.get_u16() as usize;
    self.precision = input.get_u8() as usize;
    self.height = input.get_u16() as usize;
    self.width = input.get_u16() as usize;
    self.cps = input.get_u8() as usize;

    if self.precision > 16 {
      return Err("ljpeg: More than 16 bits per channel is not supported.".to_string())
    }
    if self.cps > 4 || self.cps < 1 {
      return Err("ljpeg: Only from 1 to 4 components are supported.".to_string())
    }
    if header_length != 8 + self.cps*3 {
      return Err("ljpeg: Header size mismatch.".to_string())
    }

    for i in 0..self.cps {
      let id = input.get_u8() as usize;
      let subs = input.get_u8() as usize;
      if input.get_u8() != 0 {
        return Err("ljpeg: Quantized components not supported.".to_string())
      }
      self.components.push(JpegComponentInfo {
        id: id,
        index: i,
        dc_tbl_num: 0,
        super_h: subs & 0xf,
        super_v: subs >> 4,
      });
    }
    Ok(())
  }

  fn parse_sos(&mut self, input: &mut ByteStream) -> Result<(usize, usize), String> {
    if self.width == 0 {
      return Err("ljpeg: Trying to parse SOS before SOF".to_string())
    }
    input.get_u16(); //skip header length
    let soscps = input.get_u8() as usize;
    if self.cps != soscps {
      return Err("ljpeg: component number mismatch in SOS".to_string())
    }
    for _ in 0..self.cps {
      let cs = input.get_u8() as usize;
      let component = match self.components.iter_mut().find(|&&mut c| c.id == cs) {
        Some(val) => val,
        None => return Err("ljpeg: invalid component selector".to_string())
      };
      let td = (input.get_u8() as usize) >> 4;
      if td > 3 {
        return Err("ljpeg: Invalid Huffman table selection".to_string())
      }
      component.dc_tbl_num = td;
    }
    let pred = input.get_u8() as usize;
    if pred > 7 {
      return Err(format!("ljpeg: invalid predictor mode {}",pred).to_string())
    }
    input.get_u8();                           // Se + Ah Not used in LJPEG
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
  dhts: [HuffTable;4],
}

impl<'a> LjpegDecompressor<'a> {
  pub fn new(src: &'a [u8], use_bigtable: bool) -> Result<LjpegDecompressor, String> {
    let mut input = ByteStream::new(src, BIG_ENDIAN);

    if try!(LjpegDecompressor::get_next_marker(&mut input, false)) != m(Marker::SOI) {
      return Err("ljpeg: Image did not start with SOI. Probably not LJPEG".to_string())
    }

    let mut sof = SOFInfo::empty();
    let mut dhts = [HuffTable::empty(0),HuffTable::empty(0),
                    HuffTable::empty(0),HuffTable::empty(0)];
    let pred;
    let pt;
    loop {
      let marker = try!(LjpegDecompressor::get_next_marker(&mut input, true));
      if marker == m(Marker::SOF3) {
        // Start of the frame, giving us the basic info
        try!(sof.parse_sof(&mut input));
        if sof.precision > 16 || sof.precision < 12 {
          return Err(format!("ljpeg: sof.precision {}", sof.precision).to_string())
        }
        dhts = [HuffTable::empty(sof.precision),HuffTable::empty(sof.precision),
                HuffTable::empty(sof.precision),HuffTable::empty(sof.precision)];
      } else if marker == m(Marker::DHT) {
        // Huffman table settings
        try!(LjpegDecompressor::parse_dht(&mut input, &mut dhts, use_bigtable));
      } else if marker == m(Marker::SOS) {
        // Start of the actual stream, we can decode after this
        let (a, b) = try!(sof.parse_sos(&mut input));
        pred = a; pt = b;
        break;
      } else if marker == m(Marker::EOI) {
        // Should never be reached as we stop at SOS
        return Err("ljpeg: reached EOI before SOS".to_string())
      } else if marker == m(Marker::DQT) {
        return Err("ljpeg: not a valid raw file, found DQT".to_string())
      }
    }

    let offset = input.get_pos();
    Ok(LjpegDecompressor {
      buffer: &src[offset..],
      sof: sof,
      predictor: pred,
      point_transform: pt,
      dhts: dhts,
    })
  }

  fn get_next_marker(input: &mut ByteStream, allowskip:bool) -> Result<u8,String> {
    if !allowskip {
      if input.get_u8() != 0xff {
        return Err("ljpeg: (noskip) expected marker not found".to_string())
      }
      let mark = input.get_u8();
      if mark == m(Marker::Stuff) || mark == m(Marker::Fill) {
        return Err("ljpeg: (noskip) expected marker but found stuff or fill".to_string())
      }
      return Ok(mark)
    }
    try!(input.skip_to_marker());

    Ok(input.get_u8())
  }

  fn parse_dht(input: &mut ByteStream, htables: &mut [HuffTable;4], use_bigtable: bool) -> Result<(), String> {
    let mut length = (input.get_u16() as usize) - 2;

    while length > 0 {
      let b = input.get_u8() as usize;
      let tc = b >> 4;
      let th = b & 0xf;

      if tc != 0 {
        return Err("ljpeg: unsuported table class in DHT".to_string())
      }
      if th > 3 {
        return Err(format!("ljpeg: unsuported table id {}", th).to_string())
      }

      let mut acc: usize = 0;
      if htables[th].initialized {
        return Err("ljpeg: duplicate table def in DHT".to_string())
      }
      for i in 0..16 {
        htables[th].bits[i+1] = input.get_u8() as u32;
        acc += htables[th].bits[i+1] as usize;
      }
      htables[th].bits[0] = 0;

      if acc > 256 {
        return Err("ljpeg: invalid DHT table".to_string())
      }

      if length < 1+16+acc {
        return Err("ljpeg: invalid DHT table length".to_string())
      }

      for i in 0..acc {
        htables[th].huffval[i] = input.get_u8() as u32;
      }

      try!(htables[th].initialize(use_bigtable));
      length -= 1 + 16 + acc;
    }

    Ok(())
  }

  pub fn decode(&self, out: &mut [u16], x: usize, stripwidth: usize, width: usize, height: usize) -> Result<(),String> {
    for component in self.sof.components.iter() {
      if component.super_h !=1 || component.super_v != 1 {
        return Err("ljpeg: subsampled images not supported".to_string());
      }
    }

    match self.predictor {
      1 => {
        match self.sof.cps {
          2 => decode_ljpeg_2components(self, out, x, stripwidth, width, height),
          c => return Err(format!("ljpeg: {} component files not supported", c).to_string()),
        }
      },
      p => return Err(format!("ljpeg: predictor {} not supported", p).to_string()),
    }
  }
}
