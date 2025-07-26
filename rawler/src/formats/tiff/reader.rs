// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::{Entry, IFD, Result, TiffError, apply_corr, entry::RawEntry, file::TiffFile};
use crate::{
  bits::Endian,
  tags::{ExifTag, TiffCommonTag, TiffTag},
};
use byteorder::{BigEndian, LittleEndian, ReadBytesExt};
use serde::{Deserialize, Serialize};
use std::io::{Cursor, Read, Seek, SeekFrom};

pub trait TiffReader {
  fn file(&self) -> &TiffFile;
  fn file_mut(&mut self) -> &mut TiffFile;

  fn chains(&self) -> &Vec<IFD> {
    &self.file().chain
  }

  fn get_endian(&self) -> Endian {
    self.root_ifd().endian
  }

  /// Returns a list of well-known tags representing SubIFDs.
  fn wellknown_sub_ifd_tags(&self) -> Vec<u16> {
    vec![
      TiffCommonTag::SubIFDs.into(),
      TiffCommonTag::ExifIFDPointer.into(),
      ExifTag::GPSInfo.into(),
      ExifTag::IccProfile.into(),
    ]
  }

  fn root_ifd(&self) -> &IFD {
    if self.file().chain.is_empty() {
      panic!("TIFF must have at least one root IFD but the IFD list is empty");
    }
    &self.file().chain[0]
  }

  fn get_entry<T: TiffTag>(&self, tag: T) -> Option<&Entry> {
    for ifd in &self.file().chain {
      if let Some(x) = ifd.get_entry(tag) {
        return Some(x);
      }
    }
    None
  }

  fn get_entry_raw<'a, T: TiffTag, R: Read + Seek>(&'a self, tag: T, file: &mut R) -> Result<Option<RawEntry<'a>>> {
    for ifd in &self.file().chain {
      if let Some(entry) = ifd.get_entry_raw(tag, file)? {
        return Ok(Some(entry));
      }
    }
    Ok(None)
  }

  fn has_entry<T: TiffTag>(&self, tag: T) -> bool {
    self.get_entry(tag).is_some()
  }

  fn find_ifds_with_tag<T: TiffTag>(&self, tag: T) -> Vec<&IFD> {
    let mut ifds = Vec::new();
    for ifd in &self.file().chain {
      if ifd.has_entry(tag) {
        ifds.push(ifd);
      }
      // Now search in all sub IFDs
      for subs in ifd.sub_ifds() {
        for ifd in subs.1 {
          if ifd.has_entry(tag) {
            ifds.push(ifd);
          }
        }
      }
    }
    ifds
  }

  fn find_ifd_with_new_subfile_type(&self, typ: u32) -> Option<&IFD> {
    let list = self.find_ifds_with_tag(TiffCommonTag::NewSubFileType);
    list
      .iter()
      .find(|ifd| ifd.get_entry(TiffCommonTag::NewSubFileType).expect("IFD must contain this entry").force_u32(0) == typ)
      .copied()
  }

  // TODO: legacy wrapper
  fn find_first_ifd<T: TiffTag>(&self, tag: T) -> Option<&IFD> {
    self.find_first_ifd_with_tag(tag)
  }

  fn find_first_ifd_with_tag<T: TiffTag>(&self, tag: T) -> Option<&IFD> {
    let ifds = self.find_ifds_with_tag(tag);
    if ifds.is_empty() { None } else { Some(ifds[0]) }
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
    IFD::new(reader, offset, base, corr, endian, sub_tags)
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
      //return Err(TiffError::General(format!("Invalid magic marker for TIFF: {}", magic)));
    }
    let mut next_ifd = reader.read_u32()?;
    if next_ifd == 0 {
      return Err(TiffError::General("Invalid TIFF header, contains no root IFD".to_string()));
    }

    let reader = reader.into_inner();

    next_ifd = apply_corr(next_ifd, self.file().corr);
    let mut chain = Vec::new();
    while next_ifd != 0 {
      // TODO: check if offset is in range
      let mut multi_sub_tags = self.wellknown_sub_ifd_tags();
      multi_sub_tags.extend_from_slice(sub_tags);
      let ifd = IFD::new(reader, next_ifd, self.file().base, self.file().corr, endian, &multi_sub_tags)?;
      if ifd.entries.is_empty() {
        return Err(TiffError::General("TIFF is invalid, IFD must contain at least one entry".to_string()));
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
      return Err(TiffError::General("TIFF is invalid, must contain at least one IFD".to_string()));
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

  pub fn little_endian(&self) -> bool {
    self.file.chain.first().unwrap().endian == Endian::Little
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
