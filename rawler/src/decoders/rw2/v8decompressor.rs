// SPDX-License-Identifier: LGPL-2.1
// Copyright 2024 Daniel Vogelbacher <daniel@chaospixel.com>

// Originally written by LibRaw LLC
// Copyright (C) 2022-2024 Alex Tutubalin, LibRaw LLC
// Ported from C++ to Rust by Daniel Vogelbacher

use itertools::Itertools;
use rayon::iter::{IntoParallelIterator, ParallelIterator};

use crate::{
  alloc_image_ok,
  bits::{Endian, clamp},
  decoders::{Result, rw2::PanasonicTag},
  formats::tiff::IFD,
  pixarray::{PixU16, SharedPix2D},
  pumps::{BitPump, BitPumpReverseBitsMSB, ByteStream},
  rawsource::RawSource,
};

/// Defines the offsets for the start of a strip.
#[derive(Clone, Debug)]
struct StripLineOffset {
  cols: u16,
  rows: u16,
}

#[derive(Clone, Debug)]
struct CF2Params {
  /// Unknown value, it's labeled as strip_height but don't match any height
  #[allow(dead_code)]
  strip_height: u32,

  /// Unknown value, it's labeled as strip_width but don't match any width
  #[allow(dead_code)]
  strip_width: u32,

  /// Gamma point and slope value.
  gamma_point: Vec<u32>,
  gamma_slope: Vec<u32>,

  /// Max data value
  gamma_clip_val: u16,

  /// Initial values (base) for huffman coding
  huf_init_val0: u16,

  /// Initial values (base) for huffman coding
  huf_init_val1: u16,

  /// Initial values (base) for huffman coding
  huf_init_val2: u16,

  /// Initial values (base) for huffman coding
  huf_init_val3: u16,

  /// Stored huffman table, usually 17 entries
  ///
  /// This is a pair of (bitcnt, symbol).
  ///
  /// Example table:
  /// 0000000     10   1022     11   2046      8    254      9    510
  /// 0000020      7    126      4     14      4     12      3      4
  /// 0000040      3      2      2      0      3      3      3      5
  /// 0000060      4     13      5     30      6     62     12   4094
  /// 0000100     12   4095
  /// 0000104
  huf_table: Vec<(u16, u16)>,

  /// Shift down (0 in all samples...)
  huf_shift_down: Vec<u16>,

  /// Number of H strips
  num_of_strips_h: u16,

  /// Number of V strips
  num_of_strips_v: u16,

  /// Offset to bitstream
  strip_byte_offsets: Vec<u32>,

  /// Starting column offset in a output line
  strip_line_offsets: Vec<StripLineOffset>,

  /// Size in bits of compressed bitstream
  strip_data_size: Vec<u32>,

  /// Strip widths in pixels
  strip_widths: Vec<u16>,

  /// Strip heights in pixels
  strip_heights: Vec<u16>,
}

