// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

// This module contains only basic TIFF parsing. No decoder specific variants
// should be implemented here.
// You can pass a list of known tags for sub-IFDs, then these tags will be parsed
// and added as sub-IFDs. This should be used only for well-defined IFDs.
// Makernotes and such stuff must be parsed in decoder implementation.

use std::{cmp::min, collections::{BTreeMap}, fmt::{Display}};
use std::str;
use std::usize;

use byteorder::{ByteOrder, LittleEndian};

use crate::{bits::Endian, tags::{LegacyTiffRootTag, TiffTagEnum}};

use log::debug;

                          // 0-1-2-3-4-5-6-7-8-9-10-11-12-13
const DATASHIFTS: [u8;14] = [0,0,0,1,2,3,0,0,1,2, 3, 2, 3, 2];

/// General type for a u16 TIFF tag
pub type LegacyTiffTag = u16;


impl Into<LegacyTiffTag> for LegacyTiffRootTag {
  fn into(self) -> LegacyTiffTag {
      self as LegacyTiffTag
  }
}

#[derive(Debug, Copy, Clone)]
pub struct LegacyTiffEntry<'a> {
  tag: u16,
  typ: u16,
  count: u32,
  parent_offset: usize,
  data_offset: usize,
  data: &'a [u8],
  endian: Endian,
}

#[derive(Debug, Clone)]
pub struct LegacyTiffIFD<'a> {
  pub chain_level: isize,
  pub entries: BTreeMap<u16,LegacyTiffEntry<'a>>,
  pub chained_ifds: Vec<LegacyTiffIFD<'a>>,
  pub sub_ifds: BTreeMap<u16, Vec<LegacyTiffIFD<'a>>>,
  nextifd: usize,
  pub start_offset: usize,
  endian: Endian,
  file_buf: &'a [u8],
}

impl<'a> Display for LegacyTiffIFD<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("IFD chained level({})\n", self.chain_level))?;
        f.write_fmt(format_args!("IFD entries: {}\n", self.entries.len()))?;
        f.write_fmt(format_args!("{0:<34}  | {1:<10} | {2:<6} | {3}\n", "Tag", "Type", "Count", "Data"))?;
        for (tag, entry) in &self.entries {
            f.write_fmt(format_args!("{0:#06x} : {0:<6} {1:<20}| {2:<10} | {3:<6} | ", tag, tag_name(tag), entry.typ_name(), entry.count))?;
            entry.fmt_data(f)?;
            f.write_str("\n")?;
        }
        f.write_fmt(format_args!("Sub IFDs:\n"))?;
        for (tag, ifds) in &self.sub_ifds {
            f.write_fmt(format_args!("Sub IFD {}\n", tag))?;
            for ifd in ifds {
                f.write_fmt(format_args!("{}\n", ifd))?;
            }
        }
        f.write_fmt(format_args!("Chained IFDs:\n"))?;
        for ifd in &self.chained_ifds {
            f.write_fmt(format_args!("{}\n", ifd))?;
        }
        Ok(())
    }
}


