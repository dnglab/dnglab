use byteorder::{BigEndian, ReadBytesExt};
use std::io::{Read, Seek, SeekFrom};
use thiserror::Error;

use crate::formats::tiff::IFD;
use crate::rawsource::RawSource;

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
        // Widen before multiplying: `xthumbnail * ythumbnail` as `u8 * u8`
        // overflows (e.g. 16 * 16) and panics under overflow checks. The
        // widened product is the same value for any real thumbnail size, so
        // valid files are unaffected; only the overflow panic on crafted
        // dimensions is removed. The subsequent `read_exact` still bounds the
        // actual allocation to the bytes present in the file.
        let thumbnail_pixels = xthumbnail as usize * ythumbnail as usize;
        let thumbnail = if thumbnail_pixels > 0 {
          let mut data = vec![0; 3 * thumbnail_pixels];
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
        let ifd = IFD::new_root(reader, pos as u32 + 2 + 6);
        reader.seek(SeekFrom::Start(pos + len))?;

        /*
        for i in ifd.dump::<TiffCommonTag>(10) {
          println!("{}", i);
        }

        for i in ifd.get_sub_ifds(TiffCommonTag::ExifIFDPointer).unwrap()[0].dump::<ExifTag>(10) {
          println!("{}", i);
        }
         */

        if let Ok(ifd) = ifd {
          return Ok(Self {
            len,
            payload: Payload::Exif(ifd),
          });
        }

        return Err(JfifError::General("Failed to read exif".into()));
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
        // `len` includes the 2-byte length field and the XMP marker; a corrupt
        // `len` smaller than `marker + 2` would underflow this subtraction. The
        // outer guard only ensures `len >= marker.len()`, not `>= marker.len() +
        // 2`, so a crafted `len` of exactly the marker length reaches here. A
        // valid XMP segment always satisfies `len >= marker.len() + 2`, so this
        // check never fires on well-formed input.
        let xpacket_len = (len as usize)
          .checked_sub(xmp_str.len() + 2)
          .ok_or_else(|| JfifError::General(format!("Invalid APP1 XMP segment length {}", len)))?;
        let mut xpacket = vec![0; xpacket_len];
        reader.read_exact(&mut xpacket)?;
        reader.seek(SeekFrom::Start(pos + len))?;
        return Ok(Self {
          len,
          payload: Payload::Xpacket(xpacket),
        });
      } else {
        reader.seek(SeekFrom::Current(-(xmp_str.len() as i64)))?;
      }
    }
    reader.seek(SeekFrom::Start(pos + len))?;
    Ok(Self {
      len,
      payload: Payload::Unknown,
    })
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
        0xFFE1 => match App1::read_segment(&mut reader, symbol) {
          Ok(app1) => Segment::APP1 { offset: pos, app1 },
          Err(err) => {
            log::warn!("Failed to read APP1 (EXIF) segement, maybe corrupt: {:?}", err);
            // Advance past this marker before retrying. The bare `continue`
            // here did not update `sym`/`pos`, so a corrupt/truncated APP1
            // (e.g. a JFIF with a partial EXIF segment) re-processed the same
            // stale `0xFFE1` symbol forever. On a well-formed file this branch
            // is never taken (the segment parses), so reading the next symbol
            // here only changes the malformed path: we skip the bad segment
            // and continue marker scanning instead of hanging.
            pos = reader.stream_position()?;
            sym = reader.read_u16::<BigEndian>().ok();
            continue;
          }
        },
        0xFFDA => Segment::SOS {
          offset: pos,
          sos: SOS::read_segment(&mut reader, symbol)?,
        },

        _ => {
          log::debug!("Unhandled JFIF segment marker: {:X}", symbol);
          let len: u64 = reader.read_u16::<BigEndian>()? as u64;
          // A JFIF segment length is inclusive of its own 2-byte length field,
          // so a well-formed segment always has `len >= 2`. A corrupt `len < 2`
          // would seek backwards (`len - 2 < 0`) and re-read the same marker
          // forever; reject it instead. Valid files never take this branch.
          let skip = (len as i64)
            .checked_sub(2)
            .filter(|&n| n >= 0)
            .ok_or_else(|| JfifError::General(format!("Invalid JFIF segment length {} for marker {:X}", len, symbol)))?;
          reader.seek(SeekFrom::Current(skip))?;

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

  pub fn new(file: &RawSource) -> Result<Self> {
    Self::parse(&mut file.reader())
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

pub fn is_jfif(file: &RawSource) -> bool {
  match file.subview(0, 4) {
    Ok(buf) => {
      let result = buf[0..4] == [0xFF, 0xD8, 0xFF, 0xE0];
      if !result {
        //panic!("Failed: {:x} {:x} {:x} {:x}", buf[0], buf[1], buf[2], buf[3]);
      }
      result
    }
    Err(err) => {
      log::error!("is_jfif(): {:?}", err);
      false
    }
  }
}

pub fn is_exif(file: &RawSource) -> bool {
  match file.subview(0, 4) {
    Ok(buf) => buf[0..4] == [0xFF, 0xD8, 0xFF, 0xE1],
    Err(err) => {
      log::error!("is_exif(): {:?}", err);
      false
    }
  }
}