impl CF2Params {
  fn new(ifd: &IFD) -> Result<Self> {
    let strip_height = fetch_tiff_tag!(ifd, PanasonicTag::CF2StripHeight).force_u32(0);
    let gamma_point = {
      let mut bs = ByteStream::new(fetch_tiff_tag!(ifd, PanasonicTag::CF2Unknown1).get_data(), Endian::Little);
      let count = bs.get_u16();
      (0..count).map(|_| bs.get_u32()).collect()
    };
    let gamma_slope = {
      let mut bs = ByteStream::new(fetch_tiff_tag!(ifd, PanasonicTag::CF2Unknown2).get_data(), Endian::Little);
      let count = bs.get_u16();
      (0..count).map(|_| bs.get_u32()).collect()
    };
    let gamma_clip_val = fetch_tiff_tag!(ifd, PanasonicTag::CF2ClipVal).force_u16(0);
    let huf_init_val0 = fetch_tiff_tag!(ifd, PanasonicTag::CF2HufInitVal0).force_u16(0);
    let huf_init_val1 = fetch_tiff_tag!(ifd, PanasonicTag::CF2HufInitVal1).force_u16(0);
    let huf_init_val2 = fetch_tiff_tag!(ifd, PanasonicTag::CF2HufInitVal2).force_u16(0);
    let huf_init_val3 = fetch_tiff_tag!(ifd, PanasonicTag::CF2HufInitVal3).force_u16(0);
    let huf_table = {
      let mut bs = ByteStream::new(fetch_tiff_tag!(ifd, PanasonicTag::CF2HufTable).get_data(), Endian::Little);
      let count = bs.get_u16();
      (0..count).map(|_| (bs.get_u16(), bs.get_u16())).collect()
    };
    let huf_shift_down = {
      let mut bs = ByteStream::new(fetch_tiff_tag!(ifd, PanasonicTag::CF2HufShiftDown).get_data(), Endian::Little);
      let count = bs.get_u16();
      (0..count).map(|_| bs.get_u16()).collect()
    };
    let num_of_strips_h = fetch_tiff_tag!(ifd, PanasonicTag::CF2NumberOfStripsH).force_u16(0);
    let num_of_strips_v = fetch_tiff_tag!(ifd, PanasonicTag::CF2NumberOfStripsV).force_u16(0);
    let strip_byte_offsets = {
      let mut bs = ByteStream::new(fetch_tiff_tag!(ifd, PanasonicTag::CF2StripByteOffsets).get_data(), Endian::Little);
      let count = bs.get_u16();
      (0..count).map(|_| bs.get_u32()).collect()
    };
    let strip_line_offsets = {
      let mut bs = ByteStream::new(fetch_tiff_tag!(ifd, PanasonicTag::CF2StripLineOffsets).get_data(), Endian::Little);
      let count = bs.get_u16();
      (0..count)
        .map(|_| StripLineOffset {
          cols: bs.get_u16(),
          rows: bs.get_u16(),
        })
        .collect()
    };
    let strip_data_size = {
      let mut bs = ByteStream::new(fetch_tiff_tag!(ifd, PanasonicTag::CF2StripDataSize).get_data(), Endian::Little);
      let count = bs.get_u16();
      (0..count).map(|_| bs.get_u32()).collect()
    };
    let strip_widths = {
      let mut bs = ByteStream::new(fetch_tiff_tag!(ifd, PanasonicTag::CF2StripWidths).get_data(), Endian::Little);
      let count = bs.get_u16();
      (0..count).map(|_| bs.get_u16()).collect()
    };
    let strip_heights = {
      let mut bs = ByteStream::new(fetch_tiff_tag!(ifd, PanasonicTag::CF2StripHeights).get_data(), Endian::Little);
      let count = bs.get_u16();
      (0..count).map(|_| bs.get_u16()).collect()
    };
    let strip_width = fetch_tiff_tag!(ifd, PanasonicTag::CF2StripWidth).force_u32(0);

    Ok(Self {
      strip_height,
      gamma_point,
      gamma_slope,
      gamma_clip_val,
      huf_init_val0,
      huf_init_val1,
      huf_init_val2,
      huf_init_val3,
      huf_table,
      huf_shift_down,
      num_of_strips_h,
      num_of_strips_v,
      strip_byte_offsets,
      strip_line_offsets,
      strip_data_size,
      strip_widths,
      strip_heights,
      strip_width,
    })
  }
}

#[derive(Clone, Debug, Default)]
struct HuffmanSymbol {
  /// Length of the symbol in bits
  bitcnt: u8,
  /// Actual Huffman symbol, right-padded with 0 bits
  symbol: u16,
  /// Pre-calculated bitmask, actually (-1) << (16-bits)
  mask: u16,
}

#[derive(Clone, Debug, Default)]
struct HuffmanDecoder {
  #[allow(dead_code)]
  huff_symbols: [HuffmanSymbol; 17],

  /// Lookup cache for all possible u16 values.
  /// If a 16 bit input value is invalid (the symbol is undefined), the
  /// value is None, otherwise it's (bitcnt, ssss)
  cache: Vec<Option<(u8, u8)>>,
}

