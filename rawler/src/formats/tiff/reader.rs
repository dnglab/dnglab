// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::{apply_corr, entry::RawEntry, file::TiffFile, Entry, Result, TiffError, Value, IFD};
use crate::{
  bits::Endian,
  tags::{LegacyTiffRootTag, TiffTagEnum},
};
use byteorder::{BigEndian, LittleEndian, ReadBytesExt};
use log::warn;
use serde::{Deserialize, Serialize};
use std::{
  collections::{BTreeMap, HashMap},
  io::{Cursor, Read, Seek, SeekFrom},
};

pub trait TiffReader {
  fn file(&self) -> &TiffFile;
  fn file_mut(&mut self) -> &mut TiffFile;

  fn chains(&self) -> &Vec<IFD> {
    &self.file().chain
  }

  fn sub_ifd_tags(&self) -> Vec<u16> {
    vec![LegacyTiffRootTag::SubIFDs.into(), LegacyTiffRootTag::ExifIFDPointer.into()]
  }

  fn root_ifd(&self) -> &IFD {
    if self.file().chain.is_empty() {
      panic!("TIFF must have at least one root IFD but the IFD list is empty");
    }
    &self.file().chain[0]
  }

  fn get_entry<T: TiffTagEnum>(&self, tag: T) -> Option<&Entry> {
    for ifd in &self.file().chain {
      match ifd.get_entry(tag) {
        Some(x) => return Some(x),
        None => {}
      }
    }
    None
  }

