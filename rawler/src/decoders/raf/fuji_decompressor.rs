///
/// Original code by libraw and rawspeed, licensed under LGPL-2
///
/// Copyright (C) 2016 Alexey Danilchenko
/// Copyright (C) 2016 Alex Tutubalin
/// Copyright (C) 2017 Uwe Müssel
/// Copyright (C) 2017 Roman Lebedev
/// Copyright (C) 2022 Daniel Vogelbacher
use multiversion::multiversion;
use rayon::prelude::*;
use std::{fmt::Display, mem::size_of};

use crate::{
  CFA, Result,
  bits::{Endian, log2ceil},
  buffer::PaddedBuf,
  cfa::CFAColor,
  imgop::Dim2,
  pixarray::{PixU16, SharedPix2D},
  pumps::{BitPump, BitPumpMSB, ByteStream},
};

/// A single gradient with two points
type Gradient = (i32, i32);

#[derive(Clone, Debug)]
struct Strip {
  offset: usize,
  size: usize,
  n: usize,
  header: Header,
  cfa: [[CFAColor; 6]; 6],
}

/// Quantization table
#[derive(Debug, Clone, Default)]
struct QTable {
  q_base: i32,
  q_table: Vec<i32>,
  max_grad: usize,
  q_gradient_multi: i32,
  raw_bits: usize,
  total_values: i32,
}

#[derive(Debug, Clone)]
struct Params {
  /// Quantization table
  qtables: Vec<QTable>,
  max_bits: usize,
  min_value: i32,
  max_value: i32,
  line_width: usize,
}

#[derive(Copy, Clone, Debug)]
struct ColorPos {
  even: usize,
  odd: usize,
}

#[derive(Clone, Debug)]
struct Colors {
  colors: [ColorPos; 3],
}

#[derive(Clone, Debug)]
struct GradientList {
  /// Gradients for lossless mode
  lossless_grads: Vec<Gradient>, // 41 elements
  /// Gradients for lossy mode
  lossy_grads: [Vec<Gradient>; 3], // 5 elements
}

impl Strip {
  const fn line_height() -> usize {
    6
  }

  // how many vertical lines does this block encode?
  fn height(&self) -> u16 {
    self.header.total_lines
  }

  // how many horizontal pixels does this block encode?
  fn width(&self) -> usize {
    // if this is not the last block, we are good.
    if (self.n + 1) != (self.header.blocks_in_row as usize) {
      return self.header.block_size as usize;
    }
    // ok, this is the last block...
    debug_assert!(self.header.block_size as usize * self.header.blocks_in_row as usize >= self.header.raw_width as usize);
    self.header.raw_width as usize - self.offset_x()
  }

  // where vertically does this block start?
  fn offset_y(&self, line: usize) -> usize {
    debug_assert!(line < (self.height() as usize));
    Self::line_height() as usize * line
  }

  // where horizontally does this block start?
  fn offset_x(&self) -> usize {
    self.header.block_size as usize * self.n
  }
}

