// Lossless JPEG encoder for 1-component
// ITU T.81 Annex H from 1992
//
// Originally written by Andrew Baldwin as lj92.c
// Ported to Rust by Daniel Vogelbacher
//
// (c) 2014 Andrew Baldwin
// (c) 2021 Daniel Vogelbacher
//
// Permission is hereby granted, free of charge, to any person obtaining a copy of
// this software and associated documentation files (the "Software"), to deal in
// the Software without restriction, including without limitation the rights to
// use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies
// of the Software, and to permit persons to whom the Software is furnished to do
// so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

use byteorder::{BigEndian, WriteBytesExt};
use std::{
  cmp::min,
  io::{Cursor, Write},
};
use thiserror::Error;

use crate::{bitarray::BitArray16, inspector};

/// Error variants for compressor
#[derive(Debug, Error)]
pub enum CompressorError {
  /// Overflow of input, size constraints...
  #[error("Overflow error: {}", _0)]
  Overflow(String),

  /// Error on internal cursor type
  #[error("I/O error")]
  Io(#[from] std::io::Error),
}

/// Result type for Compressor results
type Result<T> = std::result::Result<T, CompressorError>;

/// Encoder for Lossless JPEG
///
/// With this type you can get a instance of `LjpegCompressor`.
/// The encode() method consumes the instance and
/// returns the encoded JPEG data.
pub struct LjpegCompressor<'a> {
  /// Raw image input
  image: &'a [u16],
  /// Width of input image
  width: usize,
  /// height of input image
  height: usize,
  /// Number of components (1-4, only 1 is supported)
  components: usize,
  /// Bitdepth of input image
  bitdepth: u8,
  /// Maximum value for given bit depth
  max_value: u16,
  /// Point transformation parameter
  /// **Warning:** This is untested, use with caution
  point_transform: u8,
  /// Predictor
  predictor: u8,
  /// Skip bytes after each line before next line starts
  skip_len: usize,
  /// Component state (histogram, hufftable)
  comp_state: Vec<ComponentState>,
}

/// HUFFENC and HUFFBITS
#[derive(Debug, Default, Clone)]
struct HuffCode {
  enc: u16,
  bits: u16,
}

/// Huffman table builder
///
/// Builds an optimal Huffman table for encoding for a given
/// list of frequencies and total resolution.
#[derive(Default, Debug)]
struct HuffTableBuilder {
  /// Frequency of occurrence of symbol V
  /// Used while building the table. Initialized with the
  /// frequencies for each ssss (0-16).
  /// Reserving one code point guarantees that no code word can ever be all “1” bits.
  freq: [f32; Self::CLASSES + 1],

  /// Code size of symbol V
  /// Size (in bits) for each ssss.
  codesize: [usize; Self::CLASSES + 1],

  /// Index to next symbol in chain of all symbols in current branch of code tree
  /// Other frequencies, used during table buildup.
  others: [Option<usize>; Self::CLASSES + 1],

  /// Numbers of codes of each size
  bits: Vec<u8>,

  /// List of values (ssss) sorted in ascending
  /// code length.
  /// Unused values (at the end of array) are set
  /// to `None`.
  huffval: [Option<u8>; Self::CLASSES],

  /// Code for each symbol
  /// Is is a combination of Huffbits and Huffenc
  huffcode: [HuffCode; Self::CLASSES],

  /// Maps a value (ssss) to a symbol.
  /// This symbol can be used as index into
  /// `huffcode` to get the actual code for encoding.
  huffsym: [Option<usize>; Self::CLASSES],
}

impl HuffTableBuilder {
  /// Count of classes for Lossless JPEG
  /// For regular JPEG, V goes from 0 to 256. For lossless,
  /// we only have 17 classes for ssss (0-16).
  const CLASSES: usize = 17; // Sample classes for Lossless JPEG

  /// Construct new Huffman table for given histogram
  /// and image resolution
  fn new(histogram: [usize; Self::CLASSES], resolution: f32) -> Self {
    let mut ins = Self::default();
    ins.bits.resize(33, 0);
    for (i, freq) in histogram.iter().map(|f| *f as f32 / resolution).enumerate() {
      ins.freq[i] = freq;
    }
    //  Last freq must be 1
    ins.freq[Self::CLASSES] = 1.0;
    ins
  }

