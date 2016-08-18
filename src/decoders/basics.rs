#[allow(non_snake_case)] pub fn BEu32(buf: &[u8], pos: usize) -> u32 {
  (buf[pos] as u32) << 24 |
  (buf[pos+1] as u32) << 16 |
  (buf[pos+2] as u32) << 8 |
  (buf[pos+3] as u32)
}

#[allow(non_snake_case)] pub fn LEu32(buf: &[u8], pos: usize) -> u32 {
  (buf[pos] as u32) |
  (buf[pos+1] as u32) << 8 |
  (buf[pos+2] as u32) << 16 |
  (buf[pos+3] as u32) << 24
}

#[allow(non_snake_case)] pub fn BEu16(buf: &[u8], pos: usize) -> u16 {
  (buf[pos] as u16) << 8 | (buf[pos+1] as u16)
}

#[allow(non_snake_case)] pub fn LEu16(buf: &[u8], pos: usize) -> u16 {
  (buf[pos] as u16) | (buf[pos+1] as u16) << 8
}