fn div_round_up<T>(a: T, b: T) -> T
where
  T: std::ops::Add<Output = T>,
  T: std::ops::Sub<Output = T>,
  T: std::ops::Div<Output = T>,
  T: From<u8>,
  T: Copy,
{
  (a + b - T::from(1_u8)) / b
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct Header {
  signature: u16,
  lossless: u8,
  raw_type: u8,
  raw_bits: u8,
  raw_height: u16,
  raw_rounded_width: u16,
  raw_width: u16,
  block_size: u16,
  blocks_in_row: u8,
  total_lines: u16,
}

impl Header {
  fn is_lossless(&self) -> bool {
    self.lossless == 1
  }

  fn is_valid(&self) -> bool {
    !(self.signature != 0x4953
      //|| self.lossless != 1
      || self.raw_height > 0x3000
      || (self.raw_height as usize) < Strip::line_height()
      || (self.raw_height as usize) % Strip::line_height() != 0
      || self.raw_width > 0x3000
      || self.raw_width < 0x300
      || self.raw_width % 24 != 0
      || self.raw_rounded_width > 0x3000
      || self.block_size != 0x300
      || self.raw_rounded_width < self.block_size
      || self.raw_rounded_width % self.block_size != 0
      || self.raw_rounded_width - self.raw_width >= self.block_size
      || self.blocks_in_row > 0x10
      || self.blocks_in_row == 0
      || self.blocks_in_row as u16 != self.raw_rounded_width / self.block_size
      || self.blocks_in_row as u16 != div_round_up(self.raw_width, self.block_size as u16)
      || self.total_lines > 0x800
      || self.total_lines == 0
      || (self.total_lines as usize) != (self.raw_height as usize) / Strip::line_height()
      || (self.raw_bits != 12 && self.raw_bits != 14 && self.raw_bits != 16)
      || (self.raw_type != 16 && self.raw_type != 0))
  }
}

impl Strip {
  fn decompress_strip(&self, src: &PaddedBuf, header: &Header, params: &Params, q_bases: Option<&[u8]>, out: &mut PixU16) {
    let mut info_block = CompressedBlock::new(header, params);
    log::debug!("Fuji strip offset: {}, len: {}", self.offset, self.size);

    let mut pump = if self.offset + self.size == src.len() {
      BitPumpMSB::new(&src[self.offset..]) // use extra bytes from PaddedBuf
    } else {
      let extra_bytes = 16;
      BitPumpMSB::new(&src[self.offset..self.offset + self.size + extra_bytes])
    };

    let mtable = [
      (XT_LINE_R0, XT_LINE_R3),
      (XT_LINE_R1, XT_LINE_R4),
      (XT_LINE_G0, XT_LINE_G6),
      (XT_LINE_G1, XT_LINE_G7),
      (XT_LINE_B0, XT_LINE_B3),
      (XT_LINE_B1, XT_LINE_B4),
    ];

    let ztable = [(XT_LINE_R2, 3), (XT_LINE_G2, 6), (XT_LINE_B2, 3)];

    let mut params = params.clone();

    for cur_line in 0..self.height() as usize {
      debug_assert_eq!(header.is_lossless(), q_bases.is_none());
      // init grads and main qtable
      if !header.is_lossless() {
        let q_base = q_bases.as_ref().unwrap()[cur_line] as i32;
        if cur_line == 0 || q_base != params.qtables[0].q_base {
          let max_value = (1 << header.raw_bits) - 1; // todo: put into header as function?
          let main_qtable = Params::new_main_qtable(header, max_value, q_base);
          params.qtables[0] = main_qtable;

          // update grads
          // total_values depends on q_base for QTable
          let max_diff = 2.max((params.qtables[0].total_values + 0x20) >> 6);

          for j in 0..3 {
            debug_assert_eq!(info_block.grad_even[j].lossless_grads.len(), 41);
            debug_assert_eq!(info_block.grad_odd[j].lossless_grads.len(), 41);
            for i in 0..41 {
              info_block.grad_even[j].lossless_grads[i].0 = max_diff;
              info_block.grad_even[j].lossless_grads[i].1 = 1;
              info_block.grad_odd[j].lossless_grads[i].0 = max_diff;
              info_block.grad_odd[j].lossless_grads[i].1 = 1;
            }
          }
        }
      }

      if header.raw_type == 16 {
        info_block.fuji_xtrans_decode_block(&mut pump, &params);
      } else {
        info_block.fuji_bayer_decode_block(&mut pump, &params);
      }

      // copy data from line buffers and advance
      for i in mtable.iter() {
        debug_assert!(i.0 < i.1);
        let (dest, src) = info_block.linebuf.split_at_mut(i.0 + 1);
        dest[i.0].copy_from_slice(&src[i.1 - (i.0 + 1)]);
        //info_block.linebuf[i.0] = info_block.linebuf[i.1].clone();
      }

      if header.raw_type == 16 {
        info_block.copy_line_to_xtrans(self, cur_line, out);
      } else {
        info_block.copy_line_to_bayer(self, cur_line, out);
      }

      for i in ztable.iter() {
        // Rest all lines
        for line in i.0..i.0 + i.1 {
          for p in info_block.linebuf[line].iter_mut() {
            *p = 0;
          }
        }
        // Initialize extra pixels
        info_block.linebuf[i.0][0] = info_block.linebuf[i.0 - 1][1];
        info_block.linebuf[i.0][params.line_width + 1] = info_block.linebuf[i.0 - 1][params.line_width];
      }
    }
  }
}

/// We need PaddedBuf here, because the buffer is divided
/// into multiple strips and each strip is feed into a BitPump.
/// Each pump need as litte bit more overhead at the end.
/// For the final strip, we need the extra bytes from PaddedBuf
/// to prevent out-of-range errors in BitPump.
pub(super) fn decompress_fuji(buf: &PaddedBuf, width: usize, height: usize, _bps: usize, corrected_cfa: &CFA) -> Result<PixU16> {
  let mut stream = ByteStream::new(buf, Endian::Big);
  let header = Header {
    signature: stream.get_u16(),
    lossless: stream.get_u8(),
    raw_type: stream.get_u8(),
    raw_bits: stream.get_u8(),
    raw_height: stream.get_u16(),
    raw_rounded_width: stream.get_u16(),
    raw_width: stream.get_u16(),
    block_size: stream.get_u16(),
    blocks_in_row: stream.get_u8(),
    total_lines: stream.get_u16(),
  };
  log::debug!("Header: {:?}", header);
  if !header.is_valid() {
    return Err("Fuji header is not valid".into());
  }
  assert_eq!(
    Dim2::new(width, height),
    Dim2::new(header.raw_width.into(), header.raw_height.into()),
    "RAF header specifies different dimensions!"
  );

  let params = Params::new(&header);
  log::debug!("Params: {:?}", params);

  let mut cfa: [[CFAColor; 6]; 6] = Default::default();
  for i in 0..6 {
    for j in 0..6 {
      let color = corrected_cfa.cfa_color_at(i, j);
      match color {
        CFAColor::RED | CFAColor::GREEN | CFAColor::BLUE => cfa[i][j] = color,
        _ => panic!("Got unexpected color: {:?}", color),
      }
    }
  }

  let block_sizes: Vec<usize> = (0..header.blocks_in_row).map(|_| stream.get_u32() as usize).collect();
  let raw_offset = header.blocks_in_row as usize * size_of::<u32>();
  let raw_offset_padded = (raw_offset + 0xF) & !0xF;
  stream.consume_bytes(raw_offset_padded - raw_offset);

  // Global Q bases for all strips
  let q_bases: Option<Vec<u8>> = if !header.is_lossless() {
    let total_q_bases = block_sizes.len() * ((header.total_lines as usize + 0xF) & !0xF);
    Some(stream.get_bytes(total_q_bases))
  } else {
    None
  };

  //eprintln!("q_bases: {:?}", q_bases);
  //eprintln!("First block: {}", stream.get_pos());

  // calculating raw block offsets
  let strips: Vec<Strip> = block_sizes
    .iter()
    .enumerate()
    .map(|(n, &block_size)| {
      let strip = Strip {
        offset: stream.get_pos(),
        size: block_size,
        n,
        header: header.clone(),
        cfa,
      };
      stream.consume_bytes(block_size);
      strip
    })
    .collect();

  assert!(stream.remaining_bytes() <= 16);

  let out = SharedPix2D::new(PixU16::new(width, height));

  // Process each strip
  strips.par_iter().for_each(|strip| {
    let line_step = (header.total_lines as usize + 0xF) & !0xF;
    // Each strip has it's own q_bases
    let q_bases_strip = q_bases.as_ref().map(|buf| &buf[strip.n * line_step..]);
    // DANGEROUS: We need multiple mut refs here. This should be
    // safe as be only write pixels to pre-allocated memory.
    let outbuf = unsafe { out.inner_mut() };
    strip.decompress_strip(buf, &header, &params, q_bases_strip, outbuf);
  });

  Ok(out.into_inner())
}

impl ColorPos {
  fn new() -> Self {
    Self { even: 0, odd: 1 }
  }

  fn reset(&mut self) {
    self.even = 0;
    self.odd = 1;
  }
}

impl Colors {
  const R: usize = 0;
  const G: usize = 1;
  const B: usize = 2;

  fn new() -> Self {
    Self {
      colors: [ColorPos::new(), ColorPos::new(), ColorPos::new()],
    }
  }

  fn r(&mut self) -> &mut ColorPos {
    &mut self.colors[0]
  }

  fn g(&mut self) -> &mut ColorPos {
    &mut self.colors[1]
  }

  fn b(&mut self) -> &mut ColorPos {
    &mut self.colors[2]
  }

  fn at(&mut self, idx: usize) -> &mut ColorPos {
    &mut self.colors[idx]
  }
}

impl Default for GradientList {
  fn default() -> Self {
    Self {
      lossless_grads: vec![Default::default(); 41],
      lossy_grads: [vec![Default::default(); 5], vec![Default::default(); 5], vec![Default::default(); 5]],
    }
  }
}

/// A compressed block
struct CompressedBlock {
  // tables of gradients
  grad_even: [GradientList; 3],
  grad_odd: [GradientList; 3],
  linebuf: Vec<Vec<u16>>,
}

impl CompressedBlock {
  /// Create and initialize new compression block.
  fn new(header: &Header, params: &Params) -> Self {
    let linebuf = vec![vec![0; params.line_width + 2]; XT_LINE_TOTAL];

    let mut grad_even: [GradientList; 3] = Default::default();
    let mut grad_odd: [GradientList; 3] = Default::default();

    if header.is_lossless() {
      let max_diff = 2.max((params.qtables[0].total_values + 0x20) >> 6) as i32;
      for j in 0..3 {
        debug_assert_eq!(grad_even[j].lossless_grads.len(), 41);
        debug_assert_eq!(grad_odd[j].lossless_grads.len(), 41);
        for i in 0..41 {
          grad_even[j].lossless_grads[i].0 = max_diff;
          grad_even[j].lossless_grads[i].1 = 1;
          grad_odd[j].lossless_grads[i].0 = max_diff;
          grad_odd[j].lossless_grads[i].1 = 1;
        }
      }
    } else {
      // init static grads for lossy only - main ones are done per line
      for k in 0..3 {
        let max_diff = 2.max((params.qtables[k + 1].total_values + 0x20) >> 6) as i32;

        for j in 0..3 {
          for i in 0..5 {
            grad_even[j].lossy_grads[k][i].0 = max_diff;
            grad_even[j].lossy_grads[k][i].1 = 1;
            grad_odd[j].lossy_grads[k][i].0 = max_diff;
            grad_odd[j].lossy_grads[k][i].1 = 1;
          }
        }
      }
    }

    Self { grad_even, grad_odd, linebuf }
  }

  /// Copy line from decoding buffer to output
  fn copy_line<F>(&self, strip: &Strip, cur_line: usize, index_f: F, out: &mut PixU16)
  where
    F: Fn(usize) -> usize,
  {
    let mut line_buf_b = [0; 3];
    let mut line_buf_g = [0; 6];
    let mut line_buf_r = [0; 3];

    for i in 0..3 {
      line_buf_r[i] = XT_LINE_R2 + i;
      line_buf_b[i] = XT_LINE_B2 + i;
    }
    for i in 0..6 {
      line_buf_g[i] = XT_LINE_G2 + i;
    }
    for row_count in 0..Strip::line_height() {
      for pixel_count in 0..strip.width() {
        let line_idx = match strip.cfa[row_count][pixel_count % 6] {
          CFAColor::RED => line_buf_r[row_count >> 1],
          CFAColor::GREEN => line_buf_g[row_count],
          CFAColor::BLUE => line_buf_b[row_count >> 1],
          _ => unreachable!(),
        };
        let p = self.linebuf[line_idx][1 + index_f(pixel_count)];
        *out.at_mut(strip.offset_y(cur_line) + row_count, strip.offset_x() + pixel_count) = p;
      }
    }
  }

  /// Copy line by Bayer pattern
  fn copy_line_to_bayer(&self, strip: &Strip, cur_line: usize, out: &mut PixU16) {
    let index = |pixel_count: usize| -> usize { pixel_count >> 1 };
    self.copy_line(strip, cur_line, index, out);
  }

  /// Copy line by X-Trans pattern
  fn copy_line_to_xtrans(&self, strip: &Strip, cur_line: usize, out: &mut PixU16) {
    let index = |pixel_count: usize| -> usize { (((pixel_count * 2 / 3) & 0x7FFFFFFE) | ((pixel_count % 3) & 1)) + ((pixel_count % 3) >> 1) };
    self.copy_line(strip, cur_line, index, out);
  }

  /// Decode Bayer pattern (RGGB and the like) from block
  fn fuji_bayer_decode_block(&mut self, pump: &mut BitPumpMSB, params: &Params) {
    let line_width = params.line_width;
    let mut colors = Colors::new();

    let pass_red_green = |colors: &mut Colors, pump: &mut BitPumpMSB, block: &mut CompressedBlock, c0: usize, c1: usize, grad: usize| {
      while colors.g().even < line_width || colors.g().odd < line_width {
        if colors.g().even < line_width {
          fuji_decode_sample_even(pump, params, &mut block.linebuf, c0, &mut colors.r().even, &mut block.grad_even[grad]);
          fuji_decode_sample_even(pump, params, &mut block.linebuf, c1, &mut colors.g().even, &mut block.grad_even[grad]);
        }
        if colors.g().even > 8 {
          fuji_decode_sample_odd(pump, params, &mut block.linebuf, c0, &mut colors.r().odd, &mut block.grad_odd[grad]);
          fuji_decode_sample_odd(pump, params, &mut block.linebuf, c1, &mut colors.g().odd, &mut block.grad_odd[grad]);
        }
      }
      block.fuji_extend_red(line_width);
      block.fuji_extend_green(line_width);
    };

    let pass_green_blue = |colors: &mut Colors, pump: &mut BitPumpMSB, block: &mut CompressedBlock, c0: usize, c1: usize, grad: usize| {
      while colors.g().even < line_width || colors.g().odd < line_width {
        if colors.g().even < line_width {
          fuji_decode_sample_even(pump, params, &mut block.linebuf, c0, &mut colors.g().even, &mut block.grad_even[grad]);
          fuji_decode_sample_even(pump, params, &mut block.linebuf, c1, &mut colors.b().even, &mut block.grad_even[grad]);
        }
        if colors.g().even > 8 {
          fuji_decode_sample_odd(pump, params, &mut block.linebuf, c0, &mut colors.g().odd, &mut block.grad_odd[grad]);
          fuji_decode_sample_odd(pump, params, &mut block.linebuf, c1, &mut colors.b().odd, &mut block.grad_odd[grad]);
        }
      }
      block.fuji_extend_green(line_width);
      block.fuji_extend_blue(line_width);
    };

    pass_red_green(&mut colors, pump, self, XT_LINE_R2, XT_LINE_G2, 0);
    colors.g().reset();

    pass_green_blue(&mut colors, pump, self, XT_LINE_G3, XT_LINE_B2, 1);
    colors.r().reset();
    colors.g().reset();

    pass_red_green(&mut colors, pump, self, XT_LINE_R3, XT_LINE_G4, 2);
    colors.g().reset();
    colors.b().reset();

    pass_green_blue(&mut colors, pump, self, XT_LINE_G5, XT_LINE_B3, 0);
    colors.r().reset();
    colors.g().reset();

    pass_red_green(&mut colors, pump, self, XT_LINE_R4, XT_LINE_G6, 1);
    colors.g().reset();
    colors.b().reset();

    pass_green_blue(&mut colors, pump, self, XT_LINE_G7, XT_LINE_B4, 2);
  }

  /// A single X-Trans decoding pass for the given control colors C0 and C1
  fn fuji_xtrans_pass<F, const C0: usize, const C1: usize>(
    &mut self,
    params: &Params,
    colors: &mut Colors,
    pump: &mut BitPumpMSB,
    c0: usize,
    c1: usize,
    grad: usize,
    even_func: F,
  ) where
    F: Fn(&mut CompressedBlock, &mut BitPumpMSB, usize, usize, usize, &mut ColorPos, &mut ColorPos),
  {
    let line_width = params.line_width;
    while colors.g().even < line_width || colors.g().odd < line_width {
      if colors.g().even < line_width {
        let mut c0_pos = *colors.at(C0);
        let mut c1_pos = *colors.at(C1);
        even_func(self, pump, c0, c1, grad, &mut c0_pos, &mut c1_pos);
        *colors.at(C0) = c0_pos; // Write back
        *colors.at(C1) = c1_pos;
      }
      if colors.g().even > 8 {
        fuji_decode_sample_odd(pump, params, &mut self.linebuf, c0, &mut colors.at(C0).odd, &mut self.grad_odd[grad]);
        fuji_decode_sample_odd(pump, params, &mut self.linebuf, c1, &mut colors.at(C1).odd, &mut self.grad_odd[grad]);
      }
    }
  }

  /// Decode X-Trans pattern from block
  fn fuji_xtrans_decode_block(&mut self, pump: &mut BitPumpMSB, params: &Params) {
    let mut colors = Colors::new();
    let line_width = params.line_width;

    // Pass 1
    self.fuji_xtrans_pass::<_, { Colors::R }, { Colors::G }>(
      params,
      &mut colors,
      pump,
      XT_LINE_R2,
      XT_LINE_G2,
      0,
      |block, pump, c0, c1, grad, c0_pos, c1_pos| {
        fuji_decode_interpolation_even(block, c0, &mut c0_pos.even);
        fuji_decode_sample_even(pump, params, &mut block.linebuf, c1, &mut c1_pos.even, &mut block.grad_even[grad]);
      },
    );
    self.fuji_extend_red(line_width);
    self.fuji_extend_green(line_width);
    colors.g().reset();

    // Pass 2
    self.fuji_xtrans_pass::<_, { Colors::G }, { Colors::B }>(
      params,
      &mut colors,
      pump,
      XT_LINE_G3,
      XT_LINE_B2,
      1,
      |block, pump, c0, c1, grad, c0_pos, c1_pos| {
        fuji_decode_sample_even(pump, params, &mut block.linebuf, c0, &mut c0_pos.even, &mut block.grad_even[grad]);
        fuji_decode_interpolation_even(block, c1, &mut c1_pos.even);
      },
    );
    self.fuji_extend_green(line_width);
    self.fuji_extend_blue(line_width);
    colors.r().reset();
    colors.g().reset();

    // Pass 3
    self.fuji_xtrans_pass::<_, { Colors::R }, { Colors::G }>(
      params,
      &mut colors,
      pump,
      XT_LINE_R3,
      XT_LINE_G4,
      2,
      |block, pump, c0, c1, grad, c0_pos, c1_pos| {
        if c0_pos.even & 3 != 0 {
          fuji_decode_sample_even(pump, params, &mut block.linebuf, c0, &mut c0_pos.even, &mut block.grad_even[grad]);
        } else {
          fuji_decode_interpolation_even(block, c0, &mut c0_pos.even);
        }
        fuji_decode_interpolation_even(block, c1, &mut c1_pos.even);
      },
    );
    self.fuji_extend_red(line_width);
    self.fuji_extend_green(line_width);
    colors.g().reset();
    colors.b().reset();

    // Pass 4
    self.fuji_xtrans_pass::<_, { Colors::G }, { Colors::B }>(
      params,
      &mut colors,
      pump,
      XT_LINE_G5,
      XT_LINE_B3,
      0,
      |block, pump, c0, c1, grad, c0_pos, c1_pos| {
        fuji_decode_sample_even(pump, params, &mut block.linebuf, c0, &mut c0_pos.even, &mut block.grad_even[grad]);

        if c1_pos.even & 3 == 2 {
          fuji_decode_interpolation_even(block, c1, &mut c1_pos.even);
        } else {
          fuji_decode_sample_even(pump, params, &mut block.linebuf, c1, &mut c1_pos.even, &mut block.grad_even[grad]);
        }
      },
    );
    self.fuji_extend_green(line_width);
    self.fuji_extend_blue(line_width);
    colors.r().reset();
    colors.g().reset();

    // Pass 5
    self.fuji_xtrans_pass::<_, { Colors::R }, { Colors::G }>(
      params,
      &mut colors,
      pump,
      XT_LINE_R4,
      XT_LINE_G6,
      1,
      |block, pump, c0, c1, grad, c0_pos, c1_pos| {
        if c0_pos.even & 3 == 2 {
          fuji_decode_interpolation_even(block, c0, &mut c0_pos.even);
        } else {
          fuji_decode_sample_even(pump, params, &mut block.linebuf, c0, &mut c0_pos.even, &mut block.grad_even[grad]);
        }

        fuji_decode_sample_even(pump, params, &mut block.linebuf, c1, &mut c1_pos.even, &mut block.grad_even[grad]);
      },
    );
    self.fuji_extend_red(line_width);
    self.fuji_extend_green(line_width);
    colors.g().reset();
    colors.b().reset();

    // Pass 6
    self.fuji_xtrans_pass::<_, { Colors::G }, { Colors::B }>(
      params,
      &mut colors,
      pump,
      XT_LINE_G7,
      XT_LINE_B4,
      2,
      |block, pump, c0, c1, grad, c0_pos, c1_pos| {
        fuji_decode_interpolation_even(block, c0, &mut c0_pos.even);

        if c1_pos.even & 3 != 0 {
          fuji_decode_sample_even(pump, params, &mut block.linebuf, c1, &mut c1_pos.even, &mut block.grad_even[grad]);
        } else {
          fuji_decode_interpolation_even(block, c1, &mut c1_pos.even);
        }
      },
    );
    self.fuji_extend_green(line_width);
    self.fuji_extend_blue(line_width);
  }

  fn fuji_extend_generic(&mut self, line_width: usize, start: usize, end: usize) {
    debug_assert!(start > 0);
    for i in start..=end {
      self.linebuf[i][0] = self.linebuf[i - 1][1];
      self.linebuf[i][line_width + 1] = self.linebuf[i - 1][line_width];
    }
  }

  fn fuji_extend_red(&mut self, line_width: usize) {
    self.fuji_extend_generic(line_width, XT_LINE_R2, XT_LINE_R4)
  }

  fn fuji_extend_green(&mut self, line_width: usize) {
    self.fuji_extend_generic(line_width, XT_LINE_G2, XT_LINE_G7)
  }

  fn fuji_extend_blue(&mut self, line_width: usize) {
    self.fuji_extend_generic(line_width, XT_LINE_B2, XT_LINE_B4)
  }
}

impl Display for QTable {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_fmt(format_args!(
      "QTable q_base {}, max_grad {}, q_gradient_multi {}, raw_bits {}, total_values {}",
      self.q_base, self.max_grad, self.q_gradient_multi, self.raw_bits, self.total_values
    ))
  }
}

