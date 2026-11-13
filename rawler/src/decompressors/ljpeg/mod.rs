//! Lossless JPEG (LJPEG) decompressor.

use crate::bits::Endian;
use crate::decompressors::Decompressor;
use crate::decompressors::LineIteratorMut;
use crate::decompressors::decompress_strips_fn;
use crate::decompressors::ljpeg::decompressors::*;
use crate::decompressors::ljpeg::huffman::*;
use crate::pixarray::PixU16;
use crate::pumps::ByteStream;

mod decompressors;
pub mod huffman;

/// Decompressor for lossless JPEG (LJPEG)
#[derive(Default)]
pub struct LJpegDecompressor {
  dng_bug: bool,
}

impl LJpegDecompressor {
  pub fn new() -> Self {
    Self::new_with_dng_bug(false)
  }

  /// Create decompressor with optional handling of the 16-bit LJPEG
  /// deviation used by DNG files with DNGVersion < 1.1.0.0.
  pub fn new_with_dng_bug(dng_bug: bool) -> Self {
    Self { dng_bug }
  }
}

impl<'a> Decompressor<'a, u16> for LJpegDecompressor {
  /// Decodes a lossless JPEG buffer into `u16` pixel lines.
  ///
  /// Decodes the entire LJPEG segment into a temporary [`PixU16`] buffer,
  /// then copies the requested rows — starting after `skip_rows` — into
  /// `lines`. The copy uses `line_width` to slice the decoded buffer, so
  /// `line_width` must not exceed the decoded image width.
  ///
  /// # Errors
  /// Returns `Err` if the LJPEG stream cannot be parsed or decoded.
  fn decompress(&self, src: &[u8], skip_rows: usize, lines: impl LineIteratorMut<'a, u16>, line_width: usize) -> std::result::Result<(), String> {
    let decompressor = LjpegDecompressor::new_full(src, self.dng_bug, false)?;
    let mut pixbuf = PixU16::new(decompressor.width(), decompressor.height());

    decompressor.decode(pixbuf.pixels_mut(), 0, decompressor.width(), decompressor.width(), decompressor.height(), false)?;

    for (dst, src) in lines.zip(pixbuf.pixels().chunks_exact(line_width).skip(skip_rows)) {
      dst.copy_from_slice(src);
    }

    Ok(())
  }

  fn can_skip_rows(&self) -> bool {
    false
  }
}

enum Marker {
  Stuff = 0x00,
  SOF3 = 0xc3, // lossless
  DHT = 0xc4,  // huffman tables
  SOI = 0xd8,  // start of image
  EOI = 0xd9,  // end of image
  SOS = 0xda,  // start of scan
  DQT = 0xdb,  // quantization tables
  DRI = 0xdd,  // restart interval
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
    let header_length = input.get_u16()? as usize;
    self.precision = input.get_u8()? as usize;
    self.height = input.get_u16()? as usize;
    self.width = input.get_u16()? as usize;
    self.cps = input.get_u8()? as usize;

    if self.precision > 16 {
      return Err("ljpeg: More than 16 bits per channel is not supported.".to_string());
    }
    if self.cps > 4 || self.cps < 1 {
      return Err("ljpeg: Only from 1 to 4 components are supported.".to_string());
    }
    if header_length != 8 + self.cps * 3 {
      return Err("ljpeg: Header size mismatch.".to_string());
    }

    // The components of an earlier SOF are replaced
    self.components.clear();
    for i in 0..self.cps {
      let id = input.get_u8()? as usize;
      let subs = input.get_u8()? as usize;
      input.get_u8()?; // Skip info about quantized

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
    input.get_u16()?; //skip header length
    let soscps = input.get_u8()? as usize;
    if self.cps != soscps {
      return Err("ljpeg: component number mismatch in SOS".to_string());
    }
    for cs in 0..self.cps {
      // At least some MOS cameras have this broken
      let readcs = input.get_u8()? as usize;
      let cs = if self.csfix { cs } else { readcs };
      let component = match self.components.iter_mut().find(|&&mut c| c.id == cs) {
        Some(val) => val,
        None => return Err(format!("ljpeg: invalid component selector {}", cs)),
      };
      let td = (input.get_u8()? as usize) >> 4;
      if td > 3 {
        return Err("ljpeg: Invalid Huffman table selection".to_string());
      }
      component.dc_tbl_num = td;
    }
    let pred = input.get_u8()? as usize;
    input.get_u8()?; // Se + Ah Not used in LJPEG
    let pt = (input.get_u8()? as usize) & 0xf; // Point Transform
    Ok((pred, pt))
  }
}

