use byteorder::{BigEndian, ReadBytesExt};
use std::io::{Read, Seek, SeekFrom};
use thiserror::Error;

use crate::formats::tiff::IFD;
use crate::RawFile;

pub type Result<T> = std::result::Result<T, JfifError>;

/// Error variants for JFIF parser
#[derive(Debug, Error)]
pub enum JfifError {
  /// Overflow of input, size constraints...
  #[error("Overflow error: {}", _0)]
  Overflow(String),

  #[error("General error: {}", _0)]
  General(String),

  #[error("Format mismatch: {}", _0)]
  FormatMismatch(String),

  /// Error on internal cursor type
  #[error("I/O error: {:?}", _0)]
  Io(#[from] std::io::Error),
}

pub trait ReadSegment<T>: Sized {
  fn read_segment(_: T, symbol: u16) -> Result<Self>;
}

#[derive(Debug, Clone, Default)]
pub struct App0 {
  pub len: u64,
  pub version: u16,
  pub density_units: u8,
  pub xdensity: u16,
  pub ydensity: u16,
  pub xthumbnail: u8,
  pub ythumbnail: u8,
  pub thumbnail: Option<Vec<u8>>,
}

impl<R: Read + Seek> ReadSegment<&mut R> for App0 {
  fn read_segment(reader: &mut R, _symbol: u16) -> Result<Self> {
    //let pos = reader.stream_position()?;
    let len: u64 = reader.read_u16::<BigEndian>()? as u64;

    const APP1_JFIF_MARKER: [u8; 5] = [b'J', b'F', b'I', b'F', b'\0'];

    if len as usize >= APP1_JFIF_MARKER.len() {
      let mut jfif_str = [0; 5];
      reader.read_exact(&mut jfif_str)?;
      if jfif_str == APP1_JFIF_MARKER {
        let version = reader.read_u16::<BigEndian>()?;
        let density_units = reader.read_u8()?;
        let xdensity = reader.read_u16::<BigEndian>()?;
        let ydensity = reader.read_u16::<BigEndian>()?;
        let xthumbnail = reader.read_u8()?;
        let ythumbnail = reader.read_u8()?;
        let thumbnail = if xthumbnail * ythumbnail > 0 {
          let mut data = vec![0; (3 * xthumbnail * ythumbnail) as usize];
          reader.read_exact(&mut data)?;
          Some(data)
        } else {
          None
        };
        Ok(Self {
          len,
          version,
          density_units,
          xdensity,
          ydensity,
          xthumbnail,
          ythumbnail,
          thumbnail,
        })
      } else {
        Err(JfifError::FormatMismatch("Failed to find JFIF marker in APP0 segment".into()))
      }
    } else {
      Err(JfifError::FormatMismatch("APP0 segment is too short".into()))
    }
  }
}

#[derive(Debug, Clone)]

pub struct App1 {
  pub len: u64,
  pub payload: Payload,
}

#[derive(Debug, Clone)]

pub struct SOS {
  pub len: u64,
}

impl<R: Read + Seek> ReadSegment<&mut R> for SOS {
  fn read_segment(reader: &mut R, _symbol: u16) -> Result<Self> {
    let pos = reader.stream_position()?;
    let mut prev = 0;

    loop {
      let v = reader.read_u8()?;
      if prev == 0xFF && v == 0xFF {
        prev = 0;
        continue;
      }
      if prev == 0xFF && (v & !0x7) == 0xD0 {
        prev = 0;
        continue;
      }
      if prev == 0xFF && v == 0 {
        prev = 0;
        continue;
      }
      if prev == 0xFF {
        reader.seek(SeekFrom::Current(-2))?;
        break;
      }
      prev = v;
    }

    let len = reader.stream_position()? - pos;

    Ok(Self { len })
  }
}

#[derive(Debug, Clone)]
pub enum Payload {
  Exif(IFD),
  Xpacket(Vec<u8>),
  Unknown,
}

impl<R: Read + Seek> ReadSegment<&mut R> for App1 {
  fn read_segment(reader: &mut R, _symbol: u16) -> Result<Self> {
    let pos = reader.stream_position()?;
    let len: u64 = reader.read_u16::<BigEndian>()? as u64;

    const APP1_EXIF_MARKER: [u8; 6] = [b'E', b'x', b'i', b'f', b'\0', b'\0'];

    if len as usize >= APP1_EXIF_MARKER.len() {
      let mut exif_str = [0; 6];
      reader.read_exact(&mut exif_str)?;
      if exif_str == APP1_EXIF_MARKER {
        let ifd = IFD::new_root(reader, pos as u32 + 2 + 6).unwrap();
        reader.seek(SeekFrom::Start(pos + len))?;

        /*
        for i in ifd.dump::<TiffCommonTag>(10) {
          println!("{}", i);
        }

        for i in ifd.get_sub_ifds(TiffCommonTag::ExifIFDPointer).unwrap()[0].dump::<ExifTag>(10) {
          println!("{}", i);
        }
         */

        return Ok(Self {
          len,
          payload: Payload::Exif(ifd),
        });
      } else {
        reader.seek(SeekFrom::Current(-(exif_str.len() as i64)))?;
      }
    }

    const APP1_XMP_MARKER: &str = concat!("http://ns.adobe.com/xap/1.0/", '\0');
    if len as usize >= APP1_XMP_MARKER.len() {
      let mut xmp_str = [0; APP1_XMP_MARKER.len()];
      reader.read_exact(&mut xmp_str)?;
      if xmp_str == APP1_XMP_MARKER.as_bytes() {
        log::debug!("Found APP1 XMP marker");
        //let zero_byte = reader.read_u8()?;
        //assert_eq!(zero_byte, 0);
        let mut xpacket = vec![0; len as usize - xmp_str.len() - 2];

        reader.read_exact(&mut xpacket)?;
        //dump_buf("/tmp/dump.1",&xpacket);
        reader.seek(SeekFrom::Start(pos + len))?;
        return Ok(Self {
          len,
          payload: Payload::Xpacket(xpacket),
        });
      } else {
        reader.seek(SeekFrom::Current(-(xmp_str.len() as i64)))?;
      }
    }
    log::debug!("Found APP1: UNKNOWN");
    reader.seek(SeekFrom::Start(pos + len))?;
    return Ok(Self {
      len,
      payload: Payload::Unknown,
    });
  }
}

#[derive(Debug, Clone)]
pub enum Segment {
  SOI { offset: u64 },
  APP0 { offset: u64, app0: App0 },
  APP1 { offset: u64, app1: App1 },
  SOS { offset: u64, sos: SOS },
  EOI,
  Unknown { offset: u64, marker: u16 },
}

#[derive(Debug, Clone)]
pub struct Jfif {
  pub segments: Vec<Segment>,
}

impl Jfif {
  pub fn parse<R: Read + Seek>(mut reader: R) -> Result<Self> {
    let mut segments = Vec::new();
    let mut pos = reader.stream_position()?;
    let mut sym = Some(reader.read_u16::<BigEndian>()?);

    while let Some(symbol) = sym {
      log::debug!("Found symbol: {:X}", symbol);
      let segment = match symbol {
        0xFFD8 => Segment::SOI { offset: pos },
        0xFFD9 => Segment::EOI,
        0xFFE0 => Segment::APP0 {
          offset: pos,
          app0: App0::read_segment(&mut reader, symbol)?,
        },
        0xFFE1 => Segment::APP1 {
          offset: pos,
          app1: App1::read_segment(&mut reader, symbol)?,
        },
        0xFFDA => Segment::SOS {
          offset: pos,
          sos: SOS::read_segment(&mut reader, symbol)?,
        },

        _ => {
          log::debug!("Unhandled JFIF segment marker: {:X}", symbol);
          let len: u64 = reader.read_u16::<BigEndian>()? as u64;
          reader.seek(SeekFrom::Current(len as i64 - 2))?;

          Segment::Unknown { offset: pos, marker: symbol }
        }
      };
      if segments.is_empty() && sym != Some(0xFFD8) {
        return Err(JfifError::General("first marker must be SOI".into()));
      }
      segments.push(segment);
      if symbol == 0xFFD9 {
        break; // EOI reached
      }
      pos = reader.stream_position()?;
      sym = reader.read_u16::<BigEndian>().ok();
    }
    if !segments.is_empty() {
      Ok(Self { segments })
    } else {
      Err(JfifError::FormatMismatch("JFIF contains no segements".into()))
    }
  }