impl QTable {
  /// Lookup gradient in q_table. The absolute value of this
  /// is used as an index into the gradients vector.
  fn lookup_gradient(&self, params: &Params, v1: i32, v2: i32) -> i32 {
    self.q_gradient_multi * self.q_table[(params.max_value + (v1)) as usize] + self.q_table[(params.max_value + (v2)) as usize]
  }

  /// Build a quantization table based on 5 quantization points.
  fn build_table(header: &Header, qp: &[i32; 5]) -> Vec<i32> {
    let mut qtable = vec![0; 2 * (1 << (header.raw_bits as usize))];
    let mut cur_val = -(qp[4] as i32);

    for qt in qtable.iter_mut() {
      if cur_val > qp[4] as i32 {
        break;
      }
      *qt = match cur_val {
        x if x <= -qp[3] => -4,
        x if x <= -qp[2] => -3,
        x if x <= -qp[1] => -2,
        x if x < -qp[0] => -1,
        x if x <= qp[0] => 0,
        x if x < qp[1] => 1,
        x if x < qp[2] => 2,
        x if x < qp[3] => 3,
        _ => 4,
      };
      cur_val += 1;
    }
    qtable
  }
}

impl Params {
  /// Construct new main quantization table.
  fn new_main_qtable(header: &Header, max_value: i32, q_base: i32) -> QTable {
    let mut qp = [0; 5];
    qp[0] = q_base;
    qp[1] = 3 * q_base + 0x12;
    qp[2] = 5 * q_base + 0x43;
    qp[3] = 7 * q_base + 0x114;
    qp[4] = max_value;

    let max_val = max_value + 1;
    if qp[1] >= max_val || qp[1] < q_base + 1 {
      qp[1] = q_base + 1;
    }
    if qp[2] < qp[1] || qp[2] >= max_val {
      qp[2] = qp[1];
    }
    if qp[3] < qp[2] || qp[3] >= max_val {
      qp[3] = qp[2];
    }

    let q_table = QTable::build_table(header, &qp);
    let total_values = (qp[4] + 2 * q_base) / (2 * q_base + 1) + 1;
    let raw_bits: usize = log2ceil(total_values as usize);

    QTable {
      q_base,
      q_table,
      q_gradient_multi: 9,
      max_grad: 0,
      raw_bits,
      total_values,
    }
  }