  /// Figure K.1 - Procedure to find Huffman code sizes
  fn gen_codesizes(&mut self) {
    loop {
      // smallest frequencies found in llop
      let mut v1freq: f32 = 3.0; // just a value larger then 1.0
      let mut v2freq: f32 = 3.0;
      // Indices into frequency table
      let mut v1: Option<usize> = None;
      let mut v2: Option<usize> = None;
      // Search v1
      for (i, f) in self.freq.iter().enumerate().filter(|(_i, f)| **f > 0.0) {
        if *f <= v1freq {
          v1freq = *f;
          v1 = Some(i);
        }
      }
      // Search v2
      for (i, f) in self.freq.iter().enumerate().filter(|(i, f)| **f > 0.0 && Some(*i) != v1) {
        if *f <= v2freq {
          v2freq = *f;
          v2 = Some(i);
        }
      }

      match (&mut v1, &mut v2) {
        (Some(v1), Some(v2)) => {
          // Combine frequency values
          self.freq[*v1] += self.freq[*v2];
          self.freq[*v2] = 0.0;

          // Increment code sizes for all codewords in this tree branch
          loop {
            self.codesize[*v1] += 1;
            if let Some(other) = self.others[*v1] {
              *v1 = other
            } else {
              break;
            }
          }
          self.others[*v1] = Some(*v2);
          loop {
            self.codesize[*v2] += 1;
            if let Some(other) = self.others[*v2] {
              *v2 = other;
            } else {
              break;
            }
          }
        }
        _ => {
          // exit loop, all frequencies are processed
          break;
        }
      }
    }

    #[cfg(feature = "inspector")]
    for (i, codesize) in self.codesize.iter().enumerate() {
      inspector!("codesize[{}]={}", i, codesize);
    }
  }

  /// Figure K.2 - Procedure to find the number of codes of each size
  fn count_bits(&mut self) {
    // K2
    for i in 0..18 {
      if self.codesize[i] > 0 {
        self.bits[self.codesize[i] as usize] += 1;
      }
    } // end of K2

    self.adjust_bits();
  }

  /// Section K.2 Figure K.4 Sorting of input values according to code size
  /// The input values are sorted according to code size as shown in Figure
  /// K.4.  HUFFVAL is the list containing the input values associated with
  /// each code word, in order of increasing code length.
  ///
  /// At this point, the list of code lengths (BITS) and the list of values
  /// (HUFFVAL) can be used to generate the code tables.  These procedures
  /// are described in Annex C.
  fn sort_input(&mut self) {
    let mut k = 0;
    for i in 1..=32 {
      for j in 0..=16 {
        // ssss
        if self.codesize[j] == i {
          self.huffval[k] = Some(j as u8);
          k += 1;
        }
      }
    }
  }

  /// Section C.2 Figure C.1 Generation of table of Huffman code sizes
  fn gen_size_table(&mut self) -> usize {
    let mut k = 0;
    let mut i = 1;

    while i <= 16 {
      let mut j = 1;
      while j <= self.bits[i] {
        self.huffcode[k].bits = i as u16;
        j += 1;
        k += 1;
      }
      i += 1;
    }
    self.huffcode[k].bits = 0;
    return k;
  }

  /// Section C.2 Figure C.2 Generation of table of Huffman codes
  fn gen_code_table(&mut self) {
    let mut k = 0;
    let mut code = 0;
    let mut si = self.huffcode[0].bits;
    loop {
      loop {
        self.huffcode[k].enc = code;
        code += 1;
        k += 1;
        if self.huffcode[k].bits != si {
          break;
        }
      }
      if self.huffcode[k].bits == 0 {
        break;
      }
      loop {
        code <<= 1;
        si += 1;
        if self.huffcode[k].bits == si {
          break;
        }
      }
    }
  }

  /// Section C.2 Figure C.3 Ordering procedure for encoding code tables
  fn order_codes(&mut self, _lastk: usize) {
    for (i, ssss) in self.huffval.iter().enumerate() {
      if let Some(ssss) = ssss {
        self.huffsym[*ssss as usize] = Some(i);
      }
    }
  }