  fn get_entry_raw<'a, T: TiffTagEnum, R: Read + Seek>(&'a self, tag: T, file: &mut R) -> Result<Option<RawEntry>> {
    for ifd in &self.file().chain {
      match ifd.get_entry_raw(tag, file)? {
        Some(entry) => return Ok(Some(entry)),
        None => {}
      }
    }
    Ok(None)
  }

  fn has_entry<T: TiffTagEnum>(&self, tag: T) -> bool {
    self.get_entry(tag).is_some()
  }

  fn find_ifds_with_tag<T: TiffTagEnum>(&self, tag: T) -> Vec<&IFD> {
    let mut ifds = Vec::new();
    for ifd in &self.file().chain {
      if ifd.has_entry(tag) {
        ifds.push(ifd);
      } else {
        for subs in ifd.sub_ifds() {
          for ifd in subs.1 {
            if ifd.has_entry(tag) {
              ifds.push(ifd);
            }
          }
        }
      }
    }
    ifds
  }

  // TODO: legacy wrapper
  fn find_first_ifd<T: TiffTagEnum>(&self, tag: T) -> Option<&IFD> {
    self.find_first_ifd_with_tag(tag)
  }

  fn find_first_ifd_with_tag<T: TiffTagEnum>(&self, tag: T) -> Option<&IFD> {
    let ifds = self.find_ifds_with_tag(tag);
    if ifds.len() == 0 {
      None
    } else {
      Some(ifds[0])
    }
  }

  fn get_first_entry(&self, _tag: u16) -> Option<Entry> {
    unimplemented!();
    /*
    Some(Entry {
      value: (32 as u16).into(),
      embedded: None,
    })
     */
  }

  fn read_data<R: Read + Seek>(&self, file: &mut R, uncorr_offset: u32, buffer: &mut [u8]) -> Result<()> {
    file.seek(SeekFrom::Start(apply_corr(uncorr_offset, self.file().corr) as u64))?;
    file.read_exact(buffer)?;
    Ok(())
  }

  fn parse_ifd<R: Read + Seek>(&self, reader: &mut R, offset: u32, base: u32, corr: i32, endian: Endian, sub_tags: &[u16]) -> Result<IFD> {
    reader.seek(SeekFrom::Start((base + offset) as u64))?;
    let mut sub_ifd_offsets = HashMap::new();
    let mut reader = EndianReader::new(reader, endian);
    let entry_count = reader.read_u16()?;
    let mut entries = BTreeMap::new();
    let mut sub = HashMap::new();
    for _ in 0..entry_count {
      //let embedded = reader.read_u32()?;
      let tag = reader.read_u16()?;
      let entry = Entry::parse(&mut reader, base, corr, tag)?;
      if self.sub_ifd_tags().contains(&tag) || sub_tags.contains(&tag) {
        //let entry = Entry::parse(&mut reader, base, corr, tag)?;
        match &entry.value {
          Value::Long(offsets) => {
            sub_ifd_offsets.insert(tag, offsets.clone());
            //sub_ifd_offsets.extend_from_slice(&offsets);
          }
          _ => {
            todo!()
          }
        }
      }
      entries.insert(entry.tag, entry);
    }

    // Some TIFF writers skip the next ifd pointer
    // If we get an I/O error, we fallback to 0, signaling the end of IFD chains.
    let next_ifd = match reader.read_u32() {
      Ok(ptr) => ptr,
      Err(e) => {
        warn!("TIFF IFD reader failed to get next IFD pointer, fallback to 0. Error was: {}", e);
        0
      }
    };

    // Process SubIFDs
    let pos = reader.position()?;
    let reader = reader.into_inner();
    for subs in sub_ifd_offsets {
      let mut ifds = Vec::new();
      for offset in subs.1 {
        let ifd = self.parse_ifd(reader, apply_corr(offset, corr), base, corr, endian, sub_tags)?;
        ifds.push(ifd);
      }
      sub.insert(subs.0, ifds);
    }
    EndianReader::new(reader, endian).goto(pos)?; // restore
    Ok(IFD {
      offset,
      base,
      corr,
      next_ifd: if next_ifd == 0 { 0 } else { apply_corr(next_ifd, corr) },
      entries,
      endian,
      sub,
    })
  }

  /// Construct a TIFF reader from Read capable objects
  ///
  /// `corr` is a correction value that should be applied to offsets received
  /// from file structure.
  fn parse_file<R: Read + Seek>(&mut self, file: &mut R, max_chained: Option<usize>, sub_tags: &[u16]) -> Result<()> {
    let endian = match file.read_u16::<LittleEndian>()? {
      0x4949 => Endian::Little,
      0x4d4d => Endian::Big,
      x => {
        return Err(TiffError::General(format!("TIFF: don't know marker 0x{:x}", x)));
      }
    };
    let mut reader = EndianReader::new(file, endian);
    let magic = reader.read_u16()?;
    if magic != 42 {
      return Err(TiffError::General(format!("Invalid magic marker for TIFF: {}", magic)));
    }
    let mut next_ifd = reader.read_u32()?;
    if next_ifd == 0 {
      return Err(TiffError::General(format!("Invalid TIFF header, contains no root IFD")));
    }

    let reader = reader.into_inner();

    next_ifd = apply_corr(next_ifd, self.file().corr);
    let mut chain = Vec::new();
    while next_ifd != 0 {
      // TODO: check if offset is in range
      let ifd = self.parse_ifd(reader, next_ifd, self.file().base, self.file().corr, endian, sub_tags)?;
      if ifd.entries.is_empty() {
        return Err(TiffError::General(format!("TIFF is invalid, IFD must contain at least one entry")));
      }
      next_ifd = ifd.next_ifd;
      chain.push(ifd);
      if let Some(max) = max_chained {
        if chain.len() > max {
          break;
        }
      }
    }

    if chain.is_empty() {
      return Err(TiffError::General(format!("TIFF is invalid, must contain at least one IFD")));
    }
    self.file_mut().chain = chain;
    Ok(())
  }
}

/// Reader for TIFF files
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct GenericTiffReader {
  file: TiffFile,
}

impl TiffReader for GenericTiffReader {
  fn file(&self) -> &TiffFile {
    &self.file
  }

  fn file_mut(&mut self) -> &mut TiffFile {
    &mut self.file
  }
}

impl GenericTiffReader {
  /// Check if buffer looks like a TIFF file
  pub fn is_tiff<T: AsRef<[u8]>>(buffer: T) -> bool {
    let buffer = buffer.as_ref();
    buffer[0] == 0x49 || buffer[0] == 0x4d // TODO
  }

