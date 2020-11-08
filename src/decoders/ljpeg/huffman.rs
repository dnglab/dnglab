/*
* Huffman table generation:
* HuffTable::huff_decode()
* HuffTable::initialize()
* and used data structures are originally grabbed from the IJG software,
* and adapted by Hubert Figuiere.
*
* Copyright (C) 1991, 1992, Thomas G. Lane.
* Part of the Independent JPEG Group's software.
* See the file Copyright for more details.
*
* Copyright (c) 1993 Brian C. Smith, The Regents of the University
* of California
* All rights reserved.
*
* Copyright (c) 1994 Kongji Huang and Brian C. Smith.
* Cornell University
* All rights reserved.
*
* Permission to use, copy, modify, and distribute this software and its
* documentation for any purpose, without fee, and without written agreement is
* hereby granted, provided that the above copyright notice and the following
* two paragraphs appear in all copies of this software.
*
* IN NO EVENT SHALL CORNELL UNIVERSITY BE LIABLE TO ANY PARTY FOR
* DIRECT, INDIRECT, SPECIAL, INCIDENTAL, OR CONSEQUENTIAL DAMAGES ARISING OUT
* OF THE USE OF THIS SOFTWARE AND ITS DOCUMENTATION, EVEN IF CORNELL
* UNIVERSITY HAS BEEN ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
*
* CORNELL UNIVERSITY SPECIFICALLY DISCLAIMS ANY WARRANTIES,
* INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY
* AND FITNESS FOR A PARTICULAR PURPOSE.  THE SOFTWARE PROVIDED HEREUNDER IS
* ON AN "AS IS" BASIS, AND CORNELL UNIVERSITY HAS NO OBLIGATION TO
* PROVIDE MAINTENANCE, SUPPORT, UPDATES, ENHANCEMENTS, OR MODIFICATIONS.
*/

use std::fmt;
use crate::decoders::basics::*;

const DECODE_TABLE_BITS: u32 = 13;
const LENGTH_TABLE_BITS: u32 = 8;

pub struct HuffTable {
  // These two fields directly represent the contents of a JPEG DHT marker
  pub bits: [u32;17],
  pub huffval: [u32;256],

  // Represent the weird shifts that are needed for some NEF files
  pub shiftval: [u32;256],

  // The remaining fields are computed from the above to allow more
  // efficient coding and decoding and thus private
  mincode: [u16;17],
  maxcode: [i32;18],
  valptr: [i16;17],
  lengthtable: Vec<Option<(u32,u32,u32)>>,
  decodetable: Vec<Option<(u32,i32)>>,
  pub dng_bug: bool,
  pub initialized: bool,
}

struct MockPump {
  bits: u64,
  nbits: u32,
}

impl MockPump {
  pub fn empty() -> Self {
    MockPump {
      bits: 0,
      nbits: 0,
    }
  }

  pub fn set(&mut self, bits: u32, nbits: u32) {
    self.bits = (bits as u64) << 32;
    self.nbits = nbits + 32;
  }

  pub fn validbits(&self) -> i32 {
    self.nbits as i32 - 32
  }
}

impl BitPump for MockPump {
  fn peek_bits(&mut self, num: u32) -> u32 {
    (self.bits >> (self.nbits-num)) as u32
  }

  fn consume_bits(&mut self, num: u32) {
    self.nbits -= num;
    self.bits &= (1 << self.nbits) - 1;
  }
}

impl HuffTable {
  pub fn empty() -> HuffTable {
    HuffTable {
      bits: [0;17],
      huffval: [0;256],
      shiftval: [0;256],
      mincode: [0;17],
      maxcode: [0;18],
      valptr: [0;17],
      lengthtable: Vec::new(),
      decodetable: Vec::new(),
      dng_bug: false,
      initialized: false,
    }
  }

  pub fn new(bits: [u32;17], huffval: [u32;256], dng_bug: bool) -> Result<HuffTable,String> {
    let mut tbl = HuffTable {
      bits: bits,
      huffval: huffval,
      shiftval: [0;256],
      mincode: [0;17],
      maxcode: [0;18],
      valptr: [0;17],
      lengthtable: Vec::new(),
      decodetable: Vec::new(),
      dng_bug: dng_bug,
      initialized: false,
    };
    tbl.initialize()?;
    Ok(tbl)
  }