  /// Create new parameter
  fn new(header: &Header) -> Self {
    if (header.block_size % 3 != 0 && header.raw_type == 16) || (header.block_size & 1 != 0 && header.raw_type == 0) {
      panic!("Invalid FUJI header");
    }
    let min_value = 0x40;
    let max_value = ((1 << header.raw_bits) - 1) as i32;
    let max_bits: usize = 4 * log2ceil(max_value as usize + 1);
    let line_width = if header.raw_type == 16 {
      (header.block_size as usize * 2) / 3
    } else {
      header.block_size as usize >> 1
    };

    // Build quantization tables.
    // For lossless, only one table is required.
    // For lossy, the main table is created on each iteration
    // while 3 static extra tables are required.
    let qtables = if header.is_lossless() {
      // Only a single table is needed for lossless mode
      let q_base = 0;
      let main_qtable = Self::new_main_qtable(header, max_value, q_base);
      vec![main_qtable]
    } else {
      let mut qtables = vec![QTable::default(); 4];

      // The main table is left uninitialized here as
      // the table is setup for each iteration.
      qtables[0].q_base = -1;

      let mut qp = [0_i32; 5];
      qp[4] = max_value; // identical for all tables

      // table 0
      qtables[1].q_base = 0;
      qtables[1].max_grad = 5;
      qtables[1].q_gradient_multi = 3;
      qtables[1].total_values = qp[4] + 1;
      qtables[1].raw_bits = log2ceil(qtables[1].total_values as usize);
      qp[0] = qtables[1].q_base;
      qp[1] = if qp[4] >= 0x12 { 0x12 } else { qp[0] + 1 };
      qp[2] = if qp[4] >= 0x43 { 0x43 } else { qp[1] };
      qp[3] = if qp[4] >= 0x114 { 0x114 } else { qp[2] };
      qtables[1].q_table = QTable::build_table(header, &qp);

      // table 1
      qtables[2].q_base = 1;
      qtables[2].max_grad = 6;
      qtables[2].q_gradient_multi = 3;
      qtables[2].total_values = (qp[4] + 2) / 3 + 1;
      qtables[2].raw_bits = log2ceil(qtables[2].total_values as usize);
      qp[0] = qtables[2].q_base;
      qp[1] = if qp[4] >= 0x15 { 0x15 } else { qp[0] + 1 };
      qp[2] = if qp[4] >= 0x48 { 0x48 } else { qp[1] };
      qp[3] = if qp[4] >= 0x11B { 0x11B } else { qp[2] };
      qtables[2].q_table = QTable::build_table(header, &qp);

      // table 2
      qtables[3].q_base = 2;
      qtables[3].max_grad = 7;
      qtables[3].q_gradient_multi = 3;
      qtables[3].total_values = (qp[4] + 4) / 5 + 1;
      qtables[3].raw_bits = log2ceil(qtables[3].total_values as usize);
      qp[0] = qtables[3].q_base;
      qp[1] = if qp[4] >= 0x18 { 0x18 } else { qp[0] + 1 };
      qp[2] = if qp[4] >= 0x4D { 0x4D } else { qp[1] };
      qp[3] = if qp[4] >= 0x122 { 0x122 } else { qp[2] };
      qtables[3].q_table = QTable::build_table(header, &qp);

      qtables
    };

    Self {
      qtables,
      max_bits,
      min_value,
      max_value,
      line_width,
    }
  }
}

