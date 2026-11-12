// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::{
  Entry, Result, TiffError, Value, apply_corr,
  entry::RawEntry,
  offset_add, read_from_file,
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
  collections::{BTreeMap, HashMap, HashSet},
  io::{Read, Seek, SeekFrom},
};

const MAX_IFD_ENTRIES: usize = 4096;

/// Limit for the number of IFDs in a chain
const MAX_IFD_CHAIN: usize = 256;

/// Limit for the number of sub-IFDs of a tag
const MAX_SUB_IFDS: usize = 64;

/// Record the IFD at `offset` in a chain, false if the chain loops back to
/// an IFD in `visited` or reaches `MAX_IFD_CHAIN` IFDs
pub(super) fn visit_chained_ifd(visited: &mut HashSet<u32>, offset: u32) -> bool {
  if visited.len() >= MAX_IFD_CHAIN {
    log::warn!("TIFF: IFD chain reaches the limit of {} IFDs, ignoring the rest", MAX_IFD_CHAIN);
    false
  } else if !visited.insert(offset) {
    log::warn!("TIFF: IFD chain loops back to offset {}, ignoring the rest", offset);
    false
  } else {
    true
  }
}

#[derive(Debug)]
pub enum OffsetMode {
  Absolute,
  RelativeToIFD,
}

#[derive(Debug)]
pub enum DataMode {
  Strips,
  Tiles,
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
    reader.seek(SeekFrom::Start(base as u64 + offset as u64))?;
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

    next_ifd = apply_corr(next_ifd, corr)?;
    let mut visited = HashSet::from([next_ifd]);
    let mut root = IFD::new(reader, next_ifd, base, corr, endian, &multi_sub_tags)?;
    if root.entries.is_empty() {
      return Err(TiffError::General("TIFF is invalid, IFD must contain at least one entry".to_string()));
    }
    next_ifd = root.next_ifd;

    // max_chain limits the IFDs chained to the root, 0 for no limit
    while next_ifd != 0 && (max_chain == 0 || root.chain.len() < max_chain) && visit_chained_ifd(&mut visited, next_ifd) {
      let ifd = IFD::new(reader, next_ifd, base, corr, endian, &multi_sub_tags)?;
      if ifd.entries.is_empty() {
        return Err(TiffError::General("TIFF is invalid, IFD must contain at least one entry".to_string()));
      }
      next_ifd = ifd.next_ifd;
      root.chain.push(ifd);
    }