  /// Construct a TIFF reader from a byte buffer
  ///
  /// Byte buffer must be a full TIFF file structure, endianess is detected from TIFF
  /// header.
  ///
  /// `corr` is a correction value that should be applied to offsets received
  /// from file structure.
  pub fn new_with_buffer<T: AsRef<[u8]>>(buffer: T, base: u32, corr: i32, max_chained: Option<usize>) -> Result<Self> {
    let mut cursor = Cursor::new(buffer.as_ref());
    cursor.seek(SeekFrom::Start(base as u64))?;
    Self::new(&mut cursor, base, corr, max_chained, &[])
  }

  /// Construct a TIFF reader from Read capable objects
  ///
  /// `corr` is a correction value that should be applied to offsets received
  /// from file structure.
  pub fn new<R: Read + Seek>(file: &mut R, base: u32, corr: i32, max_chained: Option<usize>, sub_tags: &[u16]) -> Result<Self> {
    let mut ins = Self {
      file: TiffFile::new(base, corr),
    };
    ins.parse_file(file, max_chained, sub_tags)?;
    Ok(ins)
  }
}

pub trait ReadByteOrder {
  fn read_u8(&mut self) -> std::io::Result<u8>;
  fn read_i8(&mut self) -> std::io::Result<i8>;
  fn read_u16(&mut self) -> std::io::Result<u16>;
  fn read_i16(&mut self) -> std::io::Result<i16>;
  fn read_u32(&mut self) -> std::io::Result<u32>;
  fn read_i32(&mut self) -> std::io::Result<i32>;
  fn read_u64(&mut self) -> std::io::Result<u64>;
  fn read_i64(&mut self) -> std::io::Result<i64>;
  fn read_f32(&mut self) -> std::io::Result<f32>;
  fn read_f64(&mut self) -> std::io::Result<f64>;

  fn read_u8_into(&mut self, dst: &mut [u8]) -> std::io::Result<()>;
  fn read_i8_into(&mut self, dst: &mut [i8]) -> std::io::Result<()>;
  fn read_u16_into(&mut self, dst: &mut [u16]) -> std::io::Result<()>;
  fn read_i16_into(&mut self, dst: &mut [i16]) -> std::io::Result<()>;
  fn read_u32_into(&mut self, dst: &mut [u32]) -> std::io::Result<()>;
  fn read_i32_into(&mut self, dst: &mut [i32]) -> std::io::Result<()>;
  fn read_u64_into(&mut self, dst: &mut [u64]) -> std::io::Result<()>;
  fn read_i64_into(&mut self, dst: &mut [i64]) -> std::io::Result<()>;
  fn read_f32_into(&mut self, dst: &mut [f32]) -> std::io::Result<()>;
  fn read_f64_into(&mut self, dst: &mut [f64]) -> std::io::Result<()>;
}

pub struct EndianReader<'a, R: Read + Seek + 'a> {
  endian: Endian,
  inner: &'a mut R,
}

impl<'a, R: Read + Seek + 'a> EndianReader<'a, R> {
  pub fn new(inner: &'a mut R, endian: Endian) -> Self {
    Self { endian, inner }
  }

  pub fn into_inner(self) -> &'a mut R {
    self.inner
  }

  pub fn inner(&'a mut self) -> &'a mut R {
    self.inner
  }

  pub fn position(&mut self) -> Result<u32> {
    Ok(self.inner.stream_position().map(|v| v as u32)?)
  }

  // TODO: try_from?

  pub fn goto(&mut self, offset: u32) -> Result<()> {
    self.inner.seek(SeekFrom::Start(offset as u64))?;
    Ok(())

    // TODO: try_from?
  }
}