  /// Section K.2 Figure K.3 Procedure for limiting code lengths to 16 bits
  ///
  /// Figure K.3 gives the procedure for adjusting the BITS list so that no
  /// code is longer than 16 bits.  Since symbols are paired for the
  /// longest Huffman code, the symbols are removed from this length
  /// category two at a time.  The prefix for the pair (which is one bit
  /// shorter) is allocated to one of the pair; then (skipping the BITS
  /// entry for that prefix length) a code word from the next shortest
  /// non-zero BITS entry is converted into a prefix for two code words one
  /// bit longer.  After the BITS list is reduced to a maximum code length
  /// of 16 bits, the last step removes the reserved code point from the
  /// code length count.
  fn adjust_bits(&mut self) {
    let mut i = 32;

    while i > 16 {
      if self.bits[i] > 0 {
        let mut j = i - 2;
        while self.bits[j] == 0 {
          j = j - 1;
        }
        self.bits[i] -= 2;
        self.bits[i - 1] += 1;
        self.bits[j + 1] += 2;
        self.bits[j] -= 1;
      } else {
        i = i - 1;
      }
    }

    while self.bits[i] == 0 {
      i = i - 1;
    }
    self.bits[i] -= 1;
  }

  /// Build Huffman table
  fn build(mut self) -> [BitArray16; HuffTableBuilder::CLASSES] {
    self.gen_codesizes();
    self.count_bits();
    self.sort_input();
    let lastk = self.gen_size_table();
    self.gen_code_table();
    self.order_codes(lastk);
    let mut table = [BitArray16::default(); HuffTableBuilder::CLASSES];

    for ssss in 0..=16 {
      if let Some(code) = self.huffsym[ssss] {
        let enc = &self.huffcode[code];
        inspector!("ssss: {}, enc.bits: {}", ssss, enc.bits);
        table[ssss as usize] = BitArray16::from_lsb(enc.bits as usize, enc.enc);
      }
    }

    #[cfg(feature = "inspector")]
    {
      for (class, val) in self.huffcode.iter().enumerate() {
        inspector!("huffcode: idx: {}, bitlen: {}, {:b}", class, val.bits, val.enc);
      }

      for (class, val) in self.huffval.iter().enumerate() {
        inspector!("huffval: idx: {}, ssss: {:?}", class, val);
      }

      for (idx, bitlen) in self.bits.iter().enumerate() {
        inspector!("bits: codelen: {}, value_count: {}", idx, bitlen);
      }

      for (class, val) in self.huffsym.iter().enumerate() {
        inspector!("hufsym: ssss: {}, code_idx: {:?}", class, val);
      }
    }
    table
  }

  /// This is a manual optimized table for most regular images
  /// Useful for testing only.
  fn _generic_table(histogram: [usize; Self::CLASSES], _resolution: f32) -> [BitArray16; HuffTableBuilder::CLASSES] {
    let mut dist: Vec<(usize, usize)> = histogram.iter().enumerate().map(|(a, b)| (a, b.clone())).collect();
    dist.sort_by(|a, b| b.1.cmp(&a.1));
    #[cfg(feature = "inspector")]
    for (i, f) in &dist {
      inspector!("Freq: {}: {}, {}", i, f, *f as f32 / _resolution);
    }
    let mut table = [BitArray16::default(); HuffTableBuilder::CLASSES];
    table[dist[00].0] = BitArray16::from_lsb(02, 0b00);
    table[dist[01].0] = BitArray16::from_lsb(03, 0b010);
    table[dist[02].0] = BitArray16::from_lsb(03, 0b011);
    table[dist[03].0] = BitArray16::from_lsb(03, 0b100);
    table[dist[04].0] = BitArray16::from_lsb(03, 0b101);
    table[dist[05].0] = BitArray16::from_lsb(03, 0b110);
    table[dist[06].0] = BitArray16::from_lsb(04, 0b1110);
    table[dist[07].0] = BitArray16::from_lsb(05, 0b11110);
    table[dist[08].0] = BitArray16::from_lsb(06, 0b111110);
    table[dist[09].0] = BitArray16::from_lsb(07, 0b1111110);
    table[dist[10].0] = BitArray16::from_lsb(08, 0b11111110);
    table[dist[11].0] = BitArray16::from_lsb(09, 0b111111110);
    table[dist[12].0] = BitArray16::from_lsb(10, 0b1111111110);
    table[dist[13].0] = BitArray16::from_lsb(11, 0b11111111110);
    table[dist[14].0] = BitArray16::from_lsb(12, 0b111111111110);
    table[dist[15].0] = BitArray16::from_lsb(13, 0b1111111111110);
    table[dist[16].0] = BitArray16::from_lsb(14, 0b11111111111110);

    table
  }
}