  pub fn new(file: &mut RawFile) -> Result<Self> {
    Self::parse(file.inner())
  }

  pub fn exif_ifd(&self) -> Option<&IFD> {
    self.segments.iter().find_map(|seg| match seg {
      Segment::APP1 {
        app1: App1 {
          payload: Payload::Exif(ifd), ..
        },
        ..
      } => Some(ifd),
      _ => None,
    })
  }

  pub fn xpacket(&self) -> Option<&Vec<u8>> {
    self.segments.iter().find_map(|seg| match seg {
      Segment::APP1 {
        app1: App1 {
          payload: Payload::Xpacket(xpacket),
          ..
        },
        ..
      } => Some(xpacket),
      _ => None,
    })
  }
}

pub fn is_jfif(file: &mut RawFile) -> bool {
  match file.subview(0, 4) {
    Ok(buf) => {
      let result = buf[0..4] == [0xFF, 0xD8, 0xFF, 0xE0];
      if !result {
        //panic!("Failed: {:x} {:x} {:x} {:x}", buf[0], buf[1], buf[2], buf[3]);
      }
      result
    },
    Err(err) => {
      log::error!("is_jfif(): {:?}", err);
      false
    }
  }
}

pub fn is_exif(file: &mut RawFile) -> bool {
  match file.subview(0, 4) {
    Ok(buf) => buf[0..4] == [0xFF, 0xD8, 0xFF, 0xE1],
    Err(err) => {
      log::error!("is_exif(): {:?}", err);
      false
    }
  }
}
