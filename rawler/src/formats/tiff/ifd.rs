// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::{
  Entry, Result, TiffError, Value, apply_corr,
  entry::RawEntry,
  read_from_file,
  reader::{EndianReader, ReadByteOrder},
};
use crate::{
  bits::Endian,
  rawsource::RawSource,
  tags::{ExifTag, TiffCommonTag, TiffTag},
};
use byteorder::{LittleEndian, ReadBytesExt};
use log::debug;
use serde::{Deserialize, Serialize};
use std::{
  collections::{BTreeMap, HashMap},
  io::{Read, Seek, SeekFrom},
};

#[derive(Debug)]
pub enum OffsetMode {
  Absolute,
  RelativeToIFD,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct IFD {
  pub offset: u32,
  pub base: u32,
  pub corr: i32,
  pub next_ifd: u32,
  pub entries: BTreeMap<u16, Entry>,
  pub endian: Endian,
  pub sub: HashMap<u16, Vec<IFD>>,
  pub chain: Vec<IFD>,
}

// TODO: fixme
impl IFD {
  /// Construct new IFD from reader at specific base
  pub fn new_root<R: Read + Seek>(reader: &mut R, base: u32) -> Result<IFD> {
    Self::new_root_with_correction(reader, 0, base, 0, 10, &[TiffCommonTag::SubIFDs.into(), TiffCommonTag::ExifIFDPointer.into()])
  }

  pub fn new_root_with_correction<R: Read + Seek>(reader: &mut R, offset: u32, base: u32, corr: i32, max_chain: usize, sub_tags: &[u16]) -> Result<IFD> {
    reader.seek(SeekFrom::Start((base + offset) as u64))?;
    let endian = match reader.read_u16::<LittleEndian>()? {
      0x4949 => Endian::Little,
      0x4d4d => Endian::Big,
      x => {
        return Err(TiffError::General(format!("TIFF: don't know marker 0x{:x}", x)));
      }
    };
    let mut reader = EndianReader::new(reader, endian);
    let magic = reader.read_u16()?;
    if magic != 42 {
      return Err(TiffError::General(format!("Invalid magic marker for TIFF: {}", magic)));
    }
    let mut next_ifd = reader.read_u32()?;
    if next_ifd == 0 {
      return Err(TiffError::General("Invalid TIFF header, contains no root IFD".to_string()));
    }

    let reader = reader.into_inner();
    let mut multi_sub_tags = vec![];
    multi_sub_tags.extend_from_slice(sub_tags);

    next_ifd = apply_corr(next_ifd, corr);
    let mut root = IFD::new(reader, next_ifd, base, corr, endian, &multi_sub_tags)?;
    if root.entries.is_empty() {
      return Err(TiffError::General("TIFF is invalid, IFD must contain at least one entry".to_string()));
    }
    next_ifd = root.next_ifd;

    while next_ifd != 0 {
      next_ifd = apply_corr(next_ifd, corr);
      let ifd = IFD::new(reader, next_ifd, base, corr, endian, &multi_sub_tags)?;
      if ifd.entries.is_empty() {
        return Err(TiffError::General("TIFF is invalid, IFD must contain at least one entry".to_string()));
      }
      next_ifd = ifd.next_ifd;
      root.chain.push(ifd);

      if root.chain.len() > max_chain && max_chain > 0 {
        break;
      }
    }

    Ok(root)
  }

  pub fn new<R: Read + Seek>(reader: &mut R, offset: u32, base: u32, corr: i32, endian: Endian, sub_tags: &[u16]) -> Result<IFD> {
    reader.seek(SeekFrom::Start((base + offset) as u64))?;
    let mut sub_ifd_offsets = HashMap::new();
    let mut reader = EndianReader::new(reader, endian);
    let entry_count = reader.read_u16()?;
    let mut entries = BTreeMap::new();
    let mut sub = HashMap::new();
    let mut next_pos = reader.position()?;
    debug!("Parse entries");
    for _ in 0..entry_count {
      reader.goto(next_pos)?;
      next_pos += 12;
      //let embedded = reader.read_u32()?;
      let tag = reader.read_u16()?;

      match Entry::parse(&mut reader, base, corr, tag) {
        Ok(entry) => {
          if sub_tags.contains(&tag) {
            //let entry = Entry::parse(&mut reader, base, corr, tag)?;
            match &entry.value {
              Value::Long(offsets) => {
                sub_ifd_offsets.insert(tag, offsets.clone());
                //sub_ifd_offsets.extend_from_slice(&offsets);
              }
              Value::Unknown(tag, offsets) => {
                sub_ifd_offsets.insert(*tag, vec![offsets[0] as u32]);
                //sub_ifd_offsets.extend_from_slice(&offsets);
              }
              Value::Undefined(_) => {
                sub_ifd_offsets.insert(tag, vec![entry.offset().unwrap() as u32]);
              }
              val => {
                log::info!(
                  "Found IFD offset tag, but type mismatch: {:?}. Ignoring SubIFD parsing for tag 0x{:X}",
                  val,
                  tag
                );
              }
            }
          }
          entries.insert(entry.tag, entry);
        }
        Err(err) => {
          log::info!("Failed to parse TIFF tag 0x{:X}, skipping: {:?}", tag, err);
        }
      }
    }

    // Some TIFF writers skip the next ifd pointer
    // If we get an I/O error, we fallback to 0, signaling the end of IFD chains.
    let next_ifd = match reader.read_u32() {
      Ok(ptr) => ptr,
      Err(e) => {
        debug!(
          "TIFF IFD reader failed to get next IFD pointer, fallback to 0 and continue. Original error was: {}",
          e
        );
        0
      }
    };

    // Process SubIFDs
    let pos = reader.position()?;
    let reader = reader.into_inner();
    for subs in sub_ifd_offsets {
      let mut ifds = Vec::new();
      for offset in subs.1 {
        match Self::new(reader, apply_corr(offset, corr), base, corr, endian, &[]) {
          Ok(ifd) => ifds.push(ifd),
          Err(err) => {
            log::warn!("Error while processing TIFF sub-IFD for tag 0x{:X}, ignoring it: {}", subs.0, err);
          }
        };
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
      chain: vec![],
    })
  }