/// State for one component of the image
#[derive(Default, Clone, Debug)]
struct ComponentState {
  /// Histogram of component
  histogram: [usize; 17],
  /// Huffman table for component
  hufftable: [BitArray16; HuffTableBuilder::CLASSES],
}

/// Bitstream for JPEG encoded data
pub struct BitstreamJPEG<'a> {
  inner: &'a mut dyn Write,
  next: u8,
  used: usize,
}

impl<'a> BitstreamJPEG<'a> {
  pub fn new(inner: &'a mut dyn Write) -> Self {
    Self { inner, next: 0, used: 0 }
  }

  pub fn write_bit(&mut self, value: bool) -> std::io::Result<()> {
    self.write(1, if value { 1 } else { 0 })
  }

  pub fn write(&mut self, mut bits: usize, value: u64) -> std::io::Result<()> {
    while bits > 0 {
      // flush buffer if full
      if self.used == 8 {
        self.internal_flush()?;
      }
      // how many bits are free?
      let free = 8 - self.used;
      // take exactly
      let take = min(bits, free);
      // peeked bits from value
      let peek = ((value >> (bits - take)) & ((1 << take) - 1)) as u8;
      // add peeked bits to buffer
      self.next |= peek << (free - take);
      // reduce consumed bits
      bits -= take;
      self.used += take;
    }
    Ok(())
  }

  fn internal_flush(&mut self) -> std::io::Result<()> {
    self.inner.write_u8(self.next)?;
    if self.next == 0xFF {
      // Byte stuffing
      self.inner.write_u8(0x00)?;
    }
    self.used = 0;
    self.next = 0;
    Ok(())
  }

  pub fn flush(&mut self) -> std::io::Result<()> {
    if self.used > 0 {
      self.internal_flush()?;
    }
    Ok(())
  }
}

impl<'a> LjpegCompressor<'a> {
  /// Create a new LJPEG encoder
  ///
  /// skip_len is given as byte count after a row width.
  pub fn new(image: &'a [u16], width: usize, height: usize, components: usize, bitdepth: u8, predictor: u8, skip_len: usize) -> Result<Self> {
    if !(1..=7).contains(&predictor) {
      return Err(CompressorError::Overflow(format!("Unsupported predictor: {}", predictor)));
    }
    if image.len() < height * (width * components + skip_len) {
      return Err(CompressorError::Overflow(format!(
        "Image input buffer is not large enough for given dimensions"
      )));
    }
    if bitdepth < 2 || bitdepth > 16 {
      return Err(CompressorError::Overflow(format!(
        "Overflow for bit depth {}, only 2 >= bp <= 16 is supported",
        bitdepth
      )));
    }
    if height > 65_535 {
      return Err(CompressorError::Overflow(format!(
        "Overflow for height {}, only h <= 65.535 is supported",
        height
      )));
    }
    if width > 65_535 {
      return Err(CompressorError::Overflow(format!(
        "Overflow for width {}, only h <= 65.535 is supported",
        width
      )));
    }
    let point_transform = 0;
    Ok(Self {
      image,
      width,
      height,
      components,
      bitdepth,
      max_value: ((1u32 << (bitdepth - point_transform)) - 1) as u16,
      point_transform,
      predictor,
      skip_len,
      comp_state: vec![ComponentState::default(); components],
    })
  }

  fn components(&self) -> std::ops::Range<usize> {
    (0..self.components).into_iter()
  }

  /// Encode input data and consume instance
  pub fn encode(mut self) -> Result<Vec<u8>> {
    let mut encoded = Cursor::new(Vec::with_capacity(self.resolution() * self.components));
    self.scan_frequency()?;
    for comp in self.components() {
      self.build_hufftable(comp);
      //self.create_default_table(comp)?;
      //self.create_encode_table(comp)?;
    }

    self.write_header(&mut encoded)?;
    self.write_body(&mut encoded)?;
    self.write_post(&mut encoded)?;
    Ok(encoded.into_inner())
  }

  /// Resolution of input image
  #[inline(always)]
  fn resolution(&self) -> usize {
    self.height * self.width
  }

  fn row_len(&self) -> usize {
    self.width * self.components
  }

  #[inline(always)]
  fn px_ra(current_row: &[u16], col: usize, comp: usize, cc: usize, pt: u8) -> i32 {
    (current_row[((col - 1) * cc) + comp] >> pt) as i32
  }

