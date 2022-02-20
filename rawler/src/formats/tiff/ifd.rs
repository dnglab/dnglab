// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::{
  apply_corr,
  entry::RawEntry,
  read_from_file,
  reader::{EndianReader, ReadByteOrder},
  Entry, Result, TiffError, Value,
};
use crate::{
  bits::Endian,
  tags::{ExifTag, LegacyTiffRootTag, TiffTagEnum},
};
use log::{warn, debug};
use serde::{Deserialize, Serialize};
use std::{
  collections::{BTreeMap, HashMap},
  io::{Read, Seek, SeekFrom},
};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct IFD {
  pub offset: u32,
  pub base: u32,
  pub corr: i32,
  pub next_ifd: u32,
  pub entries: BTreeMap<u16, Entry>,
  pub endian: Endian,
  pub sub: HashMap<u16, Vec<IFD>>,
}

impl IFD {
  pub fn new<R: Read + Seek>(reader: &mut R, offset: u32, base: u32, corr: i32, endian: Endian, sub_tags: &[u16]) -> Result<IFD> {
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

      if sub_tags.contains(&tag) {
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
        let ifd = Self::new(reader, apply_corr(offset, corr), base, corr, endian, sub_tags)?;
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

  /*
  pub fn new<R: Read + Seek>(reader: &mut R, offset: u32, base: u32, corr: i32, endian: Endian, sub_tags: &[u16]) -> Result<Self> {
    reader.seek(SeekFrom::Start((base + offset) as u64))?;
    let mut sub_ifd_offsets = Vec::new();
    let mut reader = EndianReader::new(reader, endian);
    let entry_count = reader.read_u16()?;
    let mut entries = BTreeMap::new();
    let mut sub = Vec::new();
    for _ in 0..entry_count {
      //let embedded = reader.read_u32()?;
      let tag = reader.read_u16()?;
      if tag == LegacyTiffRootTag::SubIFDs.into() || sub_tags.contains(&tag) {
        let entry = Entry::parse(&mut reader, base, corr, tag)?;
        match entry.value {
          Value::Long(offsets) => {
            sub_ifd_offsets.extend_from_slice(&offsets);
          }
          _ => {
            todo!()
          }
        }
      } else {
        let entry = Entry::parse(&mut reader, base, corr, tag)?;
        entries.insert(entry.tag, entry);
      }
    }
    let next_ifd = reader.read_u32()?;

    // Process SubIFDs
    let pos = reader.position()?;
    let reader = reader.into_inner();
    for offset in sub_ifd_offsets {
      let ifd = IFD::new(reader, apply_corr(offset, corr), base, corr, endian, sub_tags)?;
      sub.push(ifd);
    }
    EndianReader::new(reader, endian).goto(pos)?; // restore

    Ok(Self {
      offset,
      base,
      corr,
      next_ifd: if next_ifd == 0 { 0 } else { apply_corr(next_ifd, corr) },
      entries,
      endian,
      sub,
    })
  }
   */

  pub fn sub_ifds(&self) -> &HashMap<u16, Vec<IFD>> {
    &self.sub
  }

  pub fn entry_count(&self) -> u16 {
    self.entries.len() as u16
  }

  pub fn next_ifd(&self) -> u32 {
    self.next_ifd
  }

  pub fn entries(&self) -> &BTreeMap<u16, Entry> {
    &self.entries
  }

  pub fn get_entry<T: TiffTagEnum>(&self, tag: T) -> Option<&Entry> {
    self.entries.get(&tag.into())
  }

  pub fn get_entry_subs<T: TiffTagEnum>(&self, tag: T) -> Option<&Entry> {
    for subs in &self.sub {
      for ifd in subs.1 {
        if let Some(entry) = ifd.get_entry_recursive(tag) {
          return Some(entry);
        }
      }
    }
    None
  }

  pub fn get_entry_recursive<T: TiffTagEnum>(&self, tag: T) -> Option<&Entry> {
    self.entries.get(&tag.into()).or_else(|| self.get_entry_subs(tag))
  }

  pub fn get_entry_raw<'a, T: TiffTagEnum, R: Read + Seek>(&'a self, tag: T, file: &mut R) -> Result<Option<RawEntry>> {
    match self.get_entry(tag) {
      Some(entry) => {
        return Ok(Some(RawEntry {
          entry,
          endian: self.endian,
          data: read_from_file(file, self.base + entry.offset().unwrap() as u32, entry.byte_size())?,
        }))
      }
      None => {}
    }
    Ok(None)
  }

  /*
  pub fn get_ifd<T: TiffTagEnum, R: Read + Seek>(&self, tag: T, reader: &mut R) -> Result<Option<IFD>> {
    if let Some(offset) = self.get_entry(tag) {
      match &offset.value {
        Value::Long(v) => {
          debug!("IFD offset: {}", v[0]);
          Ok(Some(IFD::new(reader, apply_corr(v[0], self.corr), self.base, self.corr, self.endian, &[])?))
        }
        _ => {
          return Err(TiffError::General(format!(
            "TIFF tag {:?} is not of type LONG, thus can not be used as IFD offset in get_ifd().",
            tag
          )));
        }
      }
    } else {
      Ok(None)
    }
  }
   */

  pub fn has_entry<T: TiffTagEnum>(&self, tag: T) -> bool {
    self.get_entry(tag).is_some()
  }

  pub fn sub_buf<'a, R: Read + Seek>(&self, reader: &mut R, offset: usize, len: usize) -> Result<Vec<u8>> {
    //&buf[self.start_offset+offset..self.start_offset+offset+len]
    let mut buf = vec![0; len];
    reader.seek(SeekFrom::Start(self.base as u64 + offset as u64))?;
    reader.read_exact(&mut buf)?;
    Ok(buf)
  }

