use std::f32::NAN;

use crate::decoders::*;
use crate::formats::tiff_legacy::*;
use crate::bits::*;

pub fn is_x3f(buf: &[u8]) -> bool {
  buf[0..4] == b"FOVb"[..]
}

#[derive(Debug, Clone)]
struct X3fFile {
  #[allow(dead_code)]
  dirs: Vec<X3fDirectory>,
  images: Vec<X3fImage>,
}

#[derive(Debug, Clone)]
struct X3fDirectory {
  offset: usize,
  #[allow(dead_code)]
  len: usize,
  id: String,
}

#[derive(Debug, Clone)]
struct X3fImage {
  typ: usize,
  format: usize,
  width: usize,
  height: usize,
  #[allow(dead_code)]
  pitch: usize,
  doffset: usize,
}

impl X3fFile {
  fn new(buf: &Buffer) -> Result<X3fFile> {
    let offset = LEu32(&buf.buf, buf.size-4) as usize;
    let data = &buf.buf[offset..];
    let version = LEu32(data, 4);
    if version < 0x00020000 {
      return Err(RawlerError::Unsupported(format!("X3F: Directory version too old {}", version).to_string()))
    }
    let entries = LEu32(data, 8) as usize;
    let mut dirs = Vec::new();
    let mut images = Vec::new();
    for i in 0..entries {
      let dir = X3fDirectory::new(data, 12+i*12)?;
      if dir.id == "IMA2" {
        let img = X3fImage::new(&buf.buf, dir.offset)?;
        images.push(img);
      }
      dirs.push(dir);
    }

    Ok(X3fFile{
      dirs: dirs,
      images: images,
    })
  }
}

impl X3fDirectory {
  fn new(buf: &[u8], offset: usize) -> Result<X3fDirectory> {
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

impl X3fImage {
  fn new(buf: &[u8], offset: usize) -> Result<X3fImage> {
    let data = &buf[offset..];

    Ok(X3fImage {
      typ:     LEu32(data,  8) as usize,
      format:  LEu32(data, 12) as usize,
      width:   LEu32(data, 16) as usize,
      height:  LEu32(data, 20) as usize,
      pitch:   LEu32(data, 24) as usize,
      doffset: offset+28,
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
  fn raw_image(&self, _params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let caminfo = self.dir.images
        .iter()
        .find(|i| i.typ == 2 && i.format == 0x12)
        .ok_or("X3F: Couldn't find camera info".to_string())?;
    let data = &self.buffer[caminfo.doffset+6..];
    if data[0..4] != b"Exif"[..] {
      return Err(RawlerError::Unsupported("X3F: Couldn't find EXIF info".to_string()))
    }
    let tiff = LegacyTiffIFD::new_root(self.buffer, caminfo.doffset+12, &vec![])?;
    let camera = self.rawloader.check_supported_old(&tiff)?;

    let imginfo = self.dir.images
        .iter()
        .find(|i| i.typ == 1 || i.typ == 3)
        .ok_or("X3F: Couldn't find image".to_string())?;
    let width = imginfo.width;
    let height = imginfo.height;
    let offset = imginfo.doffset;
    let src = &self.buffer[offset..];

    let image = match imginfo.format {
      35 => self.decode_compressed(src, width, height, dummy)?,
      x => return Err(RawlerError::Unsupported(format!("X3F Don't know how to decode format {}", x).to_string()))
    };

    let mut img = RawImage::new(camera, width, height, self.get_wb()?, image, dummy);
    img.cpp = 3;
    Ok(img)
  }
}

impl<'a> X3fDecoder<'a> {
  fn get_wb(&self) -> Result<[f32;4]> {
    Ok([NAN,NAN,NAN,NAN])
  }

  fn decode_compressed(&self, _buf: &[u8], _width: usize, _height: usize, _dummy: bool) -> Result<Vec<u16>> {
    return Err(RawlerError::Unsupported("X3F decoding not implemented yet".to_string()))
  }
}
