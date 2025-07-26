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

impl X3fFile {
  fn new(file: &RawSource) -> Result<X3fFile> {
    let buf = file.as_vec()?;
    let offset = LEu32(&buf, buf.len() - 4) as usize;
    let data = &buf[offset..];
    let version = LEu32(data, 4);
    if version < 0x00020000 {
      return Err(format_args!("X3F: Directory version too old {}", version).into());
    }
    let entries = LEu32(data, 8) as usize;
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
    let data = &buf[offset..];
    let off = LEu32(data, 0) as usize;
    let len = LEu32(data, 4) as usize;
    let name = String::from_utf8_lossy(&data[8..12]).to_string();

    Ok(X3fDirectory { offset: off, len, id: name })
  }
}

impl X3fImage {
  fn new(buf: &[u8], offset: usize) -> Result<X3fImage> {
    let data = &buf[offset..];

    Ok(X3fImage {
      typ: LEu32(data, 8) as usize,
      format: LEu32(data, 12) as usize,
      width: LEu32(data, 16) as usize,
      height: LEu32(data, 20) as usize,
      pitch: LEu32(data, 24) as usize,
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
      .ok_or("X3F: Couldn't find camera info")?;
    let data = &buffer[caminfo.doffset + 6..];
    if data[0..4] != b"Exif"[..] {
      return Err("X3F: Couldn't find EXIF info".into());
    }
    let tiff = IFD::new(&mut Cursor::new(&buffer), (caminfo.doffset + 12) as u32, 0, 0, Endian::Little, &[]).unwrap();

    let camera = self.rawloader.check_supported(&tiff)?;

    let imginfo = self.dir.images.iter().find(|i| i.typ == 1 || i.typ == 3).ok_or("X3F: Couldn't find image")?;
    let width = imginfo.width;
    let height = imginfo.height;
    let offset = imginfo.doffset;
    let src = &buffer[offset..];

    let image = match imginfo.format {
      35 => self.decode_compressed(src, width, height, dummy)?,
      x => return Err(format_args!("X3F Don't know how to decode format {}", x).into()),
    };

    let cpp = 3;
    let photometric = RawPhotometricInterpretation::LinearRaw;
    Ok(RawImage::new(camera, image, cpp, self.get_wb()?, photometric, None, None, dummy))
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
    Err("X3F decoding not implemented yet".into())
  }
}