    Ok(root)
  }

  pub fn new<R: Read + Seek>(reader: &mut R, offset: u32, base: u32, corr: i32, endian: Endian, sub_tags: &[u16]) -> Result<IFD> {
    reader.seek(SeekFrom::Start(base as u64 + offset as u64))?;
    // Offsets of sub-IFDs, already corrected
    let mut sub_ifd_offsets = HashMap::new();
    let mut reader = EndianReader::new(reader, endian);
    let entry_count = reader.read_u16()?;

    if entry_count as usize > MAX_IFD_ENTRIES {
      log::warn!(
        "TIFF: IFD entry count {} is suspicious (limit {}). The file might be corrupt.",
        entry_count,
        MAX_IFD_ENTRIES
      );
    }

    let mut entries = BTreeMap::new();
    let mut sub = HashMap::new();
    let table_start = reader.position()?;
    let mut next_pos = table_start;
    debug!("Parse entries");
    let mut consecutive_errors = 0;
    // False if the parsing stopped before the end of the entry table
    let mut complete = true;

    for i in 0..entry_count {
      if i as usize >= MAX_IFD_ENTRIES {
        log::warn!(
          "TIFF: Reached maximum IFD entry limit ({}). Stopping parse to prevent infinite loops.",
          MAX_IFD_ENTRIES
        );
        complete = false;
        break;
      }

      if let Err(e) = reader.goto(next_pos) {
        log::warn!("Truncated IFD: Could not seek to next entry position. Stopping parse. Error: {}", e);
        complete = false;
        break;
      }

      next_pos = next_pos.saturating_add(12);

      let tag = match reader.read_u16() {
        Ok(t) => t,
        Err(e) => {
          log::warn!("Truncated IFD: Could not read tag ID (Index {}). Stopping parse. Error: {}", i, e);
          complete = false;
          break;
        }
      };

      match Entry::parse(&mut reader, base, corr, tag) {
        Ok(entry) => {
          consecutive_errors = 0;

          if sub_tags.contains(&tag) {
            match &entry.value {
              Value::Long(offsets) => match offsets.iter().map(|offset| apply_corr(*offset, corr)).collect::<Result<Vec<_>>>() {
                Ok(offsets) => {
                  sub_ifd_offsets.insert(tag, offsets);
                }
                Err(err) => log::warn!("Invalid SubIFD offset for tag 0x{:X}, skipping: {}", tag, err),
              },
              Value::Undefined(_) | Value::Unknown(..) => {
                if let Some(offset) = entry.offset() {
                  sub_ifd_offsets.insert(tag, vec![offset as u32]);
                } else {
                  log::warn!("SubIFD entry for tag 0x{:X} has no offset, skipping", tag);
                }
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
          consecutive_errors += 1;
          log::warn!("Failed to parse TIFF tag 0x{:X} (Index {}). Error: {:?}", tag, i, err);

          // If we fail 5 times in a row, the IFD is likely garbage or physically truncated.
          if consecutive_errors >= 5 {
            log::warn!("Too many consecutive parsing errors ({}). Stopping parse to prevent flood.", consecutive_errors);
            complete = false;
            break;
          }
        }
      }
    }

    // The next IFD pointer follows the entry table, a failed entry leaves the
    // reader elsewhere. If the parsing stopped early, the IFD is damaged and
    // ends the IFD chain.
    // Some TIFF writers skip the next ifd pointer: on an I/O error, we
    // fallback to 0, signaling the end of IFD chains.
    let table_end = u32::try_from(table_start as u64 + entry_count as u64 * 12).map_err(|_| TiffError::Overflow("IFD entry table exceeds 4 GiB".into()));
    let next_ifd = match table_end
      .and_then(|end| reader.goto(end))
      .and_then(|_| reader.read_u32().map_err(TiffError::from))
    {
      Ok(ptr) if complete => ptr,
      Ok(_) => 0,
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
    for (tag, offsets) in sub_ifd_offsets {
      let mut seen = HashSet::new();
      let offsets: Vec<u32> = offsets.into_iter().filter(|offset| seen.insert(*offset)).collect();
      if offsets.len() > MAX_SUB_IFDS {
        log::warn!("TIFF: tag 0x{:X} has {} sub-IFDs, reading the first {} only", tag, offsets.len(), MAX_SUB_IFDS);
      }
      let mut ifds = Vec::new();
      for offset in offsets.into_iter().take(MAX_SUB_IFDS) {
        match Self::new(reader, offset, base, corr, endian, &[]) {
          Ok(ifd) => ifds.push(ifd),
          Err(err) => {
            log::warn!("Error while processing TIFF sub-IFD for tag 0x{:X}, ignoring it: {}", tag, err);
          }
        };
      }
      sub.insert(tag, ifds);
    }
    EndianReader::new(reader, endian).goto(pos)?; // restore
    let next_ifd = match next_ifd {
      0 => 0,
      next_ifd => apply_corr(next_ifd, corr).unwrap_or_else(|err| {
        log::warn!("Invalid next IFD offset, ignoring the IFD chain: {}", err);
        0
      }),
    };
    Ok(IFD {
      offset,
      base,
      corr,
      next_ifd,
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
        val => Err(TiffError::FormatMismatch(format!(
          "Tag 0x{:X} has the type {} and is no IFD offset",
          tag,
          val.value_type_name()
        ))),
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
        data: read_from_file(
          file,
          self.base as u64
            + entry
              .offset()
              .ok_or_else(|| TiffError::General(format!("Entry for tag {:?} has no offset", tag)))? as u64,
          entry.byte_size(),
        )?,
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
        data: read_from_file(
          file,
          self.base as u64
            + entry
              .offset()
              .ok_or_else(|| TiffError::General(format!("Entry for tag {:?} has no offset", tag)))? as u64,
          len,
        )?,
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

  pub fn get_new_sub_file_type(&self) -> Option<u16> {
    if let Some(entry) = self.get_entry(TiffCommonTag::NewSubFileType) {
      Some(entry.force_u16(0))
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

    Ok(rawsource.subview(self.base as u64 + offset as u64, len as u64)?)
  }

  /// Return byte slices to strip data.
  /// If there exists a single strip only or if all strips are continous,
  /// the second return value contains the whole strip data in a single slice.
  pub fn strip_data<'a>(&self, rawsource: &'a RawSource) -> Result<(Vec<&'a [u8]>, Option<&'a [u8]>)> {
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

    // Check if all slices are continous
    let (is_continous, end_off) =
      offsets.iter().zip(sizes.iter()).fold(
        (true, offsets[0]),
        |acc, val| {
          if acc.0 && acc.1 == *val.0 { (true, acc.1 + *val.1) } else { (false, 0) }
        },
      );

    let mut subviews = Vec::with_capacity(offsets.len());
    for (offset, size) in offsets.iter().zip(sizes.iter()) {
      subviews.push(rawsource.subview(self.base as u64 + *offset as u64, *size as u64)?);
    }

    let continous = if is_continous {
      Some(rawsource.subview(self.base as u64 + offsets[0] as u64, (end_off - offsets[0]) as u64)?)
    } else {
      None
    };

    Ok((subviews, continous))
  }

  pub fn tile_data<'a>(&self, rawsource: &'a RawSource) -> Result<Vec<&'a [u8]>> {
    let offsets = if let Some(Entry { value: Value::Long(data), .. }) = self.get_entry(TiffCommonTag::TileOffsets) {
      data
    } else {
      return Err(TiffError::General("Invalid datatype for TileOffsets".to_string()));
    };

    let byte_counts = if let Some(Entry { value: Value::Long(data), .. }) = self.get_entry(TiffCommonTag::TileByteCounts) {
      data
    } else {
      return Err(TiffError::General("Invalid datatype for TileByteCounts".to_string()));
    };

    let mut tile_slices = Vec::with_capacity(offsets.len());
    offsets.iter().zip(byte_counts.iter()).for_each(|(offset, size)| {
      tile_slices.push(rawsource.subview(*offset as u64, *size as u64).map_err(TiffError::Io));
    });
    Ok(tile_slices.into_iter().collect::<Result<Vec<_>>>()?)
  }

  /// Check for the data mode (Strips or Tiles)
  pub fn data_mode(&self) -> Result<DataMode> {
    if self.has_entry(TiffCommonTag::StripOffsets) {
      Ok(DataMode::Strips)
    } else if self.has_entry(TiffCommonTag::TileOffsets) {
      Ok(DataMode::Tiles)
    } else {
      Err(TiffError::General("IFD has no StripOffsets or TileOffsets tag".into()))
    }
  }

  pub fn parse_makernote<R: Read + Seek>(&self, reader: &mut R, offset_mode: OffsetMode, sub_tags: &[u16]) -> Result<Option<IFD>> {
    if let Some(exif) = self.get_entry(ExifTag::MakerNotes) {
      let offset = exif.offset().ok_or(TiffError::General("MakerNote entry has no offset".into()))? as u32;
      debug!("Makernote offset: {}", offset);
      match &exif.value {
        Value::Undefined(data) => {
          let mut off = 0;
          let mut endian = self.endian;
          // The two bytes at `off`, None if the makernote is shorter
          let data_at = |off: usize| data.get(off..off + 2);

          // Olympus starts the makernote with their own name, sometimes truncated
          if data.starts_with(b"OLYMP") {
            off += 8;
            if data.starts_with(b"OLYMPUS") {
              off += 4;
            }
          }

          // Epson starts the makernote with its own name
          if data.starts_with(b"EPSON") {
            off += 8;
          }

          // Fujifilm has 12 extra bytes
          if data.starts_with(b"FUJIFILM") {
            off += 12;
          }

          // Sony has 12 extra bytes
          if data.starts_with(b"SONY DSC ") {
            off += 12;
          }

          // Pentax makernote starts with AOC\0 - If it's there, skip it
          if data.starts_with(b"AOC\0") {
            off += 4;
          }

          // Pentax can also start with PENTAX and in that case uses different offsets
          if data.starts_with(b"PENTAX") {
            off += 8;
            let endian = if data_at(off) == Some(b"II") { Endian::Little } else { Endian::Big };
            // All offsets in this IFD are relative to the start of this tag,
            // so wie use the offset as correction value.
            let corr = i32::try_from(offset).map_err(|_| TiffError::Overflow(format!("Makernote offset {} is out of range", offset)))?;
            // The IFD itself starts 10 bytes after tag offset.
            return Ok(Some(IFD::new(reader, offset_add(offset, 10)?, self.base, corr, endian, sub_tags)?));
          }

          if data.starts_with(b"Nikon\0\x02") {
            off += 10;
            let endian = if data_at(off) == Some(b"II") { Endian::Little } else { Endian::Big };
            return Ok(Some(IFD::new(reader, 8, offset_add(offset_add(self.base, offset)?, 10)?, 0, endian, sub_tags)?));
          }

          // Some have MM or II to indicate endianness - read that
          if data_at(off) == Some(b"II") {
            off += 2;
            endian = Endian::Little;
          }
          if data_at(off) == Some(b"MM") {
            off += 2;
            endian = Endian::Big;
          }

          let ifd_offset = offset_add(offset, off as u32)?;
          match offset_mode {
            OffsetMode::Absolute => Ok(Some(IFD::new(reader, ifd_offset, self.base, self.corr, endian, sub_tags)?)),
            OffsetMode::RelativeToIFD => {
              // Value offsets are relative to IFD offset
              let corr = i32::try_from(ifd_offset).map_err(|_| TiffError::Overflow(format!("Makernote offset {} is out of range", ifd_offset)))?;
              Ok(Some(IFD::new(reader, ifd_offset, self.base, corr, endian, sub_tags)?))
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

#[cfg(test)]
mod tests {
  use super::*;
  use std::io::Cursor;

  /// IFD at offset 8 with the entry `entry` and no next IFD
  fn ifd_with_entry(entry: [u8; 12]) -> Vec<u8> {
    let mut data = vec![0x49, 0x49, 42, 0, 8, 0, 0, 0, 1, 0];
    data.extend_from_slice(&entry);
    data.extend_from_slice(&[0, 0, 0, 0]);
    data
  }

  #[test]
  fn sub_ifd_of_unknown_type_uses_tag_and_offset() -> Result<()> {
    // tag 0x14A, type 0 (unknown), count 0
    let data = ifd_with_entry([0x4a, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let ifd = IFD::new(&mut Cursor::new(data), 8, 0, 0, Endian::Little, &[0x14a])?;
    assert!(ifd.sub.contains_key(&0x14a));
    assert!(!ifd.sub.contains_key(&0));
    Ok(())
  }

  #[test]
  fn sub_ifd_of_undefined_type_is_corrected_once() -> Result<()> {
    // tag 0x927C, type UNDEFINED, count 18, offset 24 + correction 2
    let mut data = ifd_with_entry([0x7c, 0x92, 7, 0, 18, 0, 0, 0, 24, 0, 0, 0]);
    // Sub-IFD at 26 with tag 0x200
    data.extend_from_slice(&[1, 0, 0x00, 0x02, 3, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
    let ifd = IFD::new(&mut Cursor::new(data), 8, 0, 2, Endian::Little, &[0x927c])?;
    assert!(ifd.sub[&0x927c][0].has_entry(0x200_u16));
    Ok(())
  }

  /// IFD at 8 with a SubIFDs entry of LONG `offsets`, then 256 zero bytes
  fn ifd_with_sub_ifds(offsets: &[u32]) -> Vec<u8> {
    let count = (offsets.len() as u32).to_le_bytes();
    let mut entry = [0x4a, 0x01, 4, 0, count[0], count[1], count[2], count[3], 26, 0, 0, 0];
    if offsets.len() == 1 {
      entry[8..].copy_from_slice(&offsets[0].to_le_bytes());
    }
    let mut data = ifd_with_entry(entry);
    if offsets.len() > 1 {
      data.extend(offsets.iter().flat_map(|offset| offset.to_le_bytes()));
    }
    data.resize(data.len() + 256, 0);
    data
  }

  #[test]
  fn damaged_ifd_ends_the_chain() -> Result<()> {
    // 6 entries of type LONG, count 2, with data beyond the end of the stream
    let mut data = vec![0x49, 0x49, 42, 0, 8, 0, 0, 0, 6, 0];
    for tag in 0x100..0x106_u16 {
      data.extend_from_slice(&tag.to_le_bytes());
      data.extend_from_slice(&[4, 0, 2, 0, 0, 0, 0, 0x10, 0, 0]);
    }
    data.extend_from_slice(&[0, 0, 0, 0]);
    let ifd = IFD::new(&mut Cursor::new(data), 8, 0, 0, Endian::Little, &[])?;
    assert!(ifd.entries.is_empty());
    assert_eq!(ifd.next_ifd, 0);
    Ok(())
  }

  #[test]
  fn next_ifd_follows_the_entry_table() -> Result<()> {
    use crate::formats::tiff::tests::ifd_chain;
    let data = ifd_chain(8, &[26, 0]);
    let ifd = IFD::new(&mut Cursor::new(data), 8, 0, 0, Endian::Little, &[])?;
    assert_eq!(ifd.next_ifd, 26);
    Ok(())
  }

  #[test]
  fn short_makernotes_do_not_panic() {
    for prefix in [
      &b""[..],
      b"OLY",
      b"OLYMP",
      b"OLYMPUS",
      b"EPSON",
      b"FUJIFILM",
      b"SONY DSC ",
      b"AOC\0",
      b"PENTAX",
      b"Nikon\0\x02",
      b"I",
    ] {
      // Root IFD with a MakerNotes entry of the prefix, the data after the IFD
      let count = (prefix.len() as u32).to_le_bytes();
      let mut data = ifd_with_entry([0x7c, 0x92, 7, 0, count[0], count[1], count[2], count[3], 26, 0, 0, 0]);
      if prefix.len() <= 4 {
        data[18..18 + prefix.len()].copy_from_slice(prefix);
      } else {
        data.extend_from_slice(prefix);
      }
      let ifd = IFD::new(&mut Cursor::new(data.clone()), 8, 0, 0, Endian::Little, &[]).expect("root IFD");
      for mode in [OffsetMode::Absolute, OffsetMode::RelativeToIFD] {
        let _ = ifd.parse_makernote(&mut Cursor::new(data.clone()), mode, &[]);
      }
    }
  }

  #[test]
  fn extend_sub_ifds_needs_offsets() -> Result<()> {
    use crate::formats::tiff::tests::ifd_chain;
    let data = ifd_chain(8, &[0]);
    let mut ifd = IFD::new(&mut Cursor::new(data.clone()), 8, 0, 0, Endian::Little, &[])?;
    // Tag 0x100 is a SHORT
    assert!(matches!(ifd.extend_sub_ifds(&mut Cursor::new(data), 0x100), Err(TiffError::FormatMismatch(_))));
    Ok(())
  }

  #[test]
  fn sub_ifds_are_unique_and_limited() -> Result<()> {
    let start = 26 + 4 * 100;
    // The same sub-IFD 100 times
    let data = ifd_with_sub_ifds(&[start; 100]);
    let ifd = IFD::new(&mut Cursor::new(data), 8, 0, 0, Endian::Little, &[0x14a])?;
    assert_eq!(ifd.sub[&0x14a].len(), 1);
    // 100 different empty sub-IFDs
    let offsets: Vec<u32> = (0..100).map(|i| start + 2 * i).collect();
    let data = ifd_with_sub_ifds(&offsets);
    let ifd = IFD::new(&mut Cursor::new(data), 8, 0, 0, Endian::Little, &[0x14a])?;
    assert_eq!(ifd.sub[&0x14a].len(), MAX_SUB_IFDS);
    Ok(())
  }
}
