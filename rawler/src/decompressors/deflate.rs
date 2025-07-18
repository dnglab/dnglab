use std::io::Read;

use libflate::zlib::Decoder;

use crate::{
  bits::{Binary16, Binary24, Binary32, Endian, FloatingPointParameters, extend_binary_floating_point},
  decompressors::{Decompressor, LineIteratorMut},
};

#[derive(Debug)]
pub struct DeflateDecompressor {
  pred_factor: usize,
  bps: u32,
}

impl DeflateDecompressor {
  pub fn new(cpp: usize, predictor: u16, bps: u32, _endian: Endian) -> Self {
    let pred_factor = cpp
      * match predictor {
        3 => 1,
        34894 => 2,
        34895 => 4,
        _ => todo!(),
      };
    Self { pred_factor, bps }
  }
}

fn decode_delta_bytes(src: &mut [u8], factor: usize) {
  for col in factor..src.len() {
    src[col] = src[col].wrapping_add(src[col - factor]);
  }
}

fn decode_fp_delta_row<NARROW: FloatingPointParameters>(line: &mut [f32], row: &[u8], line_width: usize) {
  for (col, pix) in line.iter_mut().enumerate() {
    let mut tmp = [0; 4];
    assert!(NARROW::STORAGE_BYTES <= tmp.len());

    for c in 0..NARROW::STORAGE_BYTES {
      tmp[c] = row[col + c * line_width];
    }
    let value = u32::from_be_bytes(tmp) >> (u32::BITS as usize - NARROW::STORAGE_WIDTH);
    *pix = f32::from_bits(extend_binary_floating_point::<NARROW, Binary32>(value));
  }
}

impl<'a> Decompressor<'a, f32> for DeflateDecompressor {
  fn decompress(&self, src: &[u8], skip_rows: usize, lines: impl LineIteratorMut<'a, f32>, line_width: usize) -> std::result::Result<(), String> {
    //eprintln!("Deflate: {:?}", self);
    let mut decoder = Decoder::new(src).unwrap();
    let mut decoded_data = Vec::new();
    decoder.read_to_end(&mut decoded_data).unwrap();

    let bytesps = self.bps as usize / 8;
    assert!(bytesps >= 2 && bytesps <= 4);

    assert_eq!(decoded_data.len(), bytesps * line_width * lines.len());

    for (line, row) in lines.zip(decoded_data.chunks_exact_mut(bytesps as usize * line_width)).skip(skip_rows) {
      assert_eq!(line.len(), line_width);
      decode_delta_bytes(row, self.pred_factor);

      match self.bps {
        16 => decode_fp_delta_row::<Binary16>(line, row, line_width),
        24 => decode_fp_delta_row::<Binary24>(line, row, line_width),
        32 => decode_fp_delta_row::<Binary32>(line, row, line_width),
        _ => unimplemented!(),
      }
    }

    Ok(())

    //let packed = PackedDecompressor::new(self.bps, self.endian);

    //packed.decompress(&decoded_data, skip_rows, lines, line_width)
  }
}