impl HuffmanDecoder {
  fn new(symlens: impl Iterator<Item = u8>, mut symbols: impl Iterator<Item = u16>) -> Self {
    let mut huff_symbols: [HuffmanSymbol; 17] = Default::default();
    for (i, symlen) in symlens.enumerate() {
      let symbol = symbols.next().expect("symbol iterator is shorter than symlens iterator");
      let bitmask = 0xFFFFu16 >> (16 - symlen);
      debug_assert_eq!(symbol, symbol & bitmask);
      // Left-align symbol and mask
      huff_symbols[i] = HuffmanSymbol {
        bitcnt: symlen,
        symbol: (symbol) << (16 - symlen),
        mask: 0xFFFF << (16 - symlen),
      };
    }

    // Generate lookup cache for all possible 16 bit input values.
    let cache = (0..=0xFFFFu16)
      .map(|x| Self::slow_lookup(&huff_symbols, x).map(|ssss| (huff_symbols[ssss as usize].bitcnt, ssss)))
      .collect_vec();
    Self { huff_symbols, cache }
  }

  /// Slow lookup into Huffman symbol table
  fn slow_lookup(huff_symbols: &[HuffmanSymbol; 17], bits: u16) -> Option<u8> {
    for i in 0..17 {
      if (bits & huff_symbols[i].mask) == huff_symbols[i].symbol {
        return Some(i as u8);
      }
    }
    None
  }

  /// Extract Huffman symbol from bitstream pump and
  /// return index into symbol table.
  fn get_next(&self, pump: &mut dyn BitPump) -> u8 {
    let next_bits = pump.peek_bits(16);
    debug_assert_eq!(self.cache.len(), u16::MAX as usize + 1);
    if let Some((bits, ssss)) = unsafe { *self.cache.get_unchecked(next_bits as usize) } {
      pump.consume_bits(bits as u32);
      ssss
    } else {
      panic!("Input value {:016b} starts not with a valid huffman symbol", next_bits);
    }
  }
}

/// Internal decoder state
struct State {
  huffdec: HuffmanDecoder,
  gamma_table: Option<Vec<u16>>,
  datamax: i32,
  line_base: CoeffBase,
  current_base: CoeffBase,
}

impl State {
  fn new(params: &CF2Params) -> Self {
    let initial = [params.huf_init_val0, params.huf_init_val1, params.huf_init_val2, params.huf_init_val3];
    let gamma_table = make_gammatable(params);

    let line_base = CoeffBase::new([initial[0], initial[1], initial[2], initial[3]]);
    let current_base = line_base;

    let huffdec = HuffmanDecoder::new(
      params.huf_table.iter().map(|(bits, _symbol)| *bits as u8),
      params.huf_table.iter().map(|(_bits, symbol)| *symbol),
    );

    Self {
      huffdec,
      gamma_table,
      datamax: params.gamma_clip_val as i32,
      line_base,
      current_base,
    }
  }
}

#[derive(Clone, Copy, Debug)]
struct CoeffBase {
  coeff: [u16; 4],
}

impl CoeffBase {
  pub fn new(coeff: [u16; 4]) -> Self {
    Self { coeff }
  }

  pub fn update(&mut self, line: &[u16]) {
    self.coeff = [line[0], line[1], line[2], line[3]];
  }
}