  pub fn copy_tag(dst: &mut Self, src: &Self, tag: impl Into<u16>) {
    if let Some(entry) = src.get_entry(tag.into()) {
      dst.entries.insert(entry.tag, entry.clone());
    }
  }

  pub fn value_iter(&self) -> impl Iterator<Item = (&u16, &Value)> {
    self.entries().iter().map(|(tag, entry)| (tag, &entry.value))
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

  /// Extend the IFD with sub-IFDs from a specific tag.
  /// The IFD corrections are used from current IFD.
  pub fn extend_sub_ifds<R: Read + Seek>(&mut self, reader: &mut R, tag: u16) -> Result<Option<&Vec<Self>>> {
    if let Some(entry) = self.get_entry(tag) {
      let mut subs = Vec::new();
      match &entry.value {
        Value::Long(offsets) => {
          for off in offsets {
            let ifd = Self::new_root_with_correction(reader, *off, self.base, self.corr, 10, &[])?;
            subs.push(ifd);
          }
          self.sub.insert(tag, subs);
          Ok(self.sub.get(&tag))
        }
        val => {
          debug!("Found IFD offset tag, but type mismatch: {:?}", val);
          todo!()
        }
      }
    } else {
      Ok(None)
    }
  }

  pub fn extend_sub_ifds_custom<R, F>(&mut self, reader: &mut R, tag: u16, op: F) -> Result<Option<&Vec<Self>>>
  where
    R: Read + Seek,
    F: FnOnce(&mut R, &IFD, &Entry) -> Result<Option<Vec<IFD>>>,
  {
    if let Some(entry) = self.get_entry(tag) {
      if let Some(subs) = op(reader, self, entry)? {
        self.sub.insert(tag, subs);
        Ok(self.sub.get(&tag))
      } else {
        Ok(None)
      }
    } else {
      Ok(None)
    }
  }

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

  pub fn get_entry<T: TiffTag>(&self, tag: T) -> Option<&Entry> {
    self.entries.get(&tag.into())
  }

  pub fn get_entry_subs<T: TiffTag>(&self, tag: T) -> Option<&Entry> {
    for subs in &self.sub {
      for ifd in subs.1 {
        if let Some(entry) = ifd.get_entry_recursive(tag) {
          return Some(entry);
        }
      }
    }
    None
  }

  pub fn get_entry_recursive<T: TiffTag>(&self, tag: T) -> Option<&Entry> {
    self.entries.get(&tag.into()).or_else(|| self.get_entry_subs(tag))
  }

  pub fn get_entry_raw<'a, T: TiffTag, R: Read + Seek>(&'a self, tag: T, file: &mut R) -> Result<Option<RawEntry<'a>>> {
    if let Some(entry) = self.get_entry(tag) {
      return Ok(Some(RawEntry {
        entry,
        endian: self.endian,
        data: read_from_file(file, self.base + entry.offset().unwrap() as u32, entry.byte_size())?,
      }));
    }
    Ok(None)
  }