  #[inline(always)]
  fn px_rb(prev_row: &[u16], _current_row: &[u16], col: usize, comp: usize, cc: usize, pt: u8) -> i32 {
    (prev_row[(col * cc) + comp] >> pt) as i32
  }

  #[inline(always)]
  fn px_rc(prev_row: &[u16], _current_row: &[u16], col: usize, comp: usize, cc: usize, pt: u8) -> i32 {
    (prev_row[((col - 1) * cc) + comp] >> pt) as i32
  }

  /// Predict the Px value
  fn predict_px(&self, prev_row: Option<&[u16]>, current_row: &[u16], row: usize, col: usize, comp: usize) -> i32 {
    let cc = self.components; // component count
    let pt = self.point_transform;
    match (row, col) {
      // First sample
      (0, 0) => (1u16 << (self.bitdepth - pt - 1)) as i32,
      // First row always used Ra prediction
      (0, col) => Self::px_ra(current_row, col, comp, cc, pt),
      // For first column at any row
      (_, 0) => {
        let prev_row = prev_row.expect("Previous row expected");
        Self::px_rb(prev_row, current_row, col, comp, cc, pt)
      }
      // Regular prediction mode
      (row, col) => {
        assert!(row > 0);
        let prev_row = prev_row.expect("Previous row expected");
        match self.predictor {
          1 => Self::px_ra(current_row, col, comp, cc, pt),
          2 => Self::px_rb(prev_row, current_row, col, comp, cc, pt),
          3 => Self::px_rc(prev_row, current_row, col, comp, cc, pt),
          4 => {
            let ra = Self::px_ra(current_row, col, comp, cc, pt);
            let rb = Self::px_rb(prev_row, current_row, col, comp, cc, pt);
            let rc = Self::px_rc(prev_row, current_row, col, comp, cc, pt);
            ra + rb - rc
          }
          5 => {
            let ra = Self::px_ra(current_row, col, comp, cc, pt);
            let rb = Self::px_rb(prev_row, current_row, col, comp, cc, pt);
            let rc = Self::px_rc(prev_row, current_row, col, comp, cc, pt);
            ra + ((rb - rc) >> 1)
          }
          6 => {
            let ra = Self::px_ra(current_row, col, comp, cc, pt);
            let rb = Self::px_rb(prev_row, current_row, col, comp, cc, pt);
            let rc = Self::px_rc(prev_row, current_row, col, comp, cc, pt);
            rb + ((ra - rc) >> 1)
          }
          7 => {
            let ra = Self::px_ra(current_row, col, comp, cc, pt);
            let rb = Self::px_rb(prev_row, current_row, col, comp, cc, pt);
            (ra + rb) / 2
          }
          _ => {
            // We panic here because supported predictor check
            // is done earlier.
            panic!("unsupported predictor")
          }
        }
      }
    }
  }

  /// Scan frequency for Huff table
  fn scan_frequency(&mut self) -> Result<()> {
    let mut prev_row = None;
    for row in 0..self.height {
      let curr_offset = row * (self.row_len() + self.skip_len);
      let current_row = &self.image[curr_offset..curr_offset + self.row_len()];
      if row > 0 {
        let prev_offset = (row - 1) * (self.row_len() + self.skip_len);
        prev_row = Some(&self.image[prev_offset..prev_offset + self.row_len()]);
      }
      for col in 0..self.width {
        for comp in self.components() {
          let sample = current_row[(col * self.components) + comp] >> self.point_transform;
          if sample > self.max_value {
            inspector!("sample: {:#x} max: {:#x}", sample, self.max_value);
            return Err(CompressorError::Overflow(format!(
              "Sample overflow, sample is {:#x} but max value is {:#x}",
              sample, self.max_value
            )));
          }
          let px = self.predict_px(prev_row, current_row, row, col, comp);
          let diff: i32 = sample as i32 - px;
          let ssss = if diff == 0 { 0 } else { 32 - diff.abs().leading_zeros() };
          self.comp_state[comp].histogram[ssss as usize] += 1;
        }
      }
    }
    #[cfg(feature = "inspector")]
    for comp in self.components() {
      inspector!("Huffman table {}", comp);
      for i in 0..17 {
        inspector!("scan: self.huffman[{}].hist[{}]={}", comp, i, self.comp_state[comp].histogram[i]);
      }
    }
    Ok(())
  }

