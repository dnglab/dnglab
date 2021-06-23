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

use std::io::Cursor;
use log::debug;
use byteorder::{BigEndian, WriteBytesExt};
use thiserror::Error;

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
  components: u8,
  /// Bitdepth of input image
  bitdepth: u8,
  /// Maximum value for given bit depth
  max_value: u16,
  /// Skip bytes after each line before next line starts
  skip_len: usize,
  /// Storage for encoded data
  encoded: Cursor<Vec<u8>>,
  /// SSSS frequency histogram
  hist: [usize; 17],
  bits: [u8; 17], // decoder uses u32?
  huffval: [u8; 17], // decoder uses u32?
  huffenc: [u16; 17],
  huffbits: [u16; 17],
  huffsym: [usize; 17],
}

impl<'a> LjpegCompressor<'a> {
  /// Create a new LJPEG encoder
  ///
  /// skip_len is given as byte count after a row width.
  pub fn new(image: &'a [u16], width: usize, height: usize, bitdepth: u8, skip_len: usize) -> Result<Self> {
    if image.len() < height * (width + skip_len) {
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
    Ok(Self {
      image,
      width,
      height,
      components: 1, // TODO: support more components
      bitdepth,
      max_value: ((1u32 << bitdepth) - 1) as u16,
      skip_len,
      // not sure if *3 is required, it was copied from original code
      // but as we have only 1-component code...
      // encoded: Cursor::new(Vec::with_capacity(width * height * 3 + 200)),
      encoded: Cursor::new(Vec::with_capacity(width * height + 200)),
      hist: [0; 17],
      bits: [0; 17],
      huffval: [0; 17],
      huffenc: [0; 17],
      huffbits: [0; 17],
      huffsym: [0; 17],
    })
  }

  /// Encode input data and consume instance
  pub fn encode(mut self) -> Result<Vec<u8>> {
    self.scan_frequency()?;
    self.create_encode_table()?;
    self.write_header()?;
    self.write_body()?;
    self.write_post()?;
    Ok(self.encoded.into_inner())
  }

  /// Resolution of input image
  #[inline(always)]
  fn resolution(&self) -> usize {
    self.height * self.width
  }

  /// Predict the Px value
  /// Only predictor 1 is supported.
  #[inline(always)]
  fn predict_px(&self, prev_row: Option<&[u16]>, current_row: &[u16], row: usize, col: usize, predictor: usize) -> u16 {
    match predictor {
      1 => {
        if row == 0 && col == 0 {
          1u16 << (self.bitdepth - 1)
        } else if col == 0 {
          // prev_row must be Some() here
          prev_row.unwrap()[col]
        } else {
          current_row[col - 1]
        }
      }
      _ => {
        // We panic here because supported predictor check
        // is done earlier.
        panic!("unsupported predictor")
      }
    }
  }

  /// Scan frequency for Huff table
  fn scan_frequency(&mut self) -> Result<()> {
    let mut prev_row = None;
    for row in 0..self.height {
      let curr_offset = row * (self.width + self.skip_len);
      let current_row = &self.image[curr_offset..curr_offset + self.width];
      if row > 0 {
        let prev_offset = (row - 1) * (self.width + self.skip_len);
        prev_row = Some(&self.image[prev_offset..prev_offset + self.width]);
      }
      for col in 0..self.width {
        let sample = current_row[col];
        if sample > self.max_value {
          debug!("sample: {:#x} max: {:#x}", sample, self.max_value);
          return Err(CompressorError::Overflow(format!(
            "Sample overflow, sample is {:#x} but max value is {:#x}",
            sample, self.max_value
          )));
        }
        let px = self.predict_px(prev_row, current_row, row, col, 1);
        let diff: i32 = sample as i32 - px as i32;
        let ssss = if diff == 0 { 0 } else { 32 - diff.abs().leading_zeros() };
        //debug!("ssss: {}", ssss);
        self.hist[ssss as usize] += 1;
      }
    }
    #[cfg(debug_assertions)]
    for i in 0..17 {
      debug!("scan: {}:{}", i, self.hist[i]);
    }
    Ok(())
  }

  /// Create encode table
  fn create_encode_table(&mut self) -> Result<()> {
    let mut freq: [f32; 18] = [0.0; 18];
    let mut codesize: [i32; 18] = [0; 18];
    let mut others: [i32; 18] = [0; 18];

    let totalpixels: f32 = self.resolution() as f32;

    // We fill the first 17 entries, the last one is filled seperately
    for i in 0..17 {
      freq[i] = (self.hist[i] as f32) / totalpixels;
      codesize[i] = 0;
      others[i] = -1;
    }
    freq[17] = 1.0;
    codesize[17] = 0;
    others[17] = -1;

    loop {
      let mut v1f: f32 = 3.0;
      let mut v2f: f32 = 3.0;
      let mut v1: i32 = -1;
      let mut v2: i32 = -1;
      for i in 0..18 {
        if freq[i] <= v1f && freq[i] > 0.0 {
          v1f = freq[i];
          v1 = i as i32;
        }
      }
      for i in 0..18 {
        if i == v1 as usize {
          continue;
        }
        if freq[i] < v2f && freq[i] > 0.0 {
          v2f = freq[i];
          v2 = i as i32;
        }
      }
      if v2 == -1 {
        break;
      } // Done

      freq[v1 as usize] += freq[v2 as usize];
      freq[v2 as usize] = 0.0;

      loop {
        codesize[v1 as usize] += 1;
        if others[v1 as usize] == -1 {
          break;
        }
        v1 = others[v1 as usize];
      }
      others[v1 as usize] = v2;
      loop {
        codesize[v2 as usize] += 1;
        if others[v2 as usize] == -1 {
          break;
        }
        v2 = others[v2 as usize];
      }
    }
    for i in 0..18 {
      if codesize[i] != 0 {
        self.bits[codesize[i] as usize] += 1;
      }
    }

    #[cfg(debug_assertions)]
    for i in 0..17 {
      debug!("bits:{},{},{}", i, self.bits[i], codesize[i]);
    }

    let mut k = 0;
    for i in 1..=32 {
      for j in 0..17 {
        if codesize[j] == i {
          self.huffval[k] = j as u8;
          k += 1;
        }
      }
    }

    #[cfg(debug_assertions)]
    for i in 0..17 {
      debug!("i={},huffval[i]={:#x}", i, self.huffval[i]);
    }

    let mut maxbits = 16;
    while maxbits > 0 {
      if self.bits[maxbits] != 0 {
        break;
      }
      maxbits -= 1;
    }

    let mut bitsused = 1;
    let mut _hv = 0;
    let mut rv = 0;
    let mut vl = 0;
    let mut sym = 0;
    let mut i: u32 = 0;
    while i < (1 << maxbits) {
      if bitsused > maxbits {
        panic!("This should never happen");
      }
      if vl >= self.bits[bitsused] {
        bitsused += 1;
        vl = 0;
        continue;
      }
      if rv == (1 << (maxbits - bitsused)) {
        rv = 0;
        vl += 1;
        _hv += 1;
        continue;
      }
      self.huffbits[sym] = bitsused as u16;
      self.huffenc[sym] = (i >> (maxbits - bitsused)) as u16;
      sym += 1;
      i += 1 << (maxbits - bitsused);
      rv = 1 << (maxbits - bitsused);
    }
    for i in 0..17 {
      if self.huffbits[i] > 0 {
        self.huffsym[self.huffval[i] as usize] = i;
      }
      #[cfg(debug_assertions)]
      debug!(
        "huffval[{}]={},huffenc[{}]={:#x},bits={}",
        i, self.huffval[i], i, self.huffenc[i], self.huffbits[i]
      );
    }

    #[cfg(debug_assertions)]
    for i in 0..17 {
      debug!("huffsym[{}]={}", i, self.huffsym[i]);
    }

    Ok(())
  }

  /// Write JPEG header
  fn write_header(&mut self) -> Result<()> {
    self.encoded.write_u16::<BigEndian>(0xffd8)?; // SOI
    self.encoded.write_u16::<BigEndian>(0xffc3)?; // SOF_3 Lossless (sequential), Huffman coding

    // Write SOF
    self
      .encoded
      .write_u16::<BigEndian>(2 + 6 + self.components as u16 * 3)?; // Lf, frame header length
    self.encoded.write_u8(self.bitdepth)?; // Sample precision P
    self.encoded.write_u16::<BigEndian>(self.height as u16)?;
    self.encoded.write_u16::<BigEndian>(self.width as u16)?;

    self.encoded.write_u8(self.components)?; // Components Nf
    for c in 0..self.components {
      self.encoded.write_u8(c)?; // Component ID
      self.encoded.write_u8(0x11)?; // H_i / V_i, Sampling factor 0001 0001
      self.encoded.write_u8(0)?; // Quantisation table Tq (not used for lossless)
    }

    // Write HUFF
    self.encoded.write_u16::<BigEndian>(0xffc4)?;
    let bit_sum: u16 = self.bits.iter().cloned().map(u16::from).sum();

    debug!("Bitsum: {}", bit_sum);
    self.encoded.write_u16::<BigEndian>(17 + 2 + bit_sum)?; // Lf, frame header length
    self.encoded.write_u8(0)?; // Table ID
    for i in 1..17 {
      self.encoded.write_u8(self.bits[i])?;
    }
    for i in 0..bit_sum as usize {
      self.encoded.write_u8(self.huffval[i])?;
    }
    // Write SCAN
    self.encoded.write_u16::<BigEndian>(0xffda)?; // SCAN
    self
      .encoded
      .write_u16::<BigEndian>(0x0006 + (self.components as u16 * 2))?; // Ls, scan header length
    self.encoded.write_u8(self.components)?; // Ns, Component count

    for c in 0..self.components {
      self.encoded.write_u8(c)?; // Cs_i, Component selector
      self.encoded.write_u8(0)?; // Td, Ta, DC/AC entropy table selector
    }
    self.encoded.write_u8(1)?; // Ss, Predictor for lossless
    self.encoded.write_u8(0)?; // Se, ignored for lossless
    self.encoded.write_u8(0)?; // Ah, ignored for lossless

    Ok(())
  }

  /// Write JPEG post
  fn write_post(&mut self) -> Result<()> {
    self.encoded.write_u16::<BigEndian>(0xffd9)?; // EOI
    Ok(())
  }

  /// Write JPEG body
  fn write_body(&mut self) -> Result<()> {
    let mut bitcount = 0;
    let mut next: u8 = 0;
    let mut nextbits: u16 = 8;
    let mut prev_row = None;
    for row in 0..self.height {
      let curr_offset = row * (self.width + self.skip_len);
      let current_row = &self.image[curr_offset..curr_offset + self.width];
      if row > 0 {
        let prev_offset = (row - 1) * (self.width + self.skip_len);
        prev_row = Some(&self.image[prev_offset..prev_offset + self.width]);
      }
      for col in 0..self.width {
        let sample = current_row[col];
        if sample > self.max_value {
          return Err(CompressorError::Overflow(format!(
            "Sample overflow, sample is {:#x} but max value is {:#x}",
            sample, self.max_value
          )));
        }
        let px = self.predict_px(prev_row, current_row, row, col, 1);
        let mut diff: i32 = sample as i32 - px as i32;
        let mut ssss = if diff == 0 { 0 } else { 32 - diff.abs().leading_zeros() };

        // Write the huffman code for ssss value
        let huffcode = self.huffsym[ssss as usize];
        let mut huffenc = self.huffenc[huffcode ];
        let mut huffbits = self.huffbits[huffcode as usize];
        bitcount = bitcount + huffbits as u32 + ssss as u32;

        let vt = if ssss > 0 { 1 << (ssss - 1) } else { 0 };

        if diff < vt {
          diff += (1 << ssss) - 1;
        }

        // Write the ssss
        while huffbits > 0 {
          let usebits: u16 = if huffbits > nextbits { nextbits } else { huffbits };
          // Add top usebits from huffval to next usebits of nextbits
          let tophuff: i32 = (huffenc >> (huffbits - usebits)) as i32;
          next |= (tophuff << (nextbits - usebits)) as u8; // accept bit loss
          nextbits -= usebits;
          huffbits -= usebits;
          huffenc &= (1 << huffbits) - 1;
          if nextbits == 0 {
            self.encoded.write_u8(next)?;
            if next == 0xff {
              self.encoded.write_u8(0x00)?;
            }
            next = 0;
            nextbits = 8;
          }
        }

        // Write the rest of the bits for the value
        while ssss > 0 {
          assert!(ssss <= u16::MAX as u32);
          let usebits: u16 = if ssss > nextbits as u32 { nextbits } else { ssss as u16 };
          // Add top usebits from huffval to next usebits of nextbits
          let tophuff: i32 = diff >> (ssss - usebits as u32);
          next |= (tophuff << (nextbits - usebits)) as u8; // accept bit loss
          nextbits -= usebits;
          ssss -= usebits as u32;
          diff &= (1 << ssss) - 1;
          if nextbits == 0 {
            self.encoded.write_u8(next)?;
            if next == 0xff {
              self.encoded.write_u8(0x00)?;
            }
            next = 0;
            nextbits = 8;
          }
        }
      }
    } // pixel loop

    // Flush the final bits
    if nextbits < 8 {
      self.encoded.write_u8(next)?;
      if next == 0xff {
        self.encoded.write_u8(0x00)?;
      }
    }

    #[cfg(debug_assertions)]
    {
      for i in 0..17 {
        debug!("{}:{}", i, self.hist[i]);
      }
      debug!("Total bytes: {}", bitcount >> 3);
    }

    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  /// We reuse the decompressor to check both...
  use crate::decompressors::ljpeg::LjpegDecompressor;

  #[test]
  fn encode_16x16_16bit_black_decode() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let h = 16;
    let w = 16;
    let mut input_image = Vec::with_capacity(h * w);
    input_image.resize(h * w, 0x0000_u16);
    let enc = LjpegCompressor::new(&input_image, w, h, 16, 0)?;
    let result = enc.encode();
    assert!(result.is_ok());
    let jpeg = result?;
    let dec = LjpegDecompressor::new(&jpeg)?;
    let mut outbuf = Vec::with_capacity(h * w);
    outbuf.resize(h * w, 0);
    dec.decode(&mut outbuf, 0, w, w, h, false)?;
    for i in 0..outbuf.len() {
      assert_eq!(outbuf[i], input_image[i]);
    }
    Ok(())
  }

  #[test]
  fn encode_16x16_16bit_black() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let h = 16;
    let w = 16;
    let mut input_image = Vec::with_capacity(h * w);
    input_image.resize(h * w, 0x0000_u16);
    let enc = LjpegCompressor::new(&input_image, w, h, 16, 0)?;
    let result = enc.encode();
    assert!(result.is_ok());
    Ok(())
  }

