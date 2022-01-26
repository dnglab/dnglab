// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::{entry::RawEntry, read_from_file, Entry, Result};
use crate::{
  bits::Endian,
  tags::{LegacyTiffRootTag, TiffTagEnum},
};
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
}

pub fn ifd_dump<T: TiffTagEnum>(ifd: &IFD, limit: usize) -> Vec<String> {
  let mut out = Vec::new();
  out.push(format!("IFD entries: {}\n", ifd.entries.len()));
  out.push(format!("{0:<34}  | {1:<10} | {2:<6} | {3}\n", "Tag", "Type", "Count", "Data"));
  for (tag, entry) in &ifd.entries {
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
  for subs in ifd.sub_ifds().iter() {
    for (i, sub) in subs.1.iter().enumerate() {
      out.push(format!("SubIFD({}:{})", subs.0, i));
      for line in ifd_dump::<T>(sub, limit) {
        out.push(format!("   {}", line));
      }
    }
  }
  out
}