  fn build_hufftable(&mut self, comp: usize) {
    #[cfg(feature = "inspector")]
    {
      let distibution: Vec<(u16, usize)> = self.comp_state[comp].histogram.iter().enumerate().map(|(a, b)| (a as u16, b.clone())).collect();
      for (i, f) in &distibution {
        inspector!("unsorted freq: {}: {}, {}", i, f, *f as f32 / (self.resolution() as f32));
      }
    }

    let huffgen = HuffTableBuilder::new(self.comp_state[comp].histogram.clone(), self.resolution() as f32);
    let table = huffgen.build();
    //let table = HuffTableBuilder::_generic_table(self.huffman[comp].hist.clone());
    #[cfg(feature = "inspector")]
    for (i, code) in table.iter().enumerate() {
      inspector!("table[{}]={}", i, code);
    }
    self.comp_state[comp].hufftable = table;
  }

  /// Write JPEG header
  fn write_header(&mut self, encoded: &mut dyn Write) -> Result<()> {
    encoded.write_u16::<BigEndian>(0xffd8)?; // SOI
    encoded.write_u16::<BigEndian>(0xffc3)?; // SOF_3 Lossless (sequential), Huffman coding

    // Write SOF
    encoded.write_u16::<BigEndian>(2 + 6 + self.components as u16 * 3)?; // Lf, frame header length
    encoded.write_u8(self.bitdepth)?; // Sample precision P
    encoded.write_u16::<BigEndian>(self.height as u16)?;
    encoded.write_u16::<BigEndian>(self.width as u16)?;

    encoded.write_u8(self.components as u8)?; // Components Nf
    for c in self.components() {
      encoded.write_u8(c as u8)?; // Component ID
      encoded.write_u8(0x11)?; // H_i / V_i, Sampling factor 0001 0001
      encoded.write_u8(0)?; // Quantisation table Tq (not used for lossless)
    }

    for comp in self.components() {
      // Write HUFF
      encoded.write_u16::<BigEndian>(0xffc4)?;

      let bit_sum: u16 = self.comp_state[comp].hufftable.iter().filter(|e| e.len() > 0).count() as u16;
      inspector!("Bitsum: {}", bit_sum);

      encoded.write_u16::<BigEndian>(2 + (1 + 16) + bit_sum)?; // Lf, frame header length
      encoded.write_u8(comp as u8)?; // Table ID

      // Write for each of the 16 possible code lengths how many codes
      // exists with the correspoding length.
      for bit_len in 1..=16 {
        let count = self.comp_state[comp].hufftable.iter().filter(|entry| entry.len() == bit_len).count();
        inspector!("COUNT: {}={}", bit_len, count);
        encoded.write_u8(count as u8)?;
      }

      for bit_len in 1..=16 {
        let mut codes: Vec<(u16, BitArray16)> = self.comp_state[comp]
          .hufftable
          .iter()
          .enumerate()
          .filter(|(_, code)| code.len() == bit_len)
          .map(|(ssss, code)| (ssss as u16, code.clone()))
          .collect();
        codes.sort_by(|a, b| a.1.cmp(&b.1));
        for (ssss, _) in codes.iter() {
          encoded.write_u8(*ssss as u8)?;
          inspector!("VAL: {}", ssss);
        }
      }
    }

    // Write SCAN
    encoded.write_u16::<BigEndian>(0xffda)?; // SCAN
    encoded.write_u16::<BigEndian>(0x0006 + (self.components as u16 * 2))?; // Ls, scan header length
    encoded.write_u8(self.components as u8)?; // Ns, Component count
    for c in self.components() {
      encoded.write_u8(c as u8)?; // Cs_i, Component selector
      encoded.write_u8((c as u8) << 4)?; // Td, Ta, DC/AC entropy table selector
    }
    encoded.write_u8(self.predictor)?; // Ss, Predictor for lossless
    encoded.write_u8(0)?; // Se, ignored for lossless
    assert!(self.point_transform <= 15);
    encoded.write_u8(0x00 | (self.point_transform & 0xF))?; // Ah=0, Al=Point transform
    Ok(())
  }

  /// Write JPEG post
  fn write_post(&mut self, encoded: &mut dyn Write) -> Result<()> {
    encoded.write_u16::<BigEndian>(0xffd9)?; // EOI
    Ok(())
  }

