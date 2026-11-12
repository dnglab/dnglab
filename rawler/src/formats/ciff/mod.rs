use std::collections::{HashMap, HashSet};

use crate::{bits::*, rawsource::RawSource};

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum CiffTag {
  //Null         = 0x0000,
  ColorInfo1 = 0x0032,
  MakeModel = 0x080a,
  ShotInfo = 0x102a,
  ColorInfo2 = 0x102c,
  WhiteBalance = 0x10a9,
  SensorInfo = 0x1031,
  ImageInfo = 0x1810,
  DecoderTable = 0x1835,
  //RawData      = 0x2005,
  //SubIFD       = 0x300a,
  //Exif         = 0x300b,
}

fn ct(tag: CiffTag) -> u16 {
  tag as u16
}

#[derive(Debug, Clone)]
pub struct CiffEntry {
  pub tag: u16,
  pub typ: u16,
  pub count: usize,
  pub bytesize: usize,
  pub data_offset: usize,
  pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct CiffIFD {
  entries: HashMap<u16, CiffEntry>,
  subifds: Vec<CiffIFD>,
}

pub fn is_ciff(file: &RawSource) -> bool {
  match file.subview(0, 14) {
    Ok(buf) => buf[6..14] == b"HEAPCCDR"[..],
    Err(_) => false,
  }
}

/// Limits of a parse against crafted files, whose entries can all point
/// to the same data or sub-heap
struct ParseState {
  /// Bytes left for the entries and the data they copy
  budget: usize,
  /// Sub-heaps already parsed, as start and end
  visited: HashSet<(usize, usize)>,
}

impl ParseState {
  /// Take `bytes` from the budget
  fn charge(&mut self, bytes: usize) -> Result<(), String> {
    self.budget = self
      .budget
      .checked_sub(bytes)
      .ok_or("CIFF: the entries need more data than twice the file size, the file may be crafted")?;
    Ok(())
  }
}

impl CiffIFD {
  pub fn new_file(file: &RawSource) -> Result<CiffIFD, String> {
    let data = file.buf();
    // The header length is the offset of the root heap
    let header_len = data.get(2..6).map(|_| LEu32(data, 2) as usize).ok_or("CIFF: file is too short for a header")?;
    CiffIFD::new(data, header_len, data.len(), 1)
  }

  /// Heap from `start` to `end` in `buf`. It ends with the offset of its
  /// entry table, which follows the value data.
  pub fn new(buf: &[u8], start: usize, end: usize, depth: u32) -> Result<CiffIFD, String> {
    let mut state = ParseState {
      budget: buf.len().saturating_mul(2).saturating_add(1024),
      visited: HashSet::from([(start, end)]),
    };
    Self::parse(buf, start, end, depth, &mut state)
  }

  fn parse(buf: &[u8], start: usize, end: usize, depth: u32, state: &mut ParseState) -> Result<CiffIFD, String> {
    let mut entries = HashMap::new();
    let mut subifds = Vec::new();

    let out_of_heap = || format!("CIFF: heap {}..{} has no valid entry table", start, end);
    let table_end = end
      .checked_sub(4)
      .filter(|table_end| *table_end >= start && end <= buf.len())
      .ok_or_else(out_of_heap)?;
    let valuedata_size = LEu32(buf, table_end) as usize;
    let table = start
      .checked_add(valuedata_size)
      .filter(|table| table.saturating_add(2) <= table_end)
      .ok_or_else(out_of_heap)?;
    let dircount = LEu16(buf, table) as usize;
    if table + 2 + dircount * 10 > table_end {
      return Err(out_of_heap());
    }

    for i in 0..dircount {
      let entry_offset: usize = start + valuedata_size + 2 + i * 10;
      state.charge(10)?;
      let mut e = CiffEntry::header(buf, start, entry_offset)?;
      let sub_heap = (e.data_offset, e.data_offset + e.bytesize);
      if e.typ == 0x2800 || e.typ == 0x3000 {
        // SubIFDs
        if depth < 10 && state.visited.insert(sub_heap) {
          // Avoid infinite looping IFDs
          let ifd = CiffIFD::parse(buf, sub_heap.0, sub_heap.1, depth + 1, state);
          match ifd {
            Ok(val) => {
              subifds.push(val);
            }
            Err(_) => {
              // Ignore unparsable IFDs
              state.charge(e.bytesize)?;
              e.data = buf[sub_heap.0..sub_heap.1].to_vec();
              entries.insert(e.tag, e);
            }
          }
        }
      } else {
        state.charge(e.bytesize)?;
        e.data = buf[sub_heap.0..sub_heap.1].to_vec();
        entries.insert(e.tag, e);
      }
    }

    Ok(CiffIFD { entries, subifds })
  }