  pub fn initialize(&mut self) -> Result<(), String> {
    // Figure C.1: make table of Huffman code length for each symbol
    // Note that this is in code-length order.
    let mut p = 0;
    let mut huffsize: [u8;257] = [0;257];
    for l in 1..17 {
      for _ in 1..((self.bits[l] as usize)+1) {
        huffsize[p] = l as u8;
        p += 1;
        if p > 256 {
          return Err("ljpeg: Code length too long. Corrupt data.".to_string())
        }
      }
    }
    huffsize[p] = 0;

    // Figure C.2: generate the codes themselves
    // Note that this is in code-length order.
    let mut code: u16 = 0;
    let mut si: u32 = huffsize[0] as u32;
    let mut huffcode: [u16;257] = [0;257];
    p = 0;
    while huffsize[p] > 0 {
      while (huffsize[p] as u32) == si {
        huffcode[p] = code;
        p += 1;
        code += 1;
      }
      code <<= 1;
      si += 1;
      if p > 256 {
        return Err("ljpeg: Code length too long. Corrupt data.".to_string())
      }
    }


    //Figure F.15: generate decoding tables
    self.mincode[0] = 0;
    self.maxcode[0] = 0;
    p = 0;
    for l in 1..17 {
      if self.bits[l] > 0 {
        self.valptr[l] = p as i16;
        self.mincode[l] = huffcode[p];
        p += self.bits[l] as usize;
        self.maxcode[l] = huffcode[p - 1] as i32;
      } else {
        self.valptr[l] = 0xff;   // This check must be present to avoid crash on junk
        self.maxcode[l] = -1;
      }
      if p > 256 {
        return Err("ljpeg: Code length too long. Corrupt data.".to_string())
      }
    }

    // We put in this value to ensure HuffDecode terminates
    self.maxcode[17] = 0xFFFFF;

    // Bootstrap the length table with the slow code
    let mut pump = MockPump::empty();
    self.lengthtable = vec![None; 1 << LENGTH_TABLE_BITS];
    let mut i = 0;
    loop {
      pump.set(i, LENGTH_TABLE_BITS);
      let res = self.huff_len_slow(&mut pump);
      let validbits = pump.validbits();
      if validbits >= 0 {
        // We had a valid decode within the lookup bits, save that result to
        // every position where the decode applies.
        for _ in 0..(1 << validbits) {
          self.lengthtable[i as usize] = Some(res);
          i += 1;
        }
      } else {
        i += 1;
      }
      if i >= 1 << LENGTH_TABLE_BITS {
        break;
      }
    }

    // Now bootstrap the full decode table
    let mut pump = MockPump::empty();
    self.decodetable = vec![None; 1 << DECODE_TABLE_BITS];
    let mut i = 0;
    loop {
      pump.set(i, DECODE_TABLE_BITS);
      let res = self.huff_decode_slow(&mut pump);
      let validbits = pump.validbits();
      if validbits >= 0 {
        // We had a valid decode within the lookup bits, save that result to
        // every position where the decode applies.
        for _ in 0..(1 << validbits) {
          self.decodetable[i as usize] = Some(res);
          i += 1;
        }
      } else {
        i += 1;
      }
      if i >= 1 << DECODE_TABLE_BITS {
        break;
      }
    }

    self.initialized = true;
    Ok(())
  }

  pub fn huff_decode(&self, pump: &mut dyn BitPump) -> Result<i32,String> {
    let code = pump.peek_bits(DECODE_TABLE_BITS) as usize;
    if let Some((bits,decode)) = self.decodetable[code] {
      pump.consume_bits(bits);
      Ok(decode)
    } else {
      let decode = self.huff_decode_slow(pump);
      Ok(decode.1)
    }
  }

  pub fn huff_decode_slow(&self, pump: &mut dyn BitPump) -> (u32,i32) {
    let len = self.huff_len(pump);
    (len.0+len.1, self.huff_diff(pump, len))
  }

  pub fn huff_len(&self, pump: &mut dyn BitPump) -> (u32,u32,u32) {
    let code = pump.peek_bits(LENGTH_TABLE_BITS) as usize;
    if let Some((bits,len,shift)) = self.lengthtable[code] {
      pump.consume_bits(bits);
      (bits, len, shift)
    } else {
      self.huff_len_slow(pump)
    }
  }

  pub fn huff_len_slow(&self, pump: &mut dyn BitPump) -> (u32,u32,u32) {
    let mut code = 0 as u32;
    let mut l = 0 as usize;
    loop {
      let temp = pump.get_bits(1);
      code = (code << 1) | temp;
      l += 1;
      if code as i32 <= self.maxcode[l] {
        break;
      }
    }
    if l >= 17 {
      (0, 0, 0)
    } else {
      let idx = self.valptr[l] as usize + (code as usize - (self.mincode[l] as usize)) as usize;
      (l as u32,self.huffval[idx],self.shiftval[idx])
    }
  }

  pub fn huff_diff(&self, pump: &mut dyn BitPump, input: (u32,u32,u32)) -> i32 {
    let (_, len, shift) = input;

    match len {
      0 => 0,
      16 => {
        if self.dng_bug {
          pump.get_bits(16); // consume can fail because we haven't peeked yet
        }
        -32768
      },
      len => {
        // decode the difference and extend sign bit
        let fulllen: i32 = (len + shift) as i32;
        let shift: i32 = shift as i32;
        let bits = pump.get_bits(len) as i32;
        let mut diff: i32 = ((bits << 1) + 1) << shift >> 1;
        if (diff & (1 << (fulllen - 1))) == 0 {
          diff -= (1 << fulllen) - ((shift == 0) as i32);
        }
        diff
      },
    }
  }
}

impl fmt::Debug for HuffTable {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    if self.initialized {
      write!(f, "HuffTable {{ bits: {:?} huffval: {:?} }}", self.bits, &self.huffval[..])
    } else {
      write!(f, "HuffTable {{ uninitialized }}")
    }
  }
}