  /// Write JPEG body
  fn write_body(&mut self, encoded: &mut dyn Write) -> Result<()> {
    let mut bitstream = BitstreamJPEG::new(encoded);

    let mut prev_row = None;
    for row in 0..self.height {
      let curr_offset = row * (self.row_len() + self.skip_len);
      let current_row = &self.image[curr_offset..curr_offset + self.row_len()];
      if row > 0 {
        let prev_offset = (row - 1) * (self.row_len() + self.skip_len);
        prev_row = Some(&self.image[prev_offset..prev_offset + self.row_len()]);
      }

      for col in 0..self.width {
        for comp in self.components() {
          let sample = current_row[(col * self.components) + comp] >> self.point_transform;
          if sample > self.max_value {
            return Err(CompressorError::Overflow(format!(
              "Sample overflow, sample is {:#x} but max value is {:#x}",
              sample, self.max_value
            )));
          }
          let px = self.predict_px(prev_row, current_row, row, col, comp);
          let mut diff: i32 = sample as i32 - px;
          //inspector!("Sample: {}, px: {}, diff: {}", sample, px, diff);
          let ssss = if diff == 0 { 0 } else { 32 - diff.abs().leading_zeros() };

          let enc = self.comp_state[comp].hufftable[ssss as usize];

          let (bits, value) = (enc.len(), enc.get_lsb() as u64);
          assert!(bits > 0);
          //inspector!("bits: {}, value: {:b}", bits, value);
          bitstream.write(bits, value)?;

          // Sign encoding
          let vt = if ssss > 0 { 1 << (ssss - 1) } else { 0 };
          if diff < vt {
            diff += (1 << ssss) - 1;
          }

          assert!(diff <= (diff & (1 << ssss) - 1));
          //inspector!("diff: {}, ssss: {}", diff, ssss);

          // Write the rest of the bits for the value
          // For ssss == 16 no additional bits are written
          if ssss == 16 {
            // ignore
          } else {
            bitstream.write(ssss as usize, diff as u64)?;
          }
        }
      }
    } // pixel loop

    // Flush the final bits
    bitstream.flush()?;
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use simple_logger::SimpleLogger;

  use super::*;
  /// We reuse the decompressor to check both...
  use crate::decompressors::ljpeg::LjpegDecompressor;

  #[test]
  fn bitstream_test1() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let _ = SimpleLogger::new().init().unwrap_or(());
    let mut buf = Vec::new();
    let mut bs = BitstreamJPEG::new(&mut buf);
    bs.write(1, 0b1)?;
    bs.flush()?;
    bs.write(1, 0b0)?;
    bs.write(3, 0b101)?;
    bs.write(4, 0b11111101)?;
    bs.write(2, 0b101)?;
    bs.flush()?;
    bs.write(16, 0b1111111111111111)?;
    bs.flush()?;
    bs.write(16, 0b0)?;
    bs.flush()?;
    drop(bs);
    assert_eq!(buf[0], 0b10000000);
    assert_eq!(buf[1], 0b01011101);
    assert_eq!(buf[2], 0b01000000);
    assert_eq!(buf[3], 0xFF);
    assert_eq!(buf[4], 0x00); // stuffing
    assert_eq!(buf[5], 0xFF);
    assert_eq!(buf[6], 0x00); // stuffing
    assert_eq!(buf[7], 0x00);
    Ok(())
  }