fn calc_gamma(params: &CF2Params, idx: u32) -> u16 {
  let gamma_base = 0;
  let gamma_points = &params.gamma_point; // [65536, 65536, 65536, 65536, 65536, 65536]
  let gamma_slopes = &params.gamma_slope; // 0
  let clipping = params.gamma_clip_val;

  let mut x = {
    let mut tmp: u32 = idx | 0xFFFF0000;
    if (idx & 0x10000) == 0 {
      tmp = idx & 0x1FFFF;
    }
    u32::min(gamma_base + tmp, 0xFFFF)
  };

  let mut idx = 0;
  if (x & 0x80000000) != 0 {
    x = 0;
  }

  if x >= (0xFFFF & gamma_slopes[1]) {
    idx = 1;
    if x >= (0xFFFF & gamma_slopes[2]) {
      idx = 2;
      if x >= (0xFFFF & gamma_slopes[3]) {
        idx = 3;
        if x >= (0xFFFF & gamma_slopes[4]) {
          idx = (((x as u64 | 0x5_00000000u64) - (0xFFFF & gamma_slopes[5]) as u64) >> 32) as usize;
        }
      }
    }
  }

  let point = gamma_points[idx];
  let slope = gamma_slopes[idx];
  let mut tmp: u32 = x - (slope & 0xFFFF);
  let result: u16;

  if (point & 0x1F) == 31 {
    result = if idx == 5 { 0xFFFF } else { (gamma_slopes[idx + 1] >> 16) & 0xFFFF } as u16;
    return u16::min(result, clipping);
  }
  if (point & 0x10) == 0 {
    if (point & 0x1F) == 15 {
      let result = ((slope >> 16) & 0xFFFF) as u16;
      return u16::min(result, clipping);
    } else if (point & 0x1F) != 0 {
      tmp = (tmp + (1 << ((point & 0x1F) - 1))) >> (point & 0x1F);
    }
  } else {
    tmp <<= point & 0xF;
  }

  result = (tmp + ((slope >> 16) & 0xFFFF)) as u16;
  u16::min(result, clipping)
}

fn make_gammatable(params: &CF2Params) -> Option<Vec<u16>> {
  const LINEAR_POINTS: [u32; 6] = [65536; 6];
  const LINEAR_SLOPES: [u32; 6] = [0; 6];

  if params.gamma_point == LINEAR_POINTS && params.gamma_slope == LINEAR_SLOPES {
    None
  } else {
    let mut table = vec![0; 0x10000];
    let mut _linear = true;
    for idx in 0..0x10000 {
      table[idx] = calc_gamma(params, idx as u32);
      _linear = _linear && table[idx] == idx as u16;
    }
    Some(table)
  }
}

/// Decode Panasonic V8 bitstreams
pub(crate) fn decode_panasonic_v8(rawfile: &RawSource, width: usize, height: usize, _bps: u32, ifd: &IFD, dummy: bool) -> Result<PixU16> {
  let out = alloc_image_ok!(width, height, dummy);

  let params = CF2Params::new(ifd)?;
  log::debug!("pana8: params: {:?}", params);

  assert_eq!(
    params.strip_widths.iter().map(|x| *x as i32).sum::<i32>() as usize / params.num_of_strips_v as usize,
    width
  );

  // Shared output buffer, we need to write from multiple rayon threads to output image.
  let shared_pix = SharedPix2D::new(out);

  let total_strip_count = (params.num_of_strips_h * params.num_of_strips_v) as usize;
  let mut bitstreams = Vec::with_capacity(total_strip_count);
  for strip_id in 0..total_strip_count {
    bitstreams.push(rawfile.subview(params.strip_byte_offsets[strip_id] as u64, (params.strip_data_size[strip_id] as u64 + 7) / 8)?);
  }

  // Parallel decode multiple strips
  (0..total_strip_count).into_par_iter().for_each(|strip_id| {
    let buf = &bitstreams[strip_id];
    decode_strip(buf, &params, strip_id, unsafe { shared_pix.inner_mut() });
  });
  Ok(shared_pix.into_inner())
}

