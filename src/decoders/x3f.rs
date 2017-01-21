use decoders::*;
use decoders::basics::*;
  use std::f32::NAN;

pub fn is_x3f(buf: &[u8]) -> bool {
  buf[0..4] == b"FOVb"[..]
}

#[derive(Debug, Clone)]
struct X3fFile {
  dirs: Vec<X3fDirectory>,
}

#[derive(Debug, Clone)]
struct X3fDirectory {
  offset: usize,
  len: usize,
  id: String,
}

impl X3fFile {
  fn new(buf: &Buffer) -> Result<X3fFile, String> {
    let offset = LEu32(&buf.buf, buf.size-4) as usize;
    let data = &buf.buf[offset..];
    let version = LEu32(data, 4);
    if version < 0x00020000 {
      return Err(format!("X3F: Directory version too old {}", version).to_string())
    }
    let entries = LEu32(data, 8) as usize;
    let mut dirs = Vec::new();
    for i in 0..entries {
      let dir = try!(X3fDirectory::new(data, 12+i*12));
      dirs.push(dir);
    }

    Ok(X3fFile{
      dirs: dirs,
    })
  }
}

impl X3fDirectory {
  fn new(buf: &[u8], offset: usize) -> Result<X3fDirectory, String> {
    let data = &buf[offset..];
    let off = LEu32(data, 0) as usize;
    let len = LEu32(data, 4) as usize;
    let name = String::from_utf8_lossy(&data[8..12]).to_string();

    Ok(X3fDirectory {
      offset: off,
      len: len,
      id: name,
    })
  }
}

#[derive(Debug, Clone)]
pub struct X3fDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  dir: X3fFile,
}

impl<'a> X3fDecoder<'a> {
  pub fn new(buf: &'a Buffer, rawloader: &'a RawLoader) -> X3fDecoder<'a> {
    let dir = X3fFile::new(buf).unwrap();

    X3fDecoder {
      buffer: &buf.buf,
      rawloader: rawloader,
      dir: dir,
    }
  }
}

impl<'a> Decoder for X3fDecoder<'a> {
  fn image(&self) -> Result<RawImage,String> {
    Err("X3F not finished yet".to_string())
  }
}

impl<'a> X3fDecoder<'a> {
  fn get_wb(&self) -> Result<[f32;4], String> {
    Ok([NAN,NAN,NAN,NAN])
  }
}