/// Count and consume all zero bits
/// Additionally, consume the first 1 bit.
#[inline(always)]
fn fuji_zerobits(pump: &mut BitPumpMSB) -> u32 {
  let count = pump.consume_zerobits();
  debug_assert_eq!(pump.peek_bits(1), 1);
  pump.consume_bits(1); // consume the next bit which is 0b1
  count
}

/// Calculate bit difference between two values
fn bit_diff(v1: i32, v2: i32) -> u32 {
  if v2 >= v1 {
    0
  } else {
    let mut dec_bits = 0;
    while dec_bits <= 14 {
      dec_bits += 1;
      if (v2 << dec_bits) >= v1 {
        return dec_bits;
      }
    }
    dec_bits
  }
}

/// Read a single code from bitstream and ajust gradient.
/// We use bmi1 feature here because it provides LZCNT for
/// leading zero count which is used here a lot.
#[multiversion(targets("x86_64+avx+avx2+fma+bmi1+bmi2", "x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn read_code(pump: &mut BitPumpMSB, params: &Params, gradient: &mut Gradient, q_table: &QTable) -> i32 {
  let sample = fuji_zerobits(pump);
  let mut code: i32 = if (sample as usize) < params.max_bits - q_table.raw_bits - 1 {
    let dec_bits = bit_diff(gradient.0, gradient.1);
    let extra_bits = if dec_bits == 0 { 0 } else { pump.get_bits(dec_bits) as i32 };
    (sample << dec_bits) as i32 + extra_bits
  } else {
    1 + pump.get_bits(q_table.raw_bits as u32) as i32
  };
  // Validate code
  if code < 0 || code >= q_table.total_values as i32 {
    panic!("Invalid code: {}", code);
  }
  // Adjust code
  if (code & 1) != 0 {
    code = -1 - code / 2;
  } else {
    code /= 2;
  }
  // Update gradient
  gradient.0 += code.abs();
  if gradient.1 == params.min_value {
    gradient.0 >>= 1;
    gradient.1 >>= 1;
  }
  gradient.1 += 1;
  code
}