impl<'a> LegacyTiffIFD<'a> {
  pub fn new_file(buf: &'a[u8], known_subifds: &Vec<u16>) -> Result<LegacyTiffIFD<'a>, String> {
      LegacyTiffIFD::new_root(buf, 0, known_subifds)
  }


  pub fn new_root(buf: &'a[u8], offset: usize, known_subifds: &Vec<u16>) -> Result<LegacyTiffIFD<'a>, String> {
    let mut chained_ifds = Vec::new();

    let endian = match LittleEndian::read_u16(&buf[offset..]) {
      0x4949 => Endian::Little,
      0x4d4d => Endian::Big,
      x => {return Err(format!("TIFF: don't know marker 0x{:x}", x).to_string())},
    };
    let mut nextifd = endian.read_u32(buf, offset+4) as usize;
    for chain_level in 0..100 { // Never read more than 100 chained IFDs
      if nextifd >= buf.len() {
        debug!("IFD is invalid, giving up");
        break;
      }
      //let ifd = TiffIFD::new(&buf[offset..], nextifd, 0, offset, chain_level, endian, known_subifds)?;
      let ifd = LegacyTiffIFD::new(buf, nextifd, 0, offset, chain_level, endian, known_subifds)?;
      nextifd = ifd.nextifd;
      chained_ifds.push(ifd);
      debug!("next ifd: {}", nextifd);
      if nextifd == 0 {
        break
      }
    }

    // This creates a virtual root IFD container that contains all other
    // real IFDs
    Ok(LegacyTiffIFD {
        chain_level: -1, // container IFD
      entries: BTreeMap::new(),
      chained_ifds: chained_ifds,
      sub_ifds: BTreeMap::new(),
      nextifd: 0,
      start_offset: offset,
      endian: endian,
      file_buf: buf,
    })
  }

  pub fn new(buf: &'a[u8], offset: usize, base_offset: usize, start_offset: usize, chain_level: isize, e: Endian, known_subifds: &Vec<u16>) -> Result<LegacyTiffIFD<'a>, String> {
    let mut entries = BTreeMap::new();
    let mut sub_ifds: BTreeMap<u16, Vec<LegacyTiffIFD<'a>>> = BTreeMap::new();

    let num = e.read_u16(buf, offset); // Directory entries in this IFD
    if num > 4000 { // TODO: add constant
      return Err(format!("too many entries in IFD ({})", num).to_string())
    }
    for i in 0..num {
      let entry_offset: usize = offset + 2 + (i as usize)*12;
      //if Tag::n(e.read_u16(buf, entry_offset)).is_none() {
      //  // Skip entries we don't know about to speedup decoding
      //  continue;
      //}
      let entry = LegacyTiffEntry::new(buf, entry_offset, base_offset, offset, e);

      if known_subifds.contains(&entry.tag) {
        if chain_level < 32 { // Avoid infinite looping IFDs
          let mut ifds = Vec::with_capacity(entry.count as usize);
          for i in 0..entry.count {
            let ifd = LegacyTiffIFD::new(buf, entry.get_u32(i as usize) as usize, base_offset, start_offset, chain_level+1, e, known_subifds);
            match ifd {
              Ok(val) => {ifds.push(val);},
              Err(_) => {
                debug!("Unable to parse IFD"); //TODO: better hint
              }
            }
          }
          if let Some(subs) = sub_ifds.get_mut(&entry.tag) {
            subs.extend(ifds);
          } else {
            sub_ifds.insert(entry.tag, ifds);
          }
        }
      }
      /*
      if entry.tag == Tag::SubIFDs.into()
      || entry.tag == Tag::ExifIFDPointer.into()
      || entry.tag == t(Tag::RafRawSubIFD)
      || entry.tag == t(Tag::KodakIFD)
      || entry.tag == t(Tag::KdcIFD) {
        if depth < 10 { // Avoid infinite looping IFDs
          for i in 0..entry.count {
            let ifd = TiffIFD::new(buf, entry.get_u32(i as usize) as usize, base_offset, start_offset, depth+1, e);
            match ifd {
              Ok(val) => {subifds.push(val);},
              Err(_) => {entries.insert(entry.tag, entry);}, // Ignore unparsable IFDs
            }
          }
        }
      } else if entry.tag == t(Tag::Makernote) {
        if depth < 10 { // Avoid infinite looping IFDs
          let ifd = TiffIFD::new_makernote(buf, entry.doffset(), base_offset, depth+1, e);
          match ifd {
            Ok(val) => {subifds.push(val);},
            Err(_) => {entries.insert(entry.tag, entry);}, // Ignore unparsable IFDs
          }
        }
      } else {
        */
        entries.insert(entry.tag, entry);
      //}
    }

    Ok(LegacyTiffIFD {
      chain_level,
      entries,
      chained_ifds: Vec::new(),
      nextifd: e.read_u32(buf, offset + (2+num*12) as usize) as usize,
      start_offset,
      endian: e,
      sub_ifds,
      file_buf: buf,
    })
  }

  pub fn sub_buf(&self, offset: usize, len: usize) -> &'a[u8] {
    &self.file_buf[self.start_offset+offset..self.start_offset+offset+len]
  }

  pub fn contains_singlestrip_image(&self) -> bool {
    true // FIXME
  }

  pub fn singlestrip_data(&self) -> Result<&[u8], String> {
    assert!(self.contains_singlestrip_image());

    let offset = self.find_entry(LegacyTiffRootTag::StripOffsets).unwrap().get_u32(0) as usize;
    let len = self.find_entry(LegacyTiffRootTag::StripByteCounts).unwrap().get_u32(0) as usize;

    let src = self.sub_buf(offset, len);

    Ok(src)
  }

  pub fn find_entry<T: Into<LegacyTiffTag> + Copy>(&self, tag: T) -> Option<&LegacyTiffEntry> {
    if self.entries.contains_key(&tag.into()) {
      self.entries.get(&tag.into())
    } else {
      for subs in self.sub_ifds.values() {
        for ifd in subs {
          match ifd.find_entry(tag) {
            Some(x) => return Some(x),
            None => {},
          }
        }
      }
      for ifd in &self.chained_ifds {
        match ifd.find_entry(tag) {
          Some(x) => return Some(x),
          None => {},
        }
      }
      None
    }
  }

  pub fn has_entry<T: Into<LegacyTiffTag> + Copy>(&self, tag: T) -> bool {
    self.find_entry(tag).is_some()
  }

  pub fn find_ifds_with_tag<T: Into<LegacyTiffTag> + Copy>(&self, tag: T) -> Vec<&LegacyTiffIFD> {
    let mut ifds = Vec::new();
    if self.entries.contains_key(&tag.into()) {
      ifds.push(self);
    }
    for subs in self.sub_ifds.values() {
      for ifd in subs {
        if ifd.entries.contains_key(&tag.into()) {
          ifds.push(ifd);
        }
        ifds.extend(ifd.find_ifds_with_tag(tag));
      }
    }
    for ifd in &self.chained_ifds {
      if ifd.entries.contains_key(&tag.into()) {
        ifds.push(ifd);
      }
      ifds.extend(ifd.find_ifds_with_tag(tag));
    }
    ifds
  }

  pub fn find_first_ifd<T: Into<LegacyTiffTag> + Copy>(&self, tag: T) -> Option<&LegacyTiffIFD> {
    let ifds = self.find_ifds_with_tag(tag);
    if ifds.len() == 0 {
      None
    } else {
      Some(ifds[0])
    }
  }

  pub fn get_endian(&self) -> Endian { self.endian }
  pub fn little_endian(&self) -> bool { self.endian.little() }
  pub fn start_offset(&self) -> usize { self.start_offset }
}

impl<'a> LegacyTiffEntry<'a> {
  pub fn new(buf: &'a[u8], offset: usize, base_offset: usize, parent_offset: usize, e: Endian) -> LegacyTiffEntry<'a> {
    let tag = e.read_u16(buf, offset);
    let mut typ = e.read_u16(buf, offset+2);
    let count = e.read_u32(buf, offset+4);

    // If we don't know the type assume byte data
    if typ == 0 || typ > 13 {
      typ = 1;
    }

    let bytesize: usize = (count as usize) << DATASHIFTS[typ as usize];
    let data_offset: usize = if bytesize <= 4 {
      offset + 8
    } else {
      (e.read_u32(buf, offset+8) as usize) - base_offset
    };

    LegacyTiffEntry {
      tag: tag,
      typ: typ,
      count: count,
      parent_offset: parent_offset,
      data_offset: data_offset,
      data: &buf[data_offset .. data_offset+bytesize],
      endian: e,
    }
  }

  pub fn copy_with_new_data(&self, data: &'a[u8]) -> LegacyTiffEntry<'a> {
    let mut copy = self.clone();
    copy.data = data;
    copy
  }

  pub fn copy_offset_from_parent(&self, buffer: &'a[u8]) -> LegacyTiffEntry<'a> {
    self.copy_with_new_data(&buffer[self.parent_offset+self.data_offset..])
  }

  pub fn data_offset(&self) -> usize { self.data_offset }
  pub fn parent_offset(&self) -> usize { self.parent_offset }
  pub fn count(&self) -> u32 { self.count }
  //pub fn typ(&self) -> u16 { self.typ }

  pub fn get_rational(&self, idx: usize) -> Rational {
    match self.typ {
      5                  => {
        let n = self.endian.read_u32(self.data, idx*8);
        let d = self.endian.read_u32(self.data, idx*8+4);
        Rational { n, d }
      },
      _ => panic!("{}", format!("Trying to read typ {} for a rational", self.typ).to_string()),
    }
  }

  pub fn get_srational(&self, idx: usize) -> SRational {
    match self.typ {
      10 => {
        let n = self.endian.read_i32(self.data, idx*8);
        let d = self.endian.read_i32(self.data, idx*8+4);
        SRational { n, d }
      }
      _ => panic!("{}", format!("Trying to read typ {} for a srational", self.typ).to_string()),
    }
  }

  pub fn get_u8(&self, idx: usize) -> u8 {
    match self.typ {
      1                  => self.data[idx] as u8,
      _ => panic!("{}", format!("Trying to read typ {} for a u8", self.typ).to_string()),
    }
  }

  pub fn get_i8(&self, idx: usize) -> i8 {
    match self.typ {
      6                  => self.data[idx] as i8,
      _ => panic!("{}", format!("Trying to read typ {} for a i8", self.typ).to_string()),
    }
  }

  pub fn get_u16(&self, idx: usize) -> u16 {
    match self.typ {
      1                  => self.data[idx] as u16,
      3 | 8              => self.get_force_u16(idx),
      _ => panic!("{}", format!("Trying to read typ {} for a u32", self.typ).to_string()),
    }
  }

  pub fn get_i16(&self, idx: usize) -> i16 {
    match self.typ {
      8                  => self.data[idx] as i16,
      _ => panic!("{}", format!("Trying to read typ {} for a i16", self.typ).to_string()),
    }
  }

  pub fn get_u32(&self, idx: usize) -> u32 {
    match self.typ {
      1 | 3 | 8          => self.get_u16(idx) as u32,
      4 | 7 | 9 | 13     => self.get_force_u32(idx),
      _ => panic!("{}", format!("Trying to read typ {} for a u32", self.typ).to_string()),
    }
  }

  pub fn get_i32(&self, idx: usize) -> i32 {
    match self.typ {
      9                  => self.data[idx] as i32,
      _ => panic!("{}", format!("Trying to read typ {} for a i32", self.typ).to_string()),
    }
  }

  pub fn get_usize(&self, idx: usize) -> usize { self.get_u32(idx) as usize }

  pub fn get_force_u32(&self, idx: usize) -> u32 {
    self.endian.read_u32(self.data, idx*4)
  }

  pub fn get_force_u16(&self, idx: usize) -> u16 {
    self.endian.read_u16(self.data, idx*2)
  }

  pub fn get_f32(&self, idx: usize) -> f32 {
    if self.typ == 5 { // Rational
      // We must multiply with 8 because a Rational type
      // is composed of 2x u32 values
      let a = self.endian.read_u32(self.data, idx*8) as f32;
      let b = self.endian.read_u32(self.data, idx*8+4) as f32;
      a / b
    } else if self.typ == 10 { // Signed Rational
      let a = self.endian.read_i32(self.data, idx*8) as f32;
      let b = self.endian.read_i32(self.data, idx*8+4) as f32;
      a / b
    } else {
      self.get_u32(idx) as f32
    }
  }

  pub fn get_str(&self) -> &str {
    // Truncate the string when there are \0 bytes
    let len = match self.data.iter().position(|&x| x == 0) {
      Some(p) => p,
      None => self.data.len(),
    };
    match str::from_utf8(&self.data[0..len]) {
      Ok(val) => val.trim(),
      Err(err) => panic!("{}", format!("from_utf8() failed: {}", err)),
    }
  }

  pub fn get_data(&self) -> &[u8] {
    self.data
  }

  pub fn data_plaintext(&self) -> String {
    let mut out = String::new();
    let maxc = min(self.count, 8) as usize;
    match self.typ() {
        TagType::BYTE => {
            for idx in 0..maxc {
                out.push_str(&format!("{} ", self.get_u8(idx)));
            }
        }
        TagType::ASCII => {
            out.push_str(self.get_str());
        }
        TagType::SHORT => {
            for idx in 0..maxc {
              out.push_str(&format!("{} ", self.get_u16(idx)));
            }
        }
        TagType::LONG => {
            for idx in 0..maxc {
              out.push_str(&format!("{} ", self.get_u32(idx)));
            }
        }
        TagType::RATIONAL => {}
        TagType::SBYTE => {
            for idx in 0..maxc {
              out.push_str(&format!("{} ", self.get_i8(idx)));
            }
        }
        TagType::UNDEFINED => {}
        TagType::SSHORT => {
            for idx in 0..maxc {
              out.push_str(&format!("{} ", self.get_i16(idx)));
            }
        }
        TagType::SLONG => {
            for idx in 0..maxc {
              out.push_str(&format!("{} ", self.get_i32(idx)));
            }
        }
        TagType::SRATIONAL => {}
        TagType::FLOAT => {}
        TagType::DOUBLE => {}
        TagType::IFD => {
            for idx in 0..maxc {
              out.push_str(&format!("<{}> ", self.get_u32(idx)));
            }
        }
        TagType::LONG8 => {}
        TagType::SLONG8 => {}
        TagType::IFD8 => {

        }
    }
    out
  }

  pub fn fmt_data(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let maxc = min(self.count, 8) as usize;
    match self.typ() {
        TagType::BYTE => {
            for idx in 0..maxc {
                f.write_fmt(format_args!("{} ", self.get_u8(idx)))?;
            }
        }
        TagType::ASCII => {
            f.write_str(self.get_str())?;
        }
        TagType::SHORT => {
            for idx in 0..maxc {
                f.write_fmt(format_args!("{} ", self.get_u16(idx)))?;
            }
        }
        TagType::LONG => {
            for idx in 0..maxc {
                f.write_fmt(format_args!("{} ", self.get_u32(idx)))?;
            }
        }
        TagType::RATIONAL => {}
        TagType::SBYTE => {
            for idx in 0..maxc {
                f.write_fmt(format_args!("{} ", self.get_i8(idx)))?;
            }
        }
        TagType::UNDEFINED => {}
        TagType::SSHORT => {
            for idx in 0..maxc {
                f.write_fmt(format_args!("{} ", self.get_i16(idx)))?;
            }
        }
        TagType::SLONG => {
            for idx in 0..maxc {
                f.write_fmt(format_args!("{} ", self.get_i32(idx)))?;
            }
        }
        TagType::SRATIONAL => {}
        TagType::FLOAT => {}
        TagType::DOUBLE => {}
        TagType::IFD => {
            for idx in 0..maxc {
                f.write_fmt(format_args!("{:#x} ", self.get_u32(idx)))?;
            }
        }
        TagType::LONG8 => {}
        TagType::SLONG8 => {}
        TagType::IFD8 => {

        }
    }
    Ok(())
  }

  pub fn typ(&self) -> TagType {
      TagType::n(self.typ).expect("Unknown tiff tag type")
  }

  pub fn typ_name(&self) -> &'static str {
    match self.typ() {
        TagType::BYTE => { "BYTE"}
        TagType::ASCII => { "ASCII"}
        TagType::SHORT => { "SHORT"}
        TagType::LONG => { "LONG"}
        TagType::RATIONAL => { "RATIONAL"}
        TagType::SBYTE => { "SBYTE"}
        TagType::UNDEFINED => { "UNDEF"}
        TagType::SSHORT => { "SSHORT"}
        TagType::SLONG => { "SLONG"}
        TagType::SRATIONAL => { "SRATIONAL"}
        TagType::FLOAT => { "FLOAT"}
        TagType::DOUBLE => { "DOUBLE"}
        TagType::IFD => {"IFD"}
        TagType::LONG8 => { "LONG8"}
        TagType::SLONG8 => { "SLONG8"}
        TagType::IFD8 => { "IFD8"}
    }
  }
}


#[allow(clippy::upper_case_acronyms)]
#[derive(Debug, Copy, Clone, PartialEq, enumn::N)]
#[repr(u16)]
pub enum TagType {
    /// 8-bit unsigned integer
    BYTE = 1,
    /// 8-bit byte that contains a 7-bit ASCII code; the last byte must be zero
    ASCII = 2,
    /// 16-bit unsigned integer
    SHORT = 3,
    /// 32-bit unsigned integer
    LONG = 4,
    /// Fraction stored as two 32-bit unsigned integers
    RATIONAL = 5,
    /// 8-bit signed integer
    SBYTE = 6,
    /// 8-bit byte that may contain anything, depending on the field
    UNDEFINED = 7,
    /// 16-bit signed integer
    SSHORT = 8,
    /// 32-bit signed integer
    SLONG = 9,
    /// Fraction stored as two 32-bit signed integers
    SRATIONAL = 10,
    /// 32-bit IEEE floating point
    FLOAT = 11,
    /// 64-bit IEEE floating point
    DOUBLE = 12,
    /// 32-bit unsigned integer (offset)
    IFD = 13,
    /// BigTIFF 64-bit unsigned integer
    LONG8 = 16,
    /// BigTIFF 64-bit signed integer
    SLONG8 = 17,
    /// BigTIFF 64-bit unsigned integer (offset)
    IFD8 = 18,
}

pub fn tag_name(tag: &u16) -> String {
    if let Some(e) = LegacyTiffRootTag::n(*tag) {
        String::from(format!("{:?}", e))
    } else {
        String::from("UNKNOWN")
    }
}

pub(crate) fn dump_ifd_entries<T: TiffTagEnum>(ifd: &LegacyTiffIFD) -> String {
  let mut out = String::new();
  out.push_str(&format!("IFD entries: {}\n", ifd.entries.len()));
  out.push_str(&format!("{0:<34}  | {1:<10} | {2:<6} | {3}\n", "Tag", "Type", "Count", "Data"));
  for (tag, entry) in &ifd.entries {
    let tag_name = {
      if let Ok(name) = T::try_from(*tag) {
        String::from(format!("{:?}", name))
      } else {
        format!("<UNKNOWN:{}>", tag).into()
      }
    };
    out.push_str(&format!("{0:#06x} : {0:<6} {1:<20}| {2:<10} | {3:<6} | ", tag, tag_name, entry.typ_name(), entry.count));
    out.push_str(&entry.data_plaintext());
    out.push_str("\n");
  }
  out
}




/// Type to represent tiff values of type `IFD`
#[derive(Clone, Debug)]
pub struct Ifd(pub u32);

/// Type to represent tiff values of type `IFD8`
#[derive(Clone, Debug)]
pub struct Ifd8(pub u64);

/// Type to represent tiff values of type `RATIONAL`
#[derive(Clone, Debug)]
pub struct Rational {
    pub n: u32,
    pub d: u32,
}

/// Type to represent tiff values of type `SRATIONAL`
#[derive(Clone, Debug)]
pub struct SRational {
    pub n: i32,
    pub d: i32,
}
