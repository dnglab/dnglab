// SPDX-License-Identifier: MIT
// Copyright 2020 Alfred Gutierrez
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::{
  fmt,
  fs::File,
  io::{self, Cursor, Read, Seek, SeekFrom},
};

use byteorder::{BigEndian, ReadBytesExt};
use log::debug;
use serde::{Serialize, Serializer, Deserialize, Deserializer};
use thiserror::Error;

pub mod co64;
pub mod dinf;
pub mod ext_cr3;
pub mod free;
pub mod ftyp;
pub mod hdlr;
pub mod mdat;
pub mod mdhd;
pub mod mdia;
pub mod minf;
pub mod moov;
pub mod mvhd;
pub mod skip;
pub mod stbl;
pub mod stsc;
pub mod stsd;
pub mod stsz;
pub mod stts;
pub mod tkhd;
pub mod trak;
pub mod vendor;
pub mod vmhd;

use ftyp::FtypBox;
use mdat::MdatBox;
use moov::MoovBox;
use uuid::Uuid;
use vendor::VendorBox;

use self::ext_cr3::cr3xpacket::Cr3XpacketBox;

type BoxUuid = Uuid;

pub const UUID_TYP: FourCC = FourCC::with(['u', 'u', 'i', 'd']);

pub fn read_box_header_ext<R: Read>(reader: &mut R) -> Result<(u8, u32)> {
  let version = reader.read_u8()?;
  let flags = reader.read_u24::<BigEndian>()?;
  Ok((version, flags))
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct BoxHeader {
  pub size: u64,
  pub typ: FourCC,
  pub uuid: Option<BoxUuid>,
  pub offset: u64, // File offset
  pub header_len: u64,
  // Full Box fields
  //pub version: Option<u8>,
  //pub flags: Option<u32>,
}

impl BoxHeader {
  pub fn parse<R: Read + Seek>(mut reader: R) -> Result<Self> {
    let start = reader.seek(SeekFrom::Current(0))?;
    let mut size = reader.read_u32::<BigEndian>()? as u64;
    let typ = reader.read_u32::<BigEndian>()?.into();
    if size == 1 {
      size = reader.read_u64::<BigEndian>()?;
    }
    let mut uuid = None;
    if typ == UUID_TYP {
      let mut buf = [0; 16];
      reader.read_exact(&mut buf)?;
      uuid = Some(Uuid::from_bytes(buf));
    }

    let current = reader.seek(SeekFrom::Current(0))?;
    Ok(BoxHeader {
      size,
      typ,
      uuid,
      offset: start,
      header_len: current - start,
    })
  }

  pub fn end_offset(&self) -> u64 {
    self.offset + self.size
  }

  pub fn make_view<'a>(&self, buffer: &'a [u8], skip: usize, limit: usize) -> &'a [u8] {
    let start = (self.offset + self.header_len) as usize + skip;
    if limit == 0 {
      &buffer[start..start + (self.size - self.header_len) as usize - skip]
    } else {
      &buffer[start..start + limit]
    }
  }
}

pub trait BmffBox: Sized {
  fn size(&self) -> u64;

  fn data_size(&self) -> u64;

  fn box_offset(&self) -> usize;

  fn box_data_offset(&self) -> usize;
}