/// Decode samples for even positions
fn fuji_decode_sample_even(pump: &mut BitPumpMSB, params: &Params, linebuf: &mut [Vec<u16>], line: usize, pos: &mut usize, grads: &mut GradientList) {
  // Line -2 |   | f |   |
  // Line -1 | c | b | d |
  // Line  0 | a | x | g |
  let rb = linebuf[line - 1][1 + *pos + 0] as i32;
  let rc = linebuf[line - 1][1 + *pos - 1] as i32;
  let rd = linebuf[line - 1][1 + *pos + 1] as i32;
  let rf = linebuf[line - 2][1 + *pos + 0] as i32;
  // Calculate horiz/vert. gradients around current sample x
  let diff_rc_rb = (rc - rb).abs();
  let diff_rf_rb = (rf - rb).abs();
  let diff_rd_rb = (rd - rb).abs();
  // Quantization table and Gradients to use
  let mut qtable = &params.qtables[0];
  let mut gradients = &mut grads.lossless_grads;
  for i in 1..4 {
    if params.qtables[0].q_base < i as i32 {
      break;
    }
    if diff_rf_rb + diff_rc_rb <= params.qtables[i].max_grad as i32 {
      qtable = &params.qtables[i];
      gradients = &mut grads.lossy_grads[i - 1];
      break;
    }
  }
  // Determine gradient
  let grad = qtable.lookup_gradient(params, rb - rf, rc - rb);

  let mut interp_val = if diff_rc_rb > diff_rf_rb && diff_rc_rb > diff_rd_rb {
    rf + rd + 2 * rb
  } else if diff_rd_rb > diff_rc_rb && diff_rd_rb > diff_rf_rb {
    rf + rc + 2 * rb
  } else {
    rd + rc + 2 * rb
  };

  let code = read_code(pump, params, &mut gradients[grad.unsigned_abs() as usize], qtable);

  // Adjustments specific to even positions
  if grad < 0 {
    interp_val = (interp_val >> 2) - code * (2 * qtable.q_base as i32 + 1);
  } else {
    interp_val = (interp_val >> 2) + code * (2 * qtable.q_base as i32 + 1);
  };

  // Generic adjustments
  if interp_val < -(qtable.q_base as i32) {
    interp_val += (qtable.total_values * (2 * qtable.q_base + 1)) as i32;
  } else if interp_val > qtable.q_base as i32 + params.max_value {
    interp_val -= (qtable.total_values * (2 * qtable.q_base + 1)) as i32;
  }

  if interp_val >= 0 {
    linebuf[line][1 + *pos] = interp_val.min(params.max_value) as u16
  } else {
    linebuf[line][1 + *pos] = 0;
  }

  *pos += 2;
}

