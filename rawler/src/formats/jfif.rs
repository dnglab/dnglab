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

/// Bytes of a segment after its length field, which counts itself
fn payload_len(len: u64) -> Result<u64> {
  len
    .checked_sub(2)
    .ok_or_else(|| JfifError::FormatMismatch(format!("Segment length {} is shorter than the length field", len)))
}

/// Skip the segment whose length field is at `start`, false if the length
/// is invalid or missing
fn skip_segment<R: Read + Seek>(reader: &mut R, start: u64) -> bool {
  fn skip<R: Read + Seek>(reader: &mut R, start: u64) -> Result<()> {
    reader.seek(SeekFrom::Start(start))?;
    let len = payload_len(reader.read_u16::<BigEndian>()? as u64)?;
    reader.seek(SeekFrom::Start(start + 2 + len))?;
    Ok(())
  }
  skip(reader, start).is_ok()
}

/// The offset and code of the next marker. Fill bytes 0xFF before the
/// marker are skipped. None at the end of the stream or if the next bytes
/// are no marker.
fn read_marker<R: Read + Seek>(reader: &mut R) -> Option<(u64, u16)> {
  if reader.read_u8().ok()? != 0xFF {
    log::warn!("JFIF segment does not start with a marker, ignoring the rest of the file");
    return None;
  }
  loop {
    match reader.read_u8().ok()? {
      0xFF => continue,
      0x00 => {
        log::warn!("JFIF contains a stuffed zero byte outside of image data, ignoring the rest of the file");
        return None;
      }
      code => return Some((reader.stream_position().ok()? - 2, 0xFF00 | code as u16)),
    }
  }
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
    let pos = reader.stream_position()?;
    let len: u64 = reader.read_u16::<BigEndian>()? as u64;

    const APP1_JFIF_MARKER: [u8; 5] = [b'J', b'F', b'I', b'F', b'\0'];

    let payload = payload_len(len)?;
    if payload >= APP1_JFIF_MARKER.len() as u64 {
      let mut jfif_str = [0; 5];
      reader.read_exact(&mut jfif_str)?;
      if jfif_str == APP1_JFIF_MARKER {
        let version = reader.read_u16::<BigEndian>()?;
        let density_units = reader.read_u8()?;
        let xdensity = reader.read_u16::<BigEndian>()?;
        let ydensity = reader.read_u16::<BigEndian>()?;
        let xthumbnail = reader.read_u8()?;
        let ythumbnail = reader.read_u8()?;
        // RGB thumbnail after the 14 bytes of the header
        let thumbnail_len = 3 * xthumbnail as usize * ythumbnail as usize;
        if 14 + thumbnail_len as u64 > payload {
          return Err(JfifError::FormatMismatch(format!(
            "APP0 thumbnail of {}x{} pixels exceeds the segment of {} bytes",
            xthumbnail, ythumbnail, len
          )));
        }
        let thumbnail = if thumbnail_len > 0 {
          let mut data = vec![0; thumbnail_len];
          reader.read_exact(&mut data)?;
          Some(data)
        } else {
          None
        };
        reader.seek(SeekFrom::Start(pos + len))?;
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
      let v = match reader.read_u8() {
        Ok(v) => v,
        // A JPEG cut off in the image data keeps the segments before it
        Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
          log::warn!("JFIF image data ends without a marker, the file may be truncated");
          break;
        }
        Err(err) => return Err(err.into()),
      };
      if prev == 0xFF {
        match v {
          // A fill byte, the next byte may start a marker
          0xFF => continue,
          // A stuffed zero byte or a restart marker RST0 to RST7
          0x00 | 0xD0..=0xD7 => {
            prev = 0;
            continue;
          }
          _ => {
            reader.seek(SeekFrom::Current(-2))?;
            break;
          }
        }
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

    let payload = payload_len(len)?;
    if payload >= APP1_EXIF_MARKER.len() as u64 {
      let mut exif_str = [0; 6];
      reader.read_exact(&mut exif_str)?;
      if exif_str == APP1_EXIF_MARKER {
        // The TIFF structure follows the length field and the marker
        let base = u32::try_from(pos + 2 + 6).map_err(|_| JfifError::Overflow(format!("EXIF data at {} exceeds 4 GiB", pos)))?;
        let ifd = IFD::new_root(reader, base);
        reader.seek(SeekFrom::Start(pos + len))?;

        /*
        for i in ifd.dump::<TiffCommonTag>(10) {
          println!("{}", i);
        }

        for i in ifd.get_sub_ifds(TiffCommonTag::ExifIFDPointer).unwrap()[0].dump::<ExifTag>(10) {
          println!("{}", i);
        }
         */

        // A broken EXIF structure does not affect the other segments
        let payload = match ifd {
          Ok(ifd) => Payload::Exif(ifd),
          Err(err) => {
            log::warn!("Failed to read the EXIF data of an APP1 segment, ignoring it: {}", err);
            Payload::Unknown
          }
        };
        return Ok(Self { len, payload });
      } else {
        reader.seek(SeekFrom::Current(-(exif_str.len() as i64)))?;
      }
    }

    const APP1_XMP_MARKER: &str = concat!("http://ns.adobe.com/xap/1.0/", '\0');
    if payload >= APP1_XMP_MARKER.len() as u64 {
      let mut xmp_str = [0; APP1_XMP_MARKER.len()];
      reader.read_exact(&mut xmp_str)?;
      if xmp_str == APP1_XMP_MARKER.as_bytes() {
        log::debug!("Found APP1 XMP marker");
        let mut xpacket = vec![0; (payload - xmp_str.len() as u64) as usize];
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
    let first = reader.read_u16::<BigEndian>()?;
    if first != 0xFFD8 {
      return Err(JfifError::FormatMismatch(format!("First marker must be SOI, but is {:X}", first)));
    }
    let mut sym = Some(first);

    while let Some(symbol) = sym {
      log::debug!("Found symbol: {:X}", symbol);
      let segment = match symbol {
        0xFFD8 => Segment::SOI { offset: pos },
        0xFFD9 => Segment::EOI,
        0xFFE0 => {
          let start = reader.stream_position()?;
          match App0::read_segment(&mut reader, symbol) {
            Ok(app0) => Segment::APP0 { offset: pos, app0 },
            // Like a JFXX extension, which follows the JFIF APP0
            Err(err) => {
              log::debug!("APP0 segment is no JFIF segment, skipping it: {:?}", err);
              if !skip_segment(&mut reader, start) {
                break;
              }
              Segment::Unknown { offset: pos, marker: symbol }
            }
          }
        }
        0xFFE1 => {
          let start = reader.stream_position()?;
          match App1::read_segment(&mut reader, symbol) {
            Ok(app1) => Segment::APP1 { offset: pos, app1 },
            Err(err) => {
              log::warn!("Failed to read APP1 segment, maybe corrupt: {:?}", err);
              // A truncated segment header has no length to skip the segment
              // with, so keep the segments parsed so far and stop.
              if !skip_segment(&mut reader, start) {
                break;
              }
              Segment::Unknown { offset: pos, marker: symbol }
            }
          }
        }
        0xFFDA => Segment::SOS {
          offset: pos,
          sos: SOS::read_segment(&mut reader, symbol)?,
        },

        // TEM and RST0 to RST7 have no length field
        0xFF01 | 0xFFD0..=0xFFD7 => Segment::Unknown { offset: pos, marker: symbol },
        _ => {
          log::debug!("Unhandled JFIF segment marker: {:X}", symbol);
          let start = reader.stream_position()?;
          if !skip_segment(&mut reader, start) {
            log::warn!("JFIF segment {:X} at {} has no valid length, ignoring the rest of the file", symbol, pos);
            break;
          }
          Segment::Unknown { offset: pos, marker: symbol }
        }
      };
      segments.push(segment);
      if symbol == 0xFFD9 {
        break; // EOI reached
      }
      sym = read_marker(&mut reader).map(|(offset, marker)| {
        pos = offset;
        marker
      });
    }
    Ok(Self { segments })
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

#[cfg(test)]
mod tests {
  use super::*;
  use std::io::Cursor;

  /// SOI, the segment `marker` with the length `len` and `payload`, EOI
  fn jpeg(marker: u8, len: u16, payload: &[u8]) -> Vec<u8> {
    let mut data = vec![0xFF, 0xD8, 0xFF, marker];
    data.extend_from_slice(&len.to_be_bytes());
    data.extend_from_slice(payload);
    data.extend_from_slice(&[0xFF, 0xD9]);
    data
  }

  const XMP_MARKER: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";

  #[test]
  fn short_xmp_segments() {
    // The length counts the 2 length bytes, a segment of 29 or 30 bytes has
    // no room for the marker, which continues behind the segment
    for len in [29, 30] {
      if let Ok(parsed) = Jfif::parse(Cursor::new(jpeg(0xE1, len, XMP_MARKER))) {
        assert!(parsed.xpacket().is_none());
      }
    }
    // Marker and an empty packet
    let parsed = Jfif::parse(Cursor::new(jpeg(0xE1, 31, XMP_MARKER))).expect("parsed");
    assert_eq!(parsed.xpacket().map(Vec::len), Some(0));
    // Marker and a packet of 3 bytes
    let parsed = Jfif::parse(Cursor::new(jpeg(0xE1, 34, &[XMP_MARKER, b"xmp"].concat()))).expect("parsed");
    assert_eq!(parsed.xpacket().map(Vec::as_slice), Some(&b"xmp"[..]));
  }

  /// APP0 payload of JFIF 1.2 with a thumbnail of `x` * `y` pixels
  fn jfif_app0(x: u8, y: u8) -> Vec<u8> {
    let mut payload = b"JFIF\0\x01\x02\0\0\x01\0\x01".to_vec();
    payload.extend_from_slice(&[x, y]);
    payload.resize(payload.len() + 3 * x as usize * y as usize, 0x80);
    payload
  }

  #[test]
  fn app0_thumbnail() {
    let payload = jfif_app0(16, 16);
    let parsed = Jfif::parse(Cursor::new(jpeg(0xE0, payload.len() as u16 + 2, &payload))).expect("parsed");
    match &parsed.segments[1] {
      Segment::APP0 { app0, .. } => assert_eq!(app0.thumbnail.as_ref().map(Vec::len), Some(16 * 16 * 3)),
      segment => panic!("APP0 expected, got {:?}", segment),
    }
    // The thumbnail does not fit in the segment, which is skipped
    let parsed = Jfif::parse(Cursor::new(jpeg(0xE0, 16, &payload[..14]))).expect("parsed");
    assert!(matches!(
      parsed.segments.as_slice(),
      [Segment::SOI { .. }, Segment::Unknown { marker: 0xFFE0, .. }, Segment::EOI]
    ));
  }

  #[test]
  fn app0_extension_and_trailing_bytes() {
    // JFIF APP0 with 2 more bytes than its header, then a JFXX APP0
    let mut payload = jfif_app0(0, 0);
    payload.extend_from_slice(&[0xFF, 0xD9]);
    let mut data = jpeg(0xE0, payload.len() as u16 + 2, &payload);
    data.truncate(data.len() - 2);
    data.extend_from_slice(&[0xFF, 0xE0, 0, 8]);
    data.extend_from_slice(b"JFXX\0\x10");
    data.extend_from_slice(&[0xFF, 0xD9]);
    let parsed = Jfif::parse(Cursor::new(data)).expect("parsed");
    assert!(matches!(
      parsed.segments.as_slice(),
      [Segment::SOI { .. }, Segment::APP0 { .. }, Segment::Unknown { marker: 0xFFE0, .. }, Segment::EOI]
    ));
  }

  #[test]
  fn broken_exif_keeps_xmp() {
    // EXIF APP1 without a TIFF header, then XMP APP1
    let mut data = jpeg(0xE1, 10, b"Exif\0\0XX");
    data.truncate(data.len() - 2);
    let xmp = [XMP_MARKER, b"xmp"].concat();
    data.extend_from_slice(&[0xFF, 0xE1]);
    data.extend_from_slice(&(xmp.len() as u16 + 2).to_be_bytes());
    data.extend_from_slice(&xmp);
    data.extend_from_slice(&[0xFF, 0xD9]);
    let parsed = Jfif::parse(Cursor::new(data)).expect("parsed");
    assert!(parsed.exif_ifd().is_none());
    assert_eq!(parsed.xpacket().map(Vec::as_slice), Some(&b"xmp"[..]));
    assert!(matches!(parsed.segments.last(), Some(Segment::EOI)));
  }

  /// TIFF structure with one IFD of one entry
  const TIFF: [u8; 26] = [b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0, 1, 3, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0];

  #[test]
  fn sos_finds_marker_after_fill_bytes() {
    let mut data = vec![0xFF, 0xD8, 0xFF, 0xDA, 0x12, 0xFF, 0x00, 0x34, 0xFF, 0xD0, 0x56];
    data.extend_from_slice(&[0xFF, 0xFF, 0xD9]);
    let parsed = Jfif::parse(Cursor::new(data)).expect("parsed");
    assert!(matches!(parsed.segments.as_slice(), [Segment::SOI { .. }, Segment::SOS { .. }, Segment::EOI]));
  }

  #[test]
  fn truncated_image_data_keeps_exif() {
    let mut data = jpeg(0xE1, 2 + 6 + TIFF.len() as u16, &[&b"Exif\0\0"[..], &TIFF].concat());
    data.truncate(data.len() - 2);
    data.extend_from_slice(&[0xFF, 0xDA, 0x12, 0x34, 0xFF]);
    let parsed = Jfif::parse(Cursor::new(data)).expect("parsed");
    assert!(parsed.exif_ifd().is_some());
    assert!(matches!(parsed.segments.last(), Some(Segment::SOS { .. })));
  }

  #[test]
  fn markers_without_length_and_fill_bytes() {
    // RST0, TEM and fill bytes before EOI
    let data = [0xFF, 0xD8, 0xFF, 0xD0, 0xFF, 0x01, 0xFF, 0xFF, 0xFF, 0xD9];
    let parsed = Jfif::parse(Cursor::new(data)).expect("parsed");
    assert!(matches!(
      parsed.segments.as_slice(),
      [
        Segment::SOI { .. },
        Segment::Unknown { marker: 0xFFD0, .. },
        Segment::Unknown { marker: 0xFF01, .. },
        Segment::EOI
      ]
    ));
  }

  #[test]
  fn invalid_segments_end_the_parse() {
    // Lengths of 0 and 1 have no room for the length field
    for len in [0, 1] {
      let parsed = Jfif::parse(Cursor::new(jpeg(0xE2, len, &[]))).expect("parsed");
      assert!(matches!(parsed.segments.as_slice(), [Segment::SOI { .. }]));
    }
    // A segment of 4 bytes, then bytes that start no marker
    let mut data = jpeg(0xE2, 4, &[1, 2]);
    data.truncate(data.len() - 2);
    data.extend_from_slice(&[0x12, 0x34, 0xFF, 0xD9]);
    let parsed = Jfif::parse(Cursor::new(data)).expect("parsed");
    assert!(matches!(
      parsed.segments.as_slice(),
      [Segment::SOI { .. }, Segment::Unknown { marker: 0xFFE2, .. }]
    ));
  }

  #[test]
  fn first_marker_must_be_soi() {
    let mut data = jpeg(0xE1, 2, &[]);
    data.drain(0..2);
    assert!(matches!(Jfif::parse(Cursor::new(data)), Err(JfifError::FormatMismatch(_))));
    assert!(Jfif::parse(Cursor::new([0xFF])).is_err());
  }

  #[test]
  fn truncated_app1_does_not_loop() {
    let bytes = [0xFF, 0xD8, 0xFF, 0xE1, 0x33];
    let parsed = Jfif::parse(Cursor::new(bytes)).expect("SOI remains a valid parsed segment");
    assert!(matches!(parsed.segments.as_slice(), [Segment::SOI { .. }]));
  }
}