  #[test]
  fn encode_16x16_16bit_white() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let h = 16;
    let w = 16;
    let mut input_image = Vec::with_capacity(h * w);
    input_image.resize(h * w, 0xffff_u16);
    let enc = LjpegCompressor::new(&input_image, w, h, 16, 0)?;
    let result = enc.encode();
    assert!(result.is_ok());
    Ok(())
  }

  #[test]
  fn encode_16x16_bitdepth_error() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let h = 16;
    let w = 16;
    let mut input_image = Vec::with_capacity(h * w);
    input_image.resize(h * w, 0xffff_u16);
    let enc = LjpegCompressor::new(&input_image, w, h, 14, 0)?;
    let result = enc.encode();
    assert!(result.is_err());
    Ok(())
  }

  #[test]
  fn encode_16x16_short_buffer_error() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let h = 16;
    let w = 16;
    let mut input_image = Vec::with_capacity(55); // Wrong size
    input_image.resize(55, 0x9999_u16);
    let result = LjpegCompressor::new(&input_image, w, h, 16, 0);
    assert!(result.is_err());
    Ok(())
  }

  #[test]
  fn encode_16x16_16bit_incr() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let h = 16;
    let w = 16;
    let mut input_image = Vec::with_capacity(h * w);
    input_image.resize(h * w, 0x0000_u16);
    for i in 0..input_image.len() {
      input_image[i] = i as u16;
    }
    let enc = LjpegCompressor::new(&input_image, w, h, 16, 0)?;
    let result = enc.encode();
    assert!(result.is_ok());
    let jpeg = result?;
    let dec = LjpegDecompressor::new(&jpeg)?;
    let mut outbuf = Vec::with_capacity(h * w);
    outbuf.resize(h * w, 0);
    dec.decode(&mut outbuf, 0, w, w, h, false)?;
    for i in 0..outbuf.len() {
      assert!((outbuf[i] as i32 - input_image[i] as i32).abs() < 2);
    }
    Ok(())
  }
}