/// Decode samples for odd positions
fn fuji_decode_sample_odd(pump: &mut BitPumpMSB, params: &Params, linebuf: &mut [Vec<u16>], line: usize, pos: &mut usize, grads: &mut GradientList) {
  // Line -2 |   | f |   |
  // Line -1 | c | b | d |
  // Line  0 | a | x | g |
  let ra = linebuf[line + 0][1 + *pos - 1] as i32;
  let rb = linebuf[line - 1][1 + *pos + 0] as i32;
  let rc = linebuf[line - 1][1 + *pos - 1] as i32;
  let rd = linebuf[line - 1][1 + *pos + 1] as i32;
  let rg = linebuf[line + 0][1 + *pos + 1] as i32;
  // Calculate horiz/vert. gradients around current sample x
  let diff_rc_ra = (rc - ra).abs();
  let diff_rb_rc = (rb - rc).abs();
  // Quantization table and Gradients to use
  let mut qtable = &params.qtables[0];
  let mut gradients = &mut grads.lossless_grads;
  for i in 1..4 {
    if params.qtables[0].q_base < i as i32 {
      break;
    }
    if diff_rb_rc + diff_rc_ra <= params.qtables[i].max_grad as i32 {
      qtable = &params.qtables[i];
      gradients = &mut grads.lossy_grads[i - 1];
      break;
    }
  }
  // Determine gradient
  let grad = qtable.lookup_gradient(params, rb - rc, rc - ra);

  let mut interp_val = if (rb > rc && rb > rd) || (rb < rc && rb < rd) {
    (rg + ra + 2 * rb) >> 2
  } else {
    (ra + rg) >> 1
  };

  let code = read_code(pump, params, &mut gradients[grad.unsigned_abs() as usize], qtable);

  // Adjustments specific to odd positions
  if grad < 0 {
    interp_val -= code * (2 * qtable.q_base as i32 + 1);
  } else {
    interp_val += code * (2 * qtable.q_base as i32 + 1);
  }

  // Generic adjustments
  if interp_val < -(qtable.q_base as i32) {
    interp_val += (qtable.total_values * (2 * qtable.q_base + 1)) as i32;
  } else if interp_val > qtable.q_base as i32 + params.max_value {
    interp_val -= (qtable.total_values * (2 * qtable.q_base + 1)) as i32;
  }

  if interp_val >= 0 {
    linebuf[line][1 + *pos] = interp_val.min(params.max_value) as u16
  } else {
    linebuf[line][1 + *pos] = 0;
  }

  *pos += 2;
}

