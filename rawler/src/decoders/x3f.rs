use std::io::Cursor;

use crate::bits::*;
use crate::decoders::*;

pub fn is_x3f(file: &RawSource) -> bool {
  match file.subview(0, 4) {
    Ok(buf) => buf[0..4] == b"FOVb"[..],
    Err(_) => false,
  }
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

/// Read a little-endian u32 at `pos`, fails if `buf` is too short
fn get_u32(buf: &[u8], pos: usize) -> Result<u32> {
  buf.get(pos..pos + 4).map(|bytes| LEu32(bytes, 0)).ok_or_else(|| behind_eof(pos))
}

/// The bytes of `buf` from `pos`, fails if `pos` is behind the end of `buf`
fn get_tail(buf: &[u8], pos: usize) -> Result<&[u8]> {
  buf.get(pos..).ok_or_else(|| behind_eof(pos))
}

fn behind_eof(pos: usize) -> RawlerError {
  decoder_error!("X3F: offset {} is behind EOF", pos)
}

impl X3fFile {
  fn new(file: &RawSource) -> Result<X3fFile> {
    let buf = file.as_vec()?;
    let offset = get_u32(&buf, buf.len().saturating_sub(4))? as usize;
    let data = get_tail(&buf, offset)?;
    let version = get_u32(data, 4)?;
    if version < 0x00020000 {
      return Err(decoder_error!("X3F: Directory version too old {}", version));
    }
    let entries = get_u32(data, 8)? as usize;
    let mut dirs = Vec::new();
    let mut images = Vec::new();
    for i in 0..entries {
      let dir = X3fDirectory::new(data, 12 + i * 12)?;
      if dir.id == "IMA2" {
        let img = X3fImage::new(&buf, dir.offset)?;
        images.push(img);
      }
      dirs.push(dir);
    }

    Ok(X3fFile { dirs, images })
  }
}

impl X3fDirectory {
  fn new(buf: &[u8], offset: usize) -> Result<X3fDirectory> {
    let off = get_u32(buf, offset)? as usize;
    let len = get_u32(buf, offset + 4)? as usize;
    let name = String::from_utf8_lossy(buf.get(offset + 8..offset + 12).ok_or_else(|| behind_eof(offset + 8))?).to_string();

    Ok(X3fDirectory { offset: off, len, id: name })
  }
}

impl X3fImage {
  fn new(buf: &[u8], offset: usize) -> Result<X3fImage> {
    Ok(X3fImage {
      typ: get_u32(buf, offset + 8)? as usize,
      format: get_u32(buf, offset + 12)? as usize,
      width: get_u32(buf, offset + 16)? as usize,
      height: get_u32(buf, offset + 20)? as usize,
      pitch: get_u32(buf, offset + 24)? as usize,
      doffset: offset + 28,
    })
  }
}

#[derive(Debug, Clone)]
pub struct X3fDecoder<'a> {
  rawloader: &'a RawLoader,
  dir: X3fFile,
}

impl<'a> X3fDecoder<'a> {
  pub fn new(file: &RawSource, rawloader: &'a RawLoader) -> Result<X3fDecoder<'a>> {
    let dir = X3fFile::new(file)?;

    Ok(X3fDecoder { rawloader, dir })
  }
}

impl<'a> Decoder for X3fDecoder<'a> {
  fn raw_image(&self, file: &RawSource, _params: &RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let buffer = file.as_vec()?;
    let caminfo = self
      .dir
      .images
      .iter()
      .find(|i| i.typ == 2 && i.format == 0x12)
      .ok_or_else(|| RawlerError::decoder_failed("X3F: Couldn't find camera info"))?;
    let data = get_tail(&buffer, caminfo.doffset + 6)?;
    if !data.starts_with(b"Exif") {
      return Err(RawlerError::decoder_failed("X3F: Couldn't find EXIF info"));
    }
    let tiff = IFD::new(&mut Cursor::new(&buffer), (caminfo.doffset + 12) as u32, 0, 0, Endian::Little, &[])?;

    let camera = self.rawloader.check_supported(&tiff)?;

    let imginfo = self
      .dir
      .images
      .iter()
      .find(|i| i.typ == 1 || i.typ == 3)
      .ok_or_else(|| RawlerError::decoder_failed("X3F: Couldn't find image"))?;
    let width = imginfo.width;
    let height = imginfo.height;
    let offset = imginfo.doffset;
    let src = get_tail(&buffer, offset)?;

    let image = match imginfo.format {
      35 => self.decode_compressed(src, width, height, dummy)?,
      x => return Err(decoder_error!("X3F Don't know how to decode format {}", x)),
    };

    let cpp = 3;
    let photometric = RawPhotometricInterpretation::LinearRaw;
    Ok(RawImage::new(camera, image, cpp, self.get_wb()?, photometric, None, None, dummy))
  }

  fn check_supported(&self) -> Result<()> {
    Err(RawlerError::decoder_failed("X3F decoding not implemented yet"))
  }

  fn format_dump(&self) -> FormatDump {
    todo!()
  }

  fn format_hint(&self) -> FormatHint {
    FormatHint::X3F
  }

  fn raw_metadata(&self, _file: &RawSource, _params: &RawDecodeParams) -> Result<RawMetadata> {
    todo!()
  }
}

impl<'a> X3fDecoder<'a> {
  fn get_wb(&self) -> Result<[f32; 4]> {
    Ok([f32::NAN, f32::NAN, f32::NAN, f32::NAN])
  }

  fn decode_compressed(&self, _buf: &[u8], _width: usize, _height: usize, _dummy: bool) -> Result<PixU16> {
    Err(RawlerError::decoder_failed("X3F decoding not implemented yet"))
  }
}