impl<'a, R: Read + Seek + 'a> ReadByteOrder for EndianReader<'a, R> {
  fn read_u16(&mut self) -> std::io::Result<u16> {
    match self.endian {
      Endian::Little => self.inner.read_u16::<LittleEndian>(),
      Endian::Big => self.inner.read_u16::<BigEndian>(),
    }
  }

  fn read_u8(&mut self) -> std::io::Result<u8> {
    self.inner.read_u8()
  }

  fn read_i8(&mut self) -> std::io::Result<i8> {
    match self.endian {
      Endian::Little => self.inner.read_i8(),
      Endian::Big => self.inner.read_i8(),
    }
  }

  fn read_i16(&mut self) -> std::io::Result<i16> {
    match self.endian {
      Endian::Little => self.inner.read_i16::<LittleEndian>(),
      Endian::Big => self.inner.read_i16::<BigEndian>(),
    }
  }

  fn read_u32(&mut self) -> std::io::Result<u32> {
    match self.endian {
      Endian::Little => self.inner.read_u32::<LittleEndian>(),
      Endian::Big => self.inner.read_u32::<BigEndian>(),
    }
  }

  fn read_i32(&mut self) -> std::io::Result<i32> {
    match self.endian {
      Endian::Little => self.inner.read_i32::<LittleEndian>(),
      Endian::Big => self.inner.read_i32::<BigEndian>(),
    }
  }

  fn read_u64(&mut self) -> std::io::Result<u64> {
    match self.endian {
      Endian::Little => self.inner.read_u64::<LittleEndian>(),
      Endian::Big => self.inner.read_u64::<BigEndian>(),
    }
  }

  fn read_i64(&mut self) -> std::io::Result<i64> {
    match self.endian {
      Endian::Little => self.inner.read_i64::<LittleEndian>(),
      Endian::Big => self.inner.read_i64::<BigEndian>(),
    }
  }

  fn read_f32(&mut self) -> std::io::Result<f32> {
    match self.endian {
      Endian::Little => self.inner.read_f32::<LittleEndian>(),
      Endian::Big => self.inner.read_f32::<BigEndian>(),
    }
  }

  fn read_f64(&mut self) -> std::io::Result<f64> {
    match self.endian {
      Endian::Little => self.inner.read_f64::<LittleEndian>(),
      Endian::Big => self.inner.read_f64::<BigEndian>(),
    }
  }

  fn read_u8_into(&mut self, dst: &mut [u8]) -> std::io::Result<()> {
    self.inner.read_exact(dst)
  }

  fn read_i8_into(&mut self, dst: &mut [i8]) -> std::io::Result<()> {
    self.inner.read_i8_into(dst)
  }

  fn read_u16_into(&mut self, dst: &mut [u16]) -> std::io::Result<()> {
    match self.endian {
      Endian::Little => self.inner.read_u16_into::<LittleEndian>(dst),
      Endian::Big => self.inner.read_u16_into::<BigEndian>(dst),
    }
  }

  fn read_i16_into(&mut self, dst: &mut [i16]) -> std::io::Result<()> {
    match self.endian {
      Endian::Little => self.inner.read_i16_into::<LittleEndian>(dst),
      Endian::Big => self.inner.read_i16_into::<BigEndian>(dst),
    }
  }

  fn read_u32_into(&mut self, dst: &mut [u32]) -> std::io::Result<()> {
    match self.endian {
      Endian::Little => self.inner.read_u32_into::<LittleEndian>(dst),
      Endian::Big => self.inner.read_u32_into::<BigEndian>(dst),
    }
  }

  fn read_i32_into(&mut self, dst: &mut [i32]) -> std::io::Result<()> {
    match self.endian {
      Endian::Little => self.inner.read_i32_into::<LittleEndian>(dst),
      Endian::Big => self.inner.read_i32_into::<BigEndian>(dst),
    }
  }

  fn read_u64_into(&mut self, dst: &mut [u64]) -> std::io::Result<()> {
    match self.endian {
      Endian::Little => self.inner.read_u64_into::<LittleEndian>(dst),
      Endian::Big => self.inner.read_u64_into::<BigEndian>(dst),
    }
  }

  fn read_i64_into(&mut self, dst: &mut [i64]) -> std::io::Result<()> {
    match self.endian {
      Endian::Little => self.inner.read_i64_into::<LittleEndian>(dst),
      Endian::Big => self.inner.read_i64_into::<BigEndian>(dst),
    }
  }

  fn read_f32_into(&mut self, dst: &mut [f32]) -> std::io::Result<()> {
    match self.endian {
      Endian::Little => self.inner.read_f32_into::<LittleEndian>(dst),
      Endian::Big => self.inner.read_f32_into::<BigEndian>(dst),
    }
  }

  fn read_f64_into(&mut self, dst: &mut [f64]) -> std::io::Result<()> {
    match self.endian {
      Endian::Little => self.inner.read_f64_into::<LittleEndian>(dst),
      Endian::Big => self.inner.read_f64_into::<BigEndian>(dst),
    }
  }
}