/// Interpolate x value from surrounding pixels
fn fuji_decode_interpolation_even(block: &mut CompressedBlock, line: usize, pos: &mut usize) {
  // Line -2 |   | f |   |
  // Line -1 | c | b | d |
  // Line  0 | a | x | g |
  let rb = block.linebuf[line - 1][1 + *pos + 0] as i32;
  let rc = block.linebuf[line - 1][1 + *pos - 1] as i32;
  let rd = block.linebuf[line - 1][1 + *pos + 1] as i32;
  let rf = block.linebuf[line - 2][1 + *pos + 0] as i32;

  let x = &mut block.linebuf[line][1 + *pos];

  let diff_rc_rb = (rc - rb).abs();
  let diff_rf_rb = (rf - rb).abs();
  let diff_rd_rb = (rd - rb).abs();

  if diff_rc_rb > diff_rf_rb && diff_rc_rb > diff_rd_rb {
    *x = ((rf + rd + 2 * rb) >> 2) as u16;
  } else if diff_rd_rb > diff_rc_rb && diff_rd_rb > diff_rf_rb {
    *x = ((rf + rc + 2 * rb) >> 2) as u16;
  } else {
    *x = ((rd + rc + 2 * rb) >> 2) as u16;
  }

  *pos += 2;
}

const XT_LINE_R0: usize = 0;
const XT_LINE_R1: usize = 1;
const XT_LINE_R2: usize = 2;
const XT_LINE_R3: usize = 3;
const XT_LINE_R4: usize = 4;
const XT_LINE_G0: usize = 5;
const XT_LINE_G1: usize = 6;
const XT_LINE_G2: usize = 7;
const XT_LINE_G3: usize = 8;
const XT_LINE_G4: usize = 9;
const XT_LINE_G5: usize = 10;
const XT_LINE_G6: usize = 11;
const XT_LINE_G7: usize = 12;
const XT_LINE_B0: usize = 13;
const XT_LINE_B1: usize = 14;
const XT_LINE_B2: usize = 15;
const XT_LINE_B3: usize = 16;
const XT_LINE_B4: usize = 17;
const XT_LINE_TOTAL: usize = 18;
