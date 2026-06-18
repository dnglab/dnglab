use std::collections::HashMap;

use crate::{bits::*, rawsource::RawSource};

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum CiffTag {
  //Null         = 0x0000,
  ColorInfo1 = 0x0032,
  MakeModel = 0x080a,
  //ShotInfo     = 0x102a,
  ColorInfo2 = 0x102c,
  WhiteBalance = 0x10a9,
  SensorInfo = 0x1031,
  //ImageInfo    = 0x1810,
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

impl CiffIFD {
  pub fn new_file(file: &RawSource) -> Result<CiffIFD, String> {
    let data = file.as_vec().map_err(|err| format!("Failed to read whole file: {:?}", err))?;

    CiffIFD::new(&data, LEu32(&data, 2) as usize, data.len(), 1)
  }

  pub fn new(buf: &[u8], start: usize, end: usize, depth: u32) -> Result<CiffIFD, String> {
    let mut entries = HashMap::new();
    let mut subifds = Vec::new();

    // The CIFF directory layout stores its value-data size in the last 4 bytes
    // of the [start, end) block. A truncated file where `end < 4` (e.g. an empty
    // input) would underflow `end - 4`; the directory count offset and entry
    // offsets are likewise derived from file values and can overflow `usize`.
    // For a well-formed CIFF every offset stays inside the block, so these
    // checks never fire on valid input — they only turn a corrupt directory into
    // a decode error instead of a panic.
    let valuedata_pos = end.checked_sub(4).ok_or("CIFF: block too small for value-data size")?;
    let valuedata_size = LEu32(buf, valuedata_pos) as usize;
    let dircount_pos = start.checked_add(valuedata_size).ok_or("CIFF: value-data offset overflow")?;
    let dircount = LEu16(buf, dircount_pos) as usize;

    for i in 0..dircount {
      let entry_offset: usize = start
        .checked_add(valuedata_size)
        .and_then(|v| v.checked_add(2))
        .and_then(|v| i.checked_mul(10).and_then(|im| v.checked_add(im)))
        .ok_or("CIFF: directory entry offset overflow")?;
      let e = CiffEntry::new(buf, start, entry_offset)?;
      if e.typ == 0x2800 || e.typ == 0x3000 {
        // SubIFDs
        if depth < 10 {
          // Avoid infinite looping IFDs
          let sub_end = e.data_offset.checked_add(e.bytesize).ok_or("CIFF: sub-IFD extent overflow")?;
          let ifd = CiffIFD::new(buf, e.data_offset, sub_end, depth + 1);
          match ifd {
            Ok(val) => {
              subifds.push(val);
            }
            Err(_) => {
              entries.insert(e.tag, e);
            } // Ignore unparsable IFDs
          }
        }
      } else {
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
    let p = LEu16(buf, offset);
    let tag = p & 0x3fff;
    let datalocation = (p & 0xc000) as usize;
    let typ = p & 0x3800;

    let (bytesize, data_offset) = match datalocation {
      // Data is offset in value_data
      0x0000 => (
        LEu32(buf, offset + 2) as usize,
        // `data_offset = value_data + stored_offset` is built from file values
        // and can overflow `usize` on a corrupt entry; a valid entry's offset
        // lands inside the buffer, so the sum is unchanged for well-formed input.
        (LEu32(buf, offset + 6) as usize)
          .checked_add(value_data)
          .ok_or("CIFF: entry data offset overflow")?,
      ),
      // Data is stored directly in entry
      0x4000 => (8, offset + 2),
      val => return Err(format!("CIFF: Don't know about data location {:x}", val)),
    };
    // `&buf[data_offset..data_offset + bytesize]` indexed directly; both bounds
    // come from the (untrusted) file and can exceed the buffer or overflow. For
    // a valid entry the data slice lies inside the buffer, so this `get`
    // succeeds and the copied data is identical; a corrupt entry becomes a
    // decode error instead of an out-of-range panic.
    let data_end = data_offset.checked_add(bytesize).ok_or("CIFF: entry data extent overflow")?;
    let data = buf.get(data_offset..data_end).ok_or("CIFF: entry data out of range")?;
    let count = bytesize >> CiffEntry::element_shift(typ);

    Ok(CiffEntry {
      tag,
      typ,
      count,
      bytesize,
      data_offset,
      data: Vec::from(data),
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