  pub fn contains_singlestrip_image(&self) -> bool {
    true // FIXME
  }

  pub fn singlestrip_data<'a, R: Read + Seek>(&self, reader: &mut R) -> Result<Vec<u8>> {
    assert!(self.contains_singlestrip_image());

    let offset = self.get_entry(LegacyTiffRootTag::StripOffsets).unwrap().value.get_usize(0)?;
    let len = self.get_entry(LegacyTiffRootTag::StripByteCounts).unwrap().value.get_usize(0)?;

    self.sub_buf(reader, offset, len)
  }

  pub fn parse_makernote<'a, R: Read + Seek>(&self, reader: &mut R) -> Result<Option<IFD>> {
    if let Some(exif) = self.get_entry(ExifTag::MakerNotes) {
      let offset = exif.offset().unwrap() as u32;
      debug!("Makernote offset: {}", offset);
      match &exif.value {
        Value::Undefined(data) => {
          let mut off = 0;
          let mut endian = self.endian;

          // Epson starts the makernote with its own name
          if data[0..5] == b"EPSON"[..] {
            off += 8;
          }

          // Pentax makernote starts with AOC\0 - If it's there, skip it
          if data[0..4] == b"AOC\0"[..] {
            off += 4;
          }

          // Pentax can also start with PENTAX and in that case uses different offsets
          if data[0..6] == b"PENTAX"[..] {
            off += 8;
            let endian = if data[off..off + 2] == b"II"[..] { Endian::Little } else { Endian::Big };
            return Ok(Some(IFD::new(reader, offset + 10, self.base, self.corr, endian, &[])?));
            //return TiffIFD::new(&buf[offset..], 10, base_offset, 0, depth, endian)
          }

          if data[0..7] == b"Nikon\0\x02"[..] {
            off += 10;
            let endian = if data[off..off + 2] == b"II"[..] { Endian::Little } else { Endian::Big };
            return Ok(Some(IFD::new(reader, offset + 8, self.base, self.corr, endian, &[])?));
          }

          // Some have MM or II to indicate endianness - read that
          if data[off..off + 2] == b"II"[..] {
            off += 2;
            endian = Endian::Little;
          }
          if data[off..off + 2] == b"MM"[..] {
            off += 2;
            endian = Endian::Big;
          }

          Ok(Some(IFD::new(reader, offset + off as u32, self.base, self.corr, endian, &[])?))
        }
        _ => Err(TiffError::General(format!("EXIF makernote has unknown type"))),
      }
    } else {
      Ok(None)
    }
  }


pub fn dump<T: TiffTagEnum>(&self, limit: usize) -> Vec<String> {
  let mut out = Vec::new();
  out.push(format!("IFD entries: {}\n", self.entries.len()));
  out.push(format!("{0:<34}  | {1:<10} | {2:<6} | {3}\n", "Tag", "Type", "Count", "Data"));
  for (tag, entry) in &self.entries {
    let mut line = String::new();
    let tag_name = {
      if let Ok(name) = T::try_from(*tag) {
        String::from(format!("{:?}", name))
      } else {
        format!("<?{}>", tag).into()
      }
    };
    line.push_str(&format!(
      "{0:#06x} : {0:<6} {1:<20}| {2:<10} | {3:<6} | ",
      tag,
      tag_name,
      entry.type_name(),
      entry.count()
    ));
    line.push_str(&entry.visual_rep(limit));
    out.push(line);
  }
  for subs in self.sub_ifds().iter() {
    for (i, sub) in subs.1.iter().enumerate() {
      out.push(format!("SubIFD({}:{})", subs.0, i));
      for line in sub.dump::<T>(limit) {
        out.push(format!("   {}", line));
      }
    }
  }
  out
}
}