  #[test]
  fn encode_16x16_16bit_black_decode_single() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let _ = SimpleLogger::new().init().unwrap_or(());
    let h = 16;
    let w = 16;
    let c = 1;
    let input_image = vec![0x0000; w * h * c];
    let enc = LjpegCompressor::new(&input_image, w, h, c, 16, 1, 0)?;
    let result = enc.encode();
    assert!(result.is_ok());
    let jpeg = result?;
    let dec = LjpegDecompressor::new(&jpeg)?;
    let mut outbuf = Vec::with_capacity(h * w);
    outbuf.resize(h * w, 0);
    dec.decode(&mut outbuf, 0, w * c, w * c, h, false)?;
    assert_eq!(outbuf[0], input_image[0]);
    assert_eq!(outbuf[1], input_image[1]);
    for i in 0..outbuf.len() {
      assert_eq!(outbuf[i], input_image[i]);
    }
    Ok(())
  }

  // #[test]
  // fn encode_4x4() -> std::result::Result<(), Box<dyn std::error::Error>> {
  //   let _ = SimpleLogger::new().init().unwrap_or(());
  //   let h = 4;
  //   let w = 4;
  //   let c = 1;
  //   let input_image = [
  //     0x4321, 0xde54, 0x8432, 0xed94, 0xb465, 0x2342, 0xaa02, 0x0054, 0x5487, 0xbb09, 0xe323, 0x9954, 0x8adc, 0x8000, 0x8001, 0xbd09,
  //   ];
  //   let enc = LjpegCompressor::new(&input_image, w, h, c, 16, 1, 0)?;
  //   let result = enc.encode();
  //   assert!(result.is_ok());
  //   let jpeg = result?;
  //   let dec = LjpegDecompressor::new_full(&jpeg, false, false)?;
  //   let mut outbuf = vec![0; w * h * c];
  //   dec.decode(&mut outbuf, 0, w * c, w * c, h, false)?;
  //   assert_eq!(outbuf[0], input_image[0]);
  //   assert_eq!(outbuf[1], input_image[1]);
  //   for i in 0..outbuf.len() {
  //     assert_eq!(outbuf[i], input_image[i]);
  //   }
  //   Ok(())
  // }

  #[test]
  fn encode_16x16_16bit_black_decode_2component() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let _ = SimpleLogger::new().init().unwrap_or(());
    let h = 2;
    let w = 2;
    let c = 2;
    let input_image = vec![0x0000; w * h * c];
    let enc = LjpegCompressor::new(&input_image, w, h, c, 16, 1, 0)?;
    let result = enc.encode();
    assert!(result.is_ok());
    let jpeg = result?;
    let dec = LjpegDecompressor::new_full(&jpeg, false, false)?;
    let mut outbuf = vec![0; w * h * c];
    dec.decode(&mut outbuf, 0, w * c, w * c, h, false)?;
    assert_eq!(outbuf[0], input_image[0]);
    assert_eq!(outbuf[1], input_image[1]);
    for i in 0..outbuf.len() {
      assert_eq!(outbuf[i], input_image[i]);
    }
    Ok(())
  }

  #[test]
  fn encode_16x16_16bit_black() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let _ = SimpleLogger::new().init().unwrap_or(());
    let h = 16;
    let w = 16;
    let c = 1;
    let input_image = vec![0x0000; w * h * c];
    let enc = LjpegCompressor::new(&input_image, w, h, c, 16, 1, 0)?;
    let result = enc.encode();
    assert!(result.is_ok());
    Ok(())
  }

  #[test]
  fn encode_16x16_16bit_white() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let _ = SimpleLogger::new().init().unwrap_or(());
    let h = 16;
    let w = 16;
    let c = 1;
    let input_image = vec![0xffff; w * h * c];
    let enc = LjpegCompressor::new(&input_image, w, h, c, 16, 1, 0)?;
    let result = enc.encode();
    assert!(result.is_ok());
    Ok(())
  }

  #[test]
  fn encode_16x16_bitdepth_error() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let _ = SimpleLogger::new().init().unwrap_or(());
    let h = 16;
    let w = 16;
    let c = 1;
    let input_image = vec![0xfffe; w * h * c];
    let enc = LjpegCompressor::new(&input_image, w, h, c, 14, 1, 0)?;
    let result = enc.encode();
    assert!(result.is_err());
    Ok(())
  }

  #[test]
  fn encode_16x16_short_buffer_error() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let _ = SimpleLogger::new().init().unwrap_or(());
    let h = 16;
    let w = 16;
    let c = 1;
    let input_image = vec![0x9999_u16; (w * h * c) - 1];
    let result = LjpegCompressor::new(&input_image, w, h, c, 16, 1, 0);
    assert!(result.is_err());
    Ok(())
  }

  #[test]
  fn encode_16x16_16bit_incr() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let _ = SimpleLogger::new().init().unwrap_or(());
    let h = 16;
    let w = 16;
    let c = 2;
    let mut input_image = vec![0; h * w * c];
    for i in 0..input_image.len() {
      input_image[i] = i as u16;
    }
    let enc = LjpegCompressor::new(&input_image, w, h, c, 16, 1, 0)?;
    let result = enc.encode();
    assert!(result.is_ok());
    let jpeg = result?;
    let dec = LjpegDecompressor::new_full(&jpeg, false, false)?;
    let mut outbuf = vec![0; h * w * c];
    dec.decode(&mut outbuf, 0, w * c, w * c, h, false)?;
    for i in 0..outbuf.len() {
      assert!((outbuf[i] as i32 - input_image[i] as i32).abs() < 2);
    }
    Ok(())
  }
}