#[derive(Debug)]
pub struct LjpegDecompressor<'a> {
  buffer: &'a [u8],
  sof: SOFInfo,
  predictor: usize,
  point_transform: usize,
  restart_interval: usize,
  dhts: Vec<HuffTable>,
}

impl<'a> LjpegDecompressor<'a> {
  pub fn new(src: &'a [u8]) -> Result<LjpegDecompressor<'a>, String> {
    LjpegDecompressor::new_full(src, false, false)
  }

  pub fn new_full(src: &'a [u8], dng_bug: bool, csfix: bool) -> Result<LjpegDecompressor<'a>, String> {
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
    let mut restart_interval = 0;
    loop {
      let marker = LjpegDecompressor::get_next_marker(&mut input, true)?;
      if marker == m(Marker::SOF3) {
        // Start of the frame, giving us the basic info
        sof.parse_sof(&mut input)?;
        if sof.precision > 16 || sof.precision < 10 {
          return Err(format!("ljpeg: sof.precision {}", sof.precision));
        }
      } else if marker == m(Marker::DHT) {
        // Huffman table settings
        LjpegDecompressor::parse_dht(&mut input, &mut dht_init, &mut dht_bits, &mut dht_huffval)?;
      } else if marker == m(Marker::DRI) {
        restart_interval = LjpegDecompressor::parse_dri(&mut input)?;
      } else if marker == m(Marker::SOS) {
        // Start of the actual stream, we can decode after this
        let (a, b) = sof.parse_sos(&mut input)?;
        pred = a;
        pt = b;
        // The base prediction is 2^(precision - point transform - 1)
        if pt >= sof.precision {
          return Err(format!("ljpeg: point transform {} for a precision of {} bits", pt, sof.precision));
        }
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
      "LJPEGDecompressor: super_h: {}, super_v: {}, pred: {}, pt: {}, prec: {}, cps: {}, restart_interval: {}",
      sof.components[0].super_h,
      sof.components[0].super_v,
      pred,
      pt,
      sof.precision,
      sof.cps,
      restart_interval,
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
      restart_interval,
      dhts,
    })
  }

  fn get_next_marker(input: &mut ByteStream, allowskip: bool) -> Result<u8, String> {
    if !allowskip {
      let fill = input.get_u8()?;
      if fill != m(Marker::Fill) {
        return Err(format!("ljpeg get_next_marker() (noskip) expected fill marker 0XFF but got 0x{:X}", fill));
      }
      let mark = input.get_u8()?;
      if mark == m(Marker::Stuff) || mark == m(Marker::Fill) {
        return Err(format!(
          "ljpeg get_next_marker() (noskip) expected marker but found STUFF or FILL (0x{:X})",
          mark
        ));
      }
      return Ok(mark);
    }
    input.skip_to_marker()?;

    Ok(input.get_u8()?)
  }

  fn parse_dht(input: &mut ByteStream, init: &mut [bool; 4], bits: &mut [[u32; 17]; 4], huffval: &mut [[u32; 256]; 4]) -> Result<(), String> {
    let mut length = (input.get_u16()? as usize)
      .checked_sub(2)
      .ok_or_else(|| "ljpeg: invalid DHT length".to_string())?;

    while length > 0 {
      let b = input.get_u8()? as usize;
      let tc = b >> 4;
      let th = b & 0xf;

      if tc != 0 {
        return Err("ljpeg: unsupported table class in DHT".to_string());
      }
      if th > 3 {
        return Err(format!("ljpeg: unsupported table id {}", th));
      }

      let mut acc: usize = 0;
      for i in 0..16 {
        bits[th][i + 1] = input.get_u8()? as u32;
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
        huffval[th][i] = input.get_u8()? as u32;
      }

      init[th] = true;
      length -= 1 + 16 + acc;
    }

    Ok(())
  }

  fn parse_dri(input: &mut ByteStream) -> Result<usize, String> {
    if input.remaining_bytes() < 4 {
      return Err(format!("ljpeg: truncated DRI segment ({} bytes remain)", input.remaining_bytes()));
    }
    let length = input.get_u16()?;
    if length != 4 {
      return Err(format!("ljpeg: invalid DRI length {length}"));
    }
    Ok(input.get_u16()? as usize)
  }

  /// Handle special SONY YUV 4:2:0 encoding in ILCE-7RM5
  pub fn decode_sony(&self, out: &mut [u16], x: usize, stripwidth: usize, width: usize, height: usize, dummy: bool) -> Result<(), String> {
    if dummy {
      return Ok(());
    }
    self.validate_restart_support()?;
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
    self.validate_restart_support()?;

    if self.sof.components[0].super_h == 2 && self.sof.components[0].super_v == 2 {
      decode_ljpeg_420(self, out, width, height)
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
    // The rows have pairs of samples
    if width == 0 || height == 0 || !width.is_multiple_of(2) {
      return Err(format!("MOS: can't decode Leaf data of {}x{} pixels", width, height));
    }
    let mut offsets = vec![0_usize; 1];
    let mut input = ByteStream::new(self.buffer, Endian::Big);

    while let Ok(marker) = LjpegDecompressor::get_next_marker(&mut input, true) {
      if marker == m(Marker::EOI) {
        break;
      }
      offsets.push(input.get_pos());
    }
    let nstrips = height.div_ceil(8);
    if offsets.len() != nstrips {
      return Err(format!("MOS: expecting {} strips found {}", nstrips, offsets.len()));
    }

    let htable1 = self.htable(0)?;
    let htable2 = self.htable(1)?;
    let bpred = 1 << (self.sof.precision - self.point_transform - 1);
    decompress_strips_fn(
      width,
      height,
      8,
      false,
      &(|lines: &mut [u16], strip, _row| {
        let offset = offsets[strip];
        let nlines = lines.len() / width;
        decode_leaf_strip(&self.buffer[offset..], lines, width, nlines, htable1, htable2, bpred)?;
        Ok(())
      }),
    )
  }

  /// The Huffman table of `component`
  fn htable(&self, component: usize) -> Result<&HuffTable, String> {
    let info = self
      .sof
      .components
      .get(component)
      .ok_or_else(|| format!("ljpeg: the image has no component {}", component))?;
    self
      .dhts
      .get(info.dc_tbl_num)
      .filter(|table| table.is_initialized())
      .ok_or_else(|| format!("ljpeg: Huffman table {} of component {} is not defined", info.dc_tbl_num, component))
  }

  fn validate_restart_support(&self) -> Result<(), String> {
    if self.restart_interval == 0 {
      return Ok(());
    }

    if let Some(component) = self.sof.components.iter().find(|component| component.super_h != 1 || component.super_v != 1) {
      return Err(format!(
        "ljpeg: restart markers are not supported for component {} sampling {}x{}",
        component.id, component.super_h, component.super_v
      ));
    }

    if !(1..=7).contains(&self.predictor) {
      return Err(format!("ljpeg: restart markers are not supported with predictor {}", self.predictor));
    }

    if self.sof.width == 0 || !self.restart_interval.is_multiple_of(self.sof.width) {
      return Err(format!(
        "ljpeg: restart interval {} must be a multiple of image width {}",
        self.restart_interval, self.sof.width
      ));
    }

    Ok(())
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
  /// Bits per sample of the frame
  pub fn precision(&self) -> usize {
    self.sof.precision
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Marker segment with its length
  fn segment(marker: u8, payload: &[u8]) -> Vec<u8> {
    let mut data = vec![0xff, marker];
    data.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    data.extend_from_slice(payload);
    data
  }

  /// DHT of table 0 with a single code of 1 bit for `category`
  fn dht(category: u8) -> Vec<u8> {
    let mut payload = vec![0x00, 1];
    payload.extend_from_slice(&[0; 15]);
    payload.push(category);
    segment(0xc4, &payload)
  }

  /// SOF3 of `width`x`height` with the components (id, sampling factors)
  fn sof(precision: u8, width: u16, height: u16, components: &[(u8, u8)]) -> Vec<u8> {
    let mut payload = vec![precision];
    payload.extend_from_slice(&height.to_be_bytes());
    payload.extend_from_slice(&width.to_be_bytes());
    payload.push(components.len() as u8);
    for (id, sampling) in components {
      payload.extend_from_slice(&[*id, *sampling, 0]);
    }
    segment(0xc3, &payload)
  }

  /// SOS of the components (id, Huffman table)
  fn sos(components: &[(u8, u8)], predictor: u8, point_transform: u8) -> Vec<u8> {
    let mut payload = vec![components.len() as u8];
    for (id, table) in components {
      payload.extend_from_slice(&[*id, table << 4]);
    }
    payload.extend_from_slice(&[predictor, 0, point_transform]);
    segment(0xda, &payload)
  }

  /// LJPEG stream of the segments. The entropy data are zero bits, which
  /// table 0 of [`dht`] decodes to differences of `category` 0.
  fn stream(segments: &[Vec<u8>]) -> Vec<u8> {
    let mut data = vec![0xff, 0xd8];
    segments.iter().for_each(|segment| data.extend_from_slice(segment));
    data.extend_from_slice(&[0; 64]);
    data.extend_from_slice(&[0xff, 0xd9]);
    data
  }

  #[test]
  fn stream_of_zero_differences() -> Result<(), String> {
    let data = stream(&[dht(0), sof(16, 4, 2, &[(1, 0x11)]), sos(&[(1, 0)], 1, 0)]);
    let ljpeg = LjpegDecompressor::new(&data)?;
    let mut out = vec![0; 4 * 2];
    ljpeg.decode(&mut out, 0, 4, 4, 2, false)?;
    assert_eq!(out, vec![32768; 4 * 2]);
    Ok(())
  }

  #[test]
  fn dht_length_below_2_is_an_error() {
    let mut data = stream(&[]);
    data.splice(2..2, [0xff, 0xc4, 0x00, 0x01]);
    assert!(LjpegDecompressor::new(&data).is_err());
  }

  #[test]
  fn undefined_tables_are_an_error() -> Result<(), String> {
    let data = stream(&[dht(0), sof(16, 4, 2, &[(1, 0x11)]), sos(&[(1, 1)], 1, 0)]);
    let ljpeg = LjpegDecompressor::new(&data)?;
    let err = ljpeg.decode(&mut [0; 4 * 2], 0, 4, 4, 2, false).err();
    assert!(err.as_ref().is_some_and(|err| err.contains("not defined")), "{:?}", err);
    Ok(())
  }

  #[test]
  fn missing_components_are_an_error() -> Result<(), String> {
    // A single component with the sampling of YUV 4:2:0 and 4:2:2
    for sampling in [0x22, 0x21] {
      let data = stream(&[dht(0), sof(16, 4, 2, &[(1, sampling)]), sos(&[(1, 0)], 1, 0)]);
      let ljpeg = LjpegDecompressor::new(&data)?;
      let err = ljpeg.decode(&mut [0; 12 * 2], 0, 12, 12, 2, false).err();
      assert!(err.as_ref().is_some_and(|err| err.contains("no component 1")), "{:x}: {:?}", sampling, err);
    }
    // Leaf data has two components
    let data = stream(&[dht(0), sof(16, 4, 2, &[(1, 0x11)]), sos(&[(1, 0)], 1, 0)]);
    let err = LjpegDecompressor::new(&data)?.decode_leaf(4, 2).err();
    assert!(err.as_ref().is_some_and(|err| err.contains("no component 1")), "{:?}", err);
    Ok(())
  }

  #[test]
  fn point_transform_must_be_below_precision() {
    for (point_transform, valid) in [(11, true), (12, false), (15, false)] {
      let data = stream(&[dht(0), sof(12, 4, 2, &[(1, 0x11)]), sos(&[(1, 0)], 1, point_transform)]);
      assert_eq!(LjpegDecompressor::new(&data).is_ok(), valid, "point transform {}", point_transform);
    }
  }

  #[test]
  fn second_sof_replaces_the_first() -> Result<(), String> {
    let data = stream(&[
      dht(0),
      sof(16, 4, 2, &[(1, 0x11), (2, 0x11)]),
      sof(16, 4, 2, &[(1, 0x11)]),
      sos(&[(1, 0)], 1, 0),
    ]);
    let ljpeg = LjpegDecompressor::new(&data)?;
    assert_eq!(ljpeg.components(), 1);
    let mut out = vec![0; 4 * 2];
    ljpeg.decode(&mut out, 0, 4, 4, 2, false)?;
    assert_eq!(out, vec![32768; 4 * 2]);
    Ok(())
  }

  #[test]
  fn output_must_hold_the_image() -> Result<(), String> {
    let data = stream(&[dht(0), sof(16, 4, 2, &[(1, 0x11)]), sos(&[(1, 0)], 1, 0)]);
    let ljpeg = LjpegDecompressor::new(&data)?;
    assert!(ljpeg.decode(&mut [0; 4 * 2 - 1], 0, 4, 4, 2, false).is_err());
    assert!(ljpeg.decode(&mut [0; 4 * 2], 0, 4, 4, 0, false).is_err());
    assert!(ljpeg.decode(&mut [0; 4 * 2], 0, 4, 0, 2, false).is_err());
    // A width of 3 samples of 2 components writes 2 pixels of each row
    let data = stream(&[dht(0), sof(16, 2, 2, &[(1, 0x11), (2, 0x11)]), sos(&[(1, 0), (2, 0)], 1, 0)]);
    let ljpeg = LjpegDecompressor::new(&data)?;
    assert!(ljpeg.decode(&mut [0; 3 * 2], 0, 3, 3, 2, false).is_err());
    ljpeg.decode(&mut [0; 3 * 2 + 1], 0, 3, 3, 2, false)?;
    Ok(())
  }

  /// YUV 4:2:0 or 4:2:2 stream with blocks of 2 pixels by `rows`
  fn yuv_stream(category: u8, width: u16, height: u16, rows: u8, point_transform: u8) -> Vec<u8> {
    let components = [(1, 0x20 | rows), (2, 0x11), (3, 0x11)];
    stream(&[
      dht(category),
      sof(16, width, height, &components),
      sos(&[(1, 0), (2, 0), (3, 0)], 1, point_transform),
    ])
  }

  #[test]
  fn yuv_blocks_must_fit_the_image() -> Result<(), String> {
    for (width, height, rows, valid) in [
      (2, 2, 2, true),
      (3, 2, 2, false),
      (2, 3, 2, false),
      (2, 0, 2, false),
      (2, 1, 1, true),
      (3, 1, 1, false),
    ] {
      let ljpeg_data = yuv_stream(0, width, height, rows, 0);
      let ljpeg = LjpegDecompressor::new(&ljpeg_data)?;
      let (width, height) = (width as usize * 3, height as usize);
      let mut out = vec![0; width * height];
      assert_eq!(
        ljpeg.decode(&mut out, 0, width, width, height, false).is_ok(),
        valid,
        "{}x{} in rows of {}",
        width,
        height,
        rows
      );
      if valid {
        assert!(ljpeg.decode(&mut out[1..], 0, width, width, height, false).is_err());
      }
    }
    Ok(())
  }

  #[test]
  fn negative_yuv_samples_wrap() -> Result<(), String> {
    // A base prediction of 1 and differences of -1
    let data = yuv_stream(1, 2, 2, 2, 15);
    let ljpeg = LjpegDecompressor::new(&data)?;
    let mut out = vec![0; 6 * 2];
    ljpeg.decode(&mut out, 0, 6, 6, 2, false)?;
    // Y of the second pixel
    assert_eq!(out[3], 65535);
    Ok(())
  }

  #[test]
  fn hasselblad_width_of_0_is_an_error() -> Result<(), String> {
    let data = stream(&[dht(0), sof(16, 4, 2, &[(1, 0x11)]), sos(&[(1, 0)], 8, 0)]);
    let ljpeg = LjpegDecompressor::new(&data)?;
    assert!(ljpeg.decode(&mut [0; 4 * 2], 0, 0, 0, 2, false).is_err());
    ljpeg.decode(&mut [0; 4 * 2], 0, 4, 4, 2, false)?;
    Ok(())
  }

  #[test]
  fn leaf_size_must_fit_pairs_of_samples() -> Result<(), String> {
    let data = stream(&[dht(0), sof(16, 2, 2, &[(1, 0x11), (2, 0x11)]), sos(&[(1, 0), (2, 0)], 1, 0)]);
    let ljpeg = LjpegDecompressor::new(&data)?;
    for (width, height) in [(4, 0), (0, 2), (3, 2)] {
      assert!(ljpeg.decode_leaf(width, height).is_err(), "{}x{}", width, height);
    }
    assert_eq!(ljpeg.decode_leaf(4, 2)?.pixels(), &[32768; 4 * 2]);
    Ok(())
  }
}