  pub fn find_entry(&self, tag: CiffTag) -> Option<&CiffEntry> {
    if self.entries.contains_key(&ct(tag)) {
      self.entries.get(&ct(tag))
    } else {
      for ifd in &self.subifds {
        if let Some(x) = ifd.find_entry(tag) {
          return Some(x);
        }
      }
      None
    }
  }
}

impl CiffEntry {
  pub fn new(buf: &[u8], value_data: usize, offset: usize) -> Result<CiffEntry, String> {
    let mut entry = Self::header(buf, value_data, offset)?;
    entry.data = buf[entry.data_offset..entry.data_offset + entry.bytesize].to_vec();
    Ok(entry)
  }

  /// The entry at `offset` without its data, which is checked to be in `buf`
  fn header(buf: &[u8], value_data: usize, offset: usize) -> Result<CiffEntry, String> {
    let p = LEu16(buf, offset);
    let tag = p & 0x3fff;
    let datalocation = (p & 0xc000) as usize;
    let typ = p & 0x3800;

    let (bytesize, data_offset) = match datalocation {
      // Data is offset in value_data
      0x0000 => (
        LEu32(buf, offset + 2) as usize,
        (LEu32(buf, offset + 6) as usize)
          .checked_add(value_data)
          .ok_or_else(|| format!("CIFF: data offset of tag 0x{:x} overflows", tag))?,
      ),
      // Data is stored directly in entry
      0x4000 => (8, offset + 2),
      val => return Err(format!("CIFF: Don't know about data location {:x}", val)),
    };
    if data_offset.checked_add(bytesize).is_none_or(|end| end > buf.len()) {
      return Err(format!(
        "CIFF: data of tag 0x{:x} ({} bytes at {}) is outside of the file",
        tag, bytesize, data_offset
      ));
    }
    let count = bytesize >> CiffEntry::element_shift(typ);

    Ok(CiffEntry {
      tag,
      typ,
      count,
      bytesize,
      data_offset,
      data: Vec::new(),
    })
  }

  pub fn element_shift(typ: u16) -> usize {
    match typ {
      // Byte and ASCII
      0x0000 | 0x8000 => 0,
      // Short
      0x1000 => 1,
      // Long, Mix, Sub1 and Sub2
      0x1800 | 0x2000 | 0x2800 | 0x3000 => 2,
      // Default to 0
      _ => 0,
    }
  }

  pub fn get_strings(&self) -> Vec<String> {
    String::from_utf8_lossy(&self.data).split_terminator('\0').map(|x| x.to_string()).collect()
  }

  pub fn get_u32(&self, idx: usize) -> u32 {
    match self.typ {
      0x0000 | 0x8000 => self.data[idx] as u32,
      0x1000 => LEu16(&self.data, idx * 2) as u32,
      0x1800 | 0x2000 | 0x2800 | 0x3000 => LEu32(&self.data, idx * 4),
      _ => panic!("{}", format!("Trying to read typ {} for a u32", self.typ)),
    }
  }