/// Decode a single strip
fn decode_strip(buf: &[u8], params: &CF2Params, strip_id: usize, out: &mut PixU16) {
  let mut pump = BitPumpReverseBitsMSB::new(buf);
  let width = params.strip_widths[strip_id] as usize;
  let height = params.strip_heights[strip_id] as usize;
  let halfheight = height >> 1;
  let halfwidth = width >> 1;
  let doublewidth = halfwidth * 4;
  let mut linebuf = vec![0_u16; doublewidth];

  // for (i, item) in params.huf_table.iter().enumerate() {
  //   let fmt = format!("{:#032b}", item.1);
  //   log::debug!("Pana8 huf_table {i}: {} (bits: {})", fmt.split_at((32 - item.0) as usize).1, item.0);
  //   //log::debug!("Pana8: huf_table {:02}: {}, {}", i, item.0, item.1);
  // }

  let mut state = State::new(params);

  // Data is encoded in RGRGRG..GBGBGB in a single line (like LJPEG92 4-7 predictors)
  for curr_row in 0..halfheight {
    state.current_base = state.line_base;
    for col in 0..doublewidth {
      // Calculate index
      let ssss = state.huffdec.get_next(&mut pump);

      // Shiftdown seems to be the count of bits shifted to right during encoding.
      // It's 0 for all existing samples so far, highly interested in samples that has
      let shift_down: u8 = (params.huf_shift_down[ssss as usize] & 0x1F) as u8;
      assert_eq!(shift_down, 0, "CF2HufShiftDown samples required");

      // Calculate total required bits to read from bitstream.
      let req_bits: u32 = ssss.saturating_sub(shift_down as u8) as u32;
      let delta1: i32 = if req_bits == 0 {
        0
      } else {
        debug_assert_ne!(req_bits, 0);
        let rawbits: u32 = pump.get_bits(req_bits as u32); // Get additional bits
        let sign = rawbits >> (req_bits - 1); // Get leading sign bit
        let val = (rawbits << (params.huf_shift_down[ssss as usize] & 0xFF)) as i32;

        if sign == 1 {
          val
        } else if ssss > 0 {
          if shift_down != 0 { val + (-1 << ssss) } else { val + (-1 << ssss) + 1 }
        } else {
          0
        }
      };

      let delta2 = if shift_down != 0 { 1 << (shift_down - 1) } else { 0 };
      let delta = delta1 + delta2;

      // For each col iteration, we write to ONE pixel of of 4-pixel group.
      let destpixel = &mut linebuf[col & !0x3..];

      if col & 3 == 2 {
        let val = state.current_base.coeff[1] as i32 + delta;
        destpixel[1] = clamp(val, 0, state.datamax) as u16;
      } else if col & 3 == 1 {
        let val = state.current_base.coeff[2] as i32 + delta;
        destpixel[2] = clamp(val, 0, state.datamax) as u16;
      } else if col & 3 != 0 {
        let val = state.current_base.coeff[3] as i32 + delta;
        destpixel[3] = clamp(val, 0, state.datamax) as u16;
      } else {
        let val = state.current_base.coeff[0] as i32 + delta;
        destpixel[0] = clamp(val, 0, state.datamax) as u16;
      }

      if col & 3 == 3 {
        state.current_base.update(destpixel);
      }
      if col == 3 {
        // base for next line (col == 3 -> first 4 pixels are complete in current row)
        state.line_base.update(&linebuf);
      }
    }

    // Copy line buffer into output image.
    // Line buffer contains two rows packed into one row with double width.
    assert_eq!(linebuf.len(), 2 * width);
    for col in (0..width).step_by(2) {
      let row_offset = params.strip_line_offsets[strip_id].rows as usize;
      let left_margin = params.strip_line_offsets[strip_id].cols as usize;
      let dest_row = row_offset + (curr_row * 2);
      if let Some(gamma_table) = &state.gamma_table {
        debug_assert!(!gamma_table.is_empty());
        *out.at_mut(dest_row + 0, left_margin + col + 0) = gamma_table[linebuf[2 * col + 0] as usize];
        *out.at_mut(dest_row + 0, left_margin + col + 1) = gamma_table[linebuf[2 * col + 1] as usize];
        *out.at_mut(dest_row + 1, left_margin + col + 0) = gamma_table[linebuf[2 * col + 2] as usize];
        *out.at_mut(dest_row + 1, left_margin + col + 1) = gamma_table[linebuf[2 * col + 3] as usize];
      } else {
        *out.at_mut(dest_row + 0, left_margin + col + 0) = linebuf[2 * col + 0];
        *out.at_mut(dest_row + 0, left_margin + col + 1) = linebuf[2 * col + 1];
        *out.at_mut(dest_row + 1, left_margin + col + 0) = linebuf[2 * col + 2];
        *out.at_mut(dest_row + 1, left_margin + col + 1) = linebuf[2 * col + 3];
      }
    }
  }
}