  /// Get the data of a tag by just reading as many `len` bytes from offet.
  pub fn get_entry_raw_with_len<'a, T: TiffTag, R: Read + Seek>(&'a self, tag: T, file: &mut R, len: usize) -> Result<Option<RawEntry<'a>>> {
    if let Some(entry) = self.get_entry(tag) {
      return Ok(Some(RawEntry {
        entry,
        endian: self.endian,
        data: read_from_file(file, self.base + entry.offset().unwrap() as u32, len)?,
      }));
    }
    Ok(None)
  }

  pub fn get_sub_ifd_all<T: TiffTag>(&self, tag: T) -> Option<&Vec<IFD>> {
    self.sub.get(&tag.into())
  }

  pub fn get_sub_ifd<T: TiffTag>(&self, tag: T) -> Option<&IFD> {
    if let Some(ifds) = self.get_sub_ifd_all(tag) {
      if ifds.len() == 1 {
        ifds.get(0)
      } else {
        log::warn!(
          "get_sub_ifd() for tag {:?} found more IFDs than expected: {}. Fallback to first IFD!",
          tag,
          ifds.len()
        );
        ifds.get(0)
      }
    } else {
      None
    }
  }

  pub fn find_ifds_with_tag<T: TiffTag>(&self, tag: T) -> Vec<&IFD> {
    let mut ifds = Vec::new();
    if self.get_entry(tag).is_some() {
      ifds.push(self);
    }
    // Now search in all sub IFDs
    for subs in self.sub_ifds() {
      for ifd in subs.1 {
        ifds.append(&mut ifd.find_ifds_with_tag(tag));
      }
    }
    ifds
  }

  pub fn find_first_ifd_with_tag<T: TiffTag>(&self, tag: T) -> Option<&IFD> {
    self.find_ifds_with_tag(tag).get(0).copied()
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

  pub fn has_entry<T: TiffTag>(&self, tag: T) -> bool {
    self.get_entry(tag).is_some()
  }

  pub fn sub_buf<R: Read + Seek>(&self, reader: &mut R, offset: usize, len: usize) -> Result<Vec<u8>> {
    //&buf[self.start_offset+offset..self.start_offset+offset+len]
    let mut buf = vec![0; len];
    reader.seek(SeekFrom::Start(self.base as u64 + offset as u64))?;
    reader.read_exact(&mut buf)?;
    Ok(buf)
  }

  pub fn contains_singlestrip_image(&self) -> bool {
    self.get_entry(TiffCommonTag::StripOffsets).map(Entry::count).unwrap_or(0) == 1
  }

  pub fn singlestrip_data<R: Read + Seek>(&self, reader: &mut R) -> Result<Vec<u8>> {
    assert!(self.contains_singlestrip_image());

    let offset = self
      .get_entry(TiffCommonTag::StripOffsets)
      .ok_or_else(|| TiffError::General(("tag not found").to_string()))?
      .value
      .force_usize(0);
    let len = self
      .get_entry(TiffCommonTag::StripByteCounts)
      .ok_or_else(|| TiffError::General(("tag not found").to_string()))?
      .value
      .force_usize(0);

    self.sub_buf(reader, offset, len)
  }

  pub fn singlestrip_data_rawsource<'a>(&self, rawsource: &'a RawSource) -> Result<&'a [u8]> {
    assert!(self.contains_singlestrip_image());

    let offset = self
      .get_entry(TiffCommonTag::StripOffsets)
      .ok_or_else(|| TiffError::General(("tag not found").to_string()))?
      .value
      .force_u32(0);
    let len = self
      .get_entry(TiffCommonTag::StripByteCounts)
      .ok_or_else(|| TiffError::General(("tag not found").to_string()))?
      .value
      .force_usize(0);

    Ok(rawsource.subview((self.base + offset) as u64, len as u64)?)
  }

  pub fn strip_data<R: Read + Seek>(&self, reader: &mut R) -> Result<Vec<Vec<u8>>> {
    if !self.has_entry(TiffCommonTag::StripOffsets) {
      return Err(TiffError::General("IFD contains no strip data".into()));
    }
    let offsets = if let Some(Entry { value: Value::Long(data), .. }) = self.get_entry(TiffCommonTag::StripOffsets) {
      data
    } else {
      return Err(TiffError::General("Invalid datatype for StripOffsets".to_string()));
    };
    let sizes = if let Some(Entry { value: Value::Long(data), .. }) = self.get_entry(TiffCommonTag::StripByteCounts) {
      data
    } else {
      return Err(TiffError::General("Invalid datatype for StripByteCounts".to_string()));
    };

    if offsets.len() != sizes.len() {
      return Err(TiffError::General(format!(
        "Can't get data from strips: offsets has len {} but sizes has len {}",
        offsets.len(),
        sizes.len()
      )));
    }
    let mut subviews = Vec::with_capacity(offsets.len());
    for (offset, size) in offsets.iter().zip(sizes.iter()) {
      subviews.push(self.sub_buf(reader, *offset as usize, *size as usize)?);
    }
    Ok(subviews)
  }

  pub fn strip_data_rawsource<'a>(&self, rawsource: &'a RawSource) -> Result<Vec<&'a [u8]>> {
    if !self.has_entry(TiffCommonTag::StripOffsets) {
      return Err(TiffError::General("IFD contains no strip data".into()));
    }
    let offsets = if let Some(Entry { value: Value::Long(data), .. }) = self.get_entry(TiffCommonTag::StripOffsets) {
      data
    } else {
      return Err(TiffError::General("Invalid datatype for StripOffsets".to_string()));
    };
    let sizes = if let Some(Entry { value: Value::Long(data), .. }) = self.get_entry(TiffCommonTag::StripByteCounts) {
      data
    } else {
      return Err(TiffError::General("Invalid datatype for StripByteCounts".to_string()));
    };

    if offsets.len() != sizes.len() {
      return Err(TiffError::General(format!(
        "Can't get data from strips: offsets has len {} but sizes has len {}",
        offsets.len(),
        sizes.len()
      )));
    }
    let mut subviews = Vec::with_capacity(offsets.len());

    // Check if all slices are continous - if yes, we can return one single large buffer.
    let (continous, end) =
      offsets.iter().zip(sizes.iter()).fold(
        (true, offsets[0]),
        |acc, val| {
          if acc.0 && acc.1 == *val.0 { (true, acc.1 + *val.1) } else { (false, 0) }
        },
      );

    if continous {
      subviews.push(rawsource.subview((self.base + offsets[0]) as u64, (end - offsets[0]) as u64)?);
    } else {
      for (offset, size) in offsets.iter().zip(sizes.iter()) {
        //subviews.push(self.sub_buf(reader, *offset as usize, *size as usize)?);
        subviews.push(rawsource.subview((self.base + *offset) as u64, *size as u64)?);
      }
    }
    Ok(subviews)
  }

  pub fn parse_makernote<R: Read + Seek>(&self, reader: &mut R, offset_mode: OffsetMode, sub_tags: &[u16]) -> Result<Option<IFD>> {
    if let Some(exif) = self.get_entry(ExifTag::MakerNotes) {
      let offset = exif.offset().unwrap() as u32;
      debug!("Makernote offset: {}", offset);
      match &exif.value {
        Value::Undefined(data) => {
          let mut off = 0;
          let mut endian = self.endian;

          // Olympus starts the makernote with their own name, sometimes truncated
          if data[0..5] == b"OLYMP"[..] {
            off += 8;
            if data[0..7] == b"OLYMPUS"[..] {
              off += 4;
            }
          }

          // Epson starts the makernote with its own name
          if data[0..5] == b"EPSON"[..] {
            off += 8;
          }

          // Fujifilm has 12 extra bytes
          if data[0..8] == b"FUJIFILM"[..] {
            off += 12;
          }

          // Sony has 12 extra bytes
          if data[0..9] == b"SONY DSC "[..] {
            off += 12;
          }

          // Pentax makernote starts with AOC\0 - If it's there, skip it
          if data[0..4] == b"AOC\0"[..] {
            off += 4;
          }

          // Pentax can also start with PENTAX and in that case uses different offsets
          if data[0..6] == b"PENTAX"[..] {
            off += 8;
            let endian = if data[off..off + 2] == b"II"[..] { Endian::Little } else { Endian::Big };
            // All offsets in this IFD are relative to the start of this tag,
            // so wie use the offset as correction value.
            let corr = offset as i32;
            // The IFD itself starts 10 bytes after tag offset.
            return Ok(Some(IFD::new(reader, offset + 10, self.base, corr, endian, sub_tags)?));
          }

          if data[0..7] == b"Nikon\0\x02"[..] {
            off += 10;
            let endian = if data[off..off + 2] == b"II"[..] { Endian::Little } else { Endian::Big };
            return Ok(Some(IFD::new(reader, 8, self.base + offset + 10, 0, endian, sub_tags)?));
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

          match offset_mode {
            OffsetMode::Absolute => Ok(Some(IFD::new(reader, offset + off as u32, self.base, self.corr, endian, sub_tags)?)),
            OffsetMode::RelativeToIFD => {
              // Value offsets are relative to IFD offset
              let corr = offset + off as u32;
              Ok(Some(IFD::new(reader, offset + off as u32, self.base, corr as i32, endian, sub_tags)?))
            }
          }
        }
        _ => Err(TiffError::General("EXIF makernote has unknown type".to_string())),
      }
    } else {
      Ok(None)
    }
  }

  pub fn dump<T: TiffTag>(&self, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    out.push(format!("IFD entries: {}\n", self.entries.len()));
    out.push(format!("{0:<34}  | {1:<10} | {2:<6} | {3}\n", "Tag", "Type", "Count", "Data"));
    for (tag, entry) in &self.entries {
      let mut line = String::new();
      let tag_name = {
        if let Ok(name) = T::try_from(*tag) {
          format!("{:?}", name)
        } else {
          format!("<?{}>", tag)
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