  pub fn get_usize(&self, idx: usize) -> usize {
    self.get_u32(idx) as usize
  }
  pub fn get_f32(&self, idx: usize) -> f32 {
    self.get_u32(idx) as f32
  }
  pub fn get_force_u16(&self, idx: usize) -> u16 {
    LEu16(&self.data, idx * 2)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// CIFF heap with the `values` and the `entries` of tag and type, size
  /// and offset in the values
  fn heap(values: &[u8], entries: &[(u16, u32, u32)]) -> Vec<u8> {
    let mut data = values.to_vec();
    data.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for (tag, size, offset) in entries {
      data.extend_from_slice(&tag.to_le_bytes());
      data.extend_from_slice(&size.to_le_bytes());
      data.extend_from_slice(&offset.to_le_bytes());
    }
    data.extend_from_slice(&(values.len() as u32).to_le_bytes());
    data
  }

  #[test]
  fn heap_bounds() {
    let data = heap(&[1, 0, 2, 0], &[(0x1031, 4, 0)]);
    let ifd = CiffIFD::new(&data, 0, data.len(), 1).expect("heap");
    assert!(ifd.find_entry(CiffTag::SensorInfo).is_some());
    // Heaps beyond the buffer, without room for the table offset, or
    // starting behind their end
    assert!(CiffIFD::new(&data, 0, data.len() + 1, 1).is_err());
    assert!(CiffIFD::new(&data, 0, 3, 1).is_err());
    assert!(CiffIFD::new(&data, 10, 8, 1).is_err());
    // The table offset or the entry count exceed the heap
    let mut bad = data.clone();
    let len = bad.len();
    bad[len - 4..].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(CiffIFD::new(&bad, 0, len, 1).is_err());
    let mut bad = data.clone();
    bad[4..6].copy_from_slice(&2_u16.to_le_bytes());
    assert!(CiffIFD::new(&bad, 0, len, 1).is_err());
  }

  #[test]
  fn entries_sharing_data_are_limited() {
    // 1000 entries with the same 1000 bytes
    let data = heap(&[0; 1000], &[(0x0032, 1000, 0); 1000]);
    assert!(CiffIFD::new(&data, 0, data.len(), 1).is_err());
    // A few of them are fine
    let data = heap(&[0; 1000], &[(0x0032, 1000, 0); 2]);
    assert!(CiffIFD::new(&data, 0, data.len(), 1).is_ok());
  }

  #[test]
  fn sub_heaps_are_parsed_once() {
    // A heap with 8 sub-heap entries of itself, 8^9 parses without a limit
    let len = 2 + 8 * 10 + 4;
    let data = heap(&[], &[(0x300a, len, 0); 8]);
    let ifd = CiffIFD::new(&data, 0, data.len(), 1).expect("heap");
    assert!(ifd.subifds.is_empty());
    // A sub-heap of 2 entries pointing to the same heap is parsed once
    let inner = heap(&[1, 0, 2, 0], &[(0x1031, 4, 0)]);
    let mut values = inner.clone();
    values.resize(values.len() + 2, 0);
    let data = heap(&values, &[(0x300a, inner.len() as u32, 0), (0x300b, inner.len() as u32, 0)]);
    let ifd = CiffIFD::new(&data, 0, data.len(), 1).expect("heap");
    assert_eq!(ifd.subifds.len(), 1);
    assert!(ifd.find_entry(CiffTag::SensorInfo).is_some());
  }

  #[test]
  fn short_files() {
    for data in [&b""[..], b"II", b"II\x1a\0"] {
      assert!(CiffIFD::new_file(&RawSource::new_from_slice(data)).is_err());
    }
  }

  #[test]
  fn entry_data_outside_of_the_file() {
    // SHORT entry of 100 bytes at offset 2 of 4 value bytes
    // The entry follows the values and the entry count
    let data = heap(&[1, 0, 2, 0], &[(0x1031, 100, 2)]);
    assert!(CiffEntry::new(&data, 0, 6).is_err());
    // Offset and size overflow usize
    let data = heap(&[1, 0, 2, 0], &[(0x1031, u32::MAX, u32::MAX)]);
    assert!(CiffEntry::new(&data, usize::MAX - 2, 6).is_err());
    let data = heap(&[1, 0, 2, 0], &[(0x1031, 4, 0)]);
    assert_eq!(CiffEntry::new(&data, 0, 6).expect("entry").data, [1, 0, 2, 0]);
  }
}
