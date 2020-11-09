use std::fmt;
use std::cmp;
use crate::decoders::basics::*;

const DECODE_TABLE_BITS: u32 = 11;

pub struct HuffTable {
  // These two fields directly represent the contents of a JPEG DHT marker
  pub bits: [u32;17],
  pub huffval: [u32;256],

  // Represent the weird shifts that are needed for some NEF files
  pub shiftval: [u32;256],

  // The remaining fields are computed from the above to allow more
  // efficient coding and decoding and thus private
  nbits: u32,
  hufftable: Vec<(u16,u16,u16)>,
  decodetable: [Option<(u16,i32)>; 1<< DECODE_TABLE_BITS],
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
      nbits: 0,
      hufftable: Vec::new(),
      decodetable: [None; 1 << DECODE_TABLE_BITS],
      dng_bug: false,
      initialized: false,
    }
  }

  pub fn new(bits: [u32;17], huffval: [u32;256], dng_bug: bool) -> Result<HuffTable,String> {
    let mut tbl = HuffTable {
      bits: bits,
      huffval: huffval,
      shiftval: [0;256],
      nbits: 0,
      hufftable: Vec::new(),
      decodetable: [None; 1 << DECODE_TABLE_BITS],
      dng_bug: dng_bug,
      initialized: false,
    };
    tbl.initialize()?;
    Ok(tbl)
  }

  pub fn initialize(&mut self) -> Result<(), String> {
    // Create the decoding table for the huffman lengths
    self.nbits = 16;
    for i in 0..16 {
      if self.bits[16-i] != 0 {
        break;
      }
      self.nbits -= 1;
    }

    let tblsize = 1 << self.nbits;
    self.hufftable = vec![(0,0,0); tblsize];

    let mut h = 0;
    let mut pos = 0;
    for len in 0..self.nbits {
      for _ in 0..self.bits[len as usize + 1] {
        for _ in 0..(1 << (self.nbits-len-1)) {
          self.hufftable[h] = (len as u16 + 1, self.huffval[pos] as u16, self.shiftval[pos] as u16);
          h += 1;
        }
        pos += 1;
      }
    }

    // Now bootstrap the full decode table
    let mut pump = MockPump::empty();
    let mut i = 0;
    loop {
      pump.set(i, DECODE_TABLE_BITS);
      let (lenbits, totalbits, decode) = self.huff_decode_slow(&mut pump);
      let validbits = pump.validbits();
      if validbits >= 0 {
        // We had a valid decode within the lookup bits, save that result to
        // every position where the decode applies.
        for _ in 0..(1 << validbits) {
          self.decodetable[i as usize] = Some((totalbits, decode));
          i += 1;
          if i >= 1 << DECODE_TABLE_BITS {
            break;
          }
        }
      } else {
        // We had an invalid decode, we can skip as many positions as the ones
        // that have the same bits for length
        i += 1 << cmp::max(0, DECODE_TABLE_BITS - (lenbits as u32));
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
      pump.consume_bits(bits as u32);
      Ok(decode)
    } else {
      let decode = self.huff_decode_slow(pump);
      Ok(decode.2)
    }
  }

  pub fn huff_decode_slow(&self, pump: &mut dyn BitPump) -> (u16, u16,i32) {
    let len = self.huff_len(pump);
    (len.0, len.0+len.1, self.huff_diff(pump, len))
  }

  pub fn huff_len(&self, pump: &mut dyn BitPump) -> (u16,u16,u16) {
    let code = pump.peek_bits(self.nbits) as usize;
    let (bits, len, shift) = self.hufftable[code];
    pump.consume_bits(bits as u32);
    (bits, len, shift)
  }

  pub fn huff_diff(&self, pump: &mut dyn BitPump, input: (u16,u16,u16)) -> i32 {
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
        let fulllen: i32 = len as i32 + shift as i32;
        let shift: i32 = shift as i32;
        let bits = pump.get_bits(len as u32) as i32;
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