#[derive(Error, Debug)]
pub enum BmffError {
  #[error("I/O error while writing DNG")]
  Io(#[from] io::Error),

  #[error("Parser error: {}", _0)]
  Parse(String),
}

type Result<T> = std::result::Result<T, BmffError>;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FileBox {
  pub ftyp: FtypBox,
  pub moov: MoovBox,
  pub mdat: MdatBox,
  pub vendor: Vec<VendorBox>,
  pub cr3xpacket: Option<Cr3XpacketBox>,
}

impl FileBox {
  pub fn parse<R: Read + Seek>(mut file: R) -> Result<FileBox> {
    let size = file.seek(SeekFrom::End(0))?;
    let mut current = file.seek(SeekFrom::Start(0))?;

    let mut ftyp = None;
    let mut moov = None;
    let mut mdat = None;
    let mut vendors = Vec::new();
    let mut cr3xpacket = None;

    while current < size {
      // get box?

      let header = BoxHeader::parse(&mut file)?;

      //let ftyp = Some(FtypBox::read_box(&mut file, header)?);

      match header.typ {
        FtypBox::TYP => {
          debug!("Found ftyp box");
          ftyp = Some(FtypBox::read_box(&mut file, header)?);
        }
        MoovBox::TYP => {
          debug!("Found moov box");
          moov = Some(MoovBox::read_box(&mut file, header)?);
        }
        MdatBox::TYP => {
          debug!("Found mdat box");
          mdat = Some(MdatBox::read_box(&mut file, header)?);
        }
        _ => {
          if let Some(uuid) = header.uuid {
            if uuid == Uuid::from_bytes(Cr3XpacketBox::UUID) {
              cr3xpacket = Some(Cr3XpacketBox::read_box(&mut file, header)?);
            } else {
              debug!("Vendor box found in filebox: {:?}", header.typ);
              let vendor = VendorBox::read_box(&mut file, header)?;
              vendors.push(vendor);
            }
          } else {
            debug!("Vendor box found in filebox: {:?}", header.typ);
            let vendor = VendorBox::read_box(&mut file, header)?;
            vendors.push(vendor);
          }
        }
      }

      current = file.seek(SeekFrom::Current(0))?;
    }

    Ok(Self {
      ftyp: ftyp.ok_or(BmffError::Parse("ftyp box not found, corrupt file?".into()))?,
      moov: moov.ok_or(BmffError::Parse("moov box not found, corrupt file?".into()))?,
      mdat: mdat.ok_or(BmffError::Parse("mdat box not found, corrupt file?".into()))?,
      vendor: vendors,
      cr3xpacket,
    })
  }
}

pub fn parse_file(file: &mut File) -> Result<FileBox> {
  let filebox = FileBox::parse(file)?;
  Ok(filebox)
}

pub fn parse_buffer(buf: &[u8]) -> Result<FileBox> {
  // TODO: add AsRef<u8>
  let mut cursor = Cursor::new(buf);
  let filebox = FileBox::parse(&mut cursor)?;
  Ok(filebox)
}

#[derive(Default, PartialEq, Eq, Clone, Copy)]
pub struct FourCC {
  pub value: [u8; 4],
}

impl Serialize for FourCC {
  fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
  where
    S: Serializer,
  {
    serializer.serialize_str(&self.to_string())
  }
}

impl<'de> Deserialize<'de> for FourCC {
  fn deserialize<D>(deserializer: D) -> std::result::Result<FourCC, D::Error>
  where
    D: Deserializer<'de>,
  {
    use serde::de::Error;
    let s = String::deserialize(deserializer)?;
    if s.len() != 4 {
      Err(D::Error::custom(format!("Invalid FourCC value: {}", s)))
    } else {
      Ok(FourCC {
        value: [s.as_bytes()[0], s.as_bytes()[1], s.as_bytes()[2], s.as_bytes()[3]],
      })
    }
  }
}

impl FourCC {
  const fn with(v: [char; 4]) -> Self {
    Self {
      value: [v[0] as u8, v[1] as u8, v[2] as u8, v[3] as u8],
    }
  }
}

impl From<u32> for FourCC {
  fn from(number: u32) -> Self {
    FourCC { value: number.to_be_bytes() }
  }
}

impl From<FourCC> for u32 {
  fn from(fourcc: FourCC) -> u32 {
    (&fourcc).into()
  }
}

impl From<&FourCC> for u32 {
  fn from(fourcc: &FourCC) -> u32 {
    u32::from_be_bytes(fourcc.value)
  }
}

impl std::str::FromStr for FourCC {
  type Err = BmffError;

  fn from_str(s: &str) -> Result<Self> {
    if let [a, b, c, d] = s.as_bytes() {
      Ok(Self { value: [*a, *b, *c, *d] })
    } else {
      Err(BmffError::Parse("expected exactly four bytes in string".into()))
    }
  }
}

impl From<[u8; 4]> for FourCC {
  fn from(value: [u8; 4]) -> FourCC {
    FourCC { value }
  }
}

impl fmt::Debug for FourCC {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    let code: u32 = self.into();
    let string = String::from_utf8_lossy(&self.value[..]);
    write!(f, "{} / {:#010X}", string, code)
  }
}

impl fmt::Display for FourCC {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    write!(f, "{}", String::from_utf8_lossy(&self.value[..]))
  }
}

pub trait ReadBox<T>: Sized {
  fn read_box(_: T, header: BoxHeader) -> Result<Self>;
}

#[derive(Clone, Debug)]
pub struct Bmff {
  pub filebox: FileBox,
}

impl Bmff {

  pub fn new<R: Read + Seek>(file: R) -> Result<Self> {
    let filebox = FileBox::parse(file)?;
    Ok(Self { filebox })
  }

  pub fn new_buf(buf: &[u8]) -> Result<Self> {
    let filebox = parse_buffer(&buf)?;
    Ok(Self { filebox })
  }

  pub fn compatible_brand(&self, _brand: &str) -> bool {
    true // FIXME
  }
}
