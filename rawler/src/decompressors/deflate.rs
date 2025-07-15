use crate::decompressors::Decompressor;
pub struct DeflateDecompressor {}

impl DeflateDecompressor {
  pub fn new() -> Self {
    Self {}
  }
}

impl<'a> Decompressor<'a, f32> for DeflateDecompressor {
  fn decompress(&self, _src: &[u8], _skip_rows: usize, _lines: impl Iterator<Item = &'a mut [f32]>, _line_width: usize) -> std::result::Result<(), String> {
    todo!()
  }
}
