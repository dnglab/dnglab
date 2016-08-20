use std::collections::HashMap;
use decoders::basics::*;

enum Tag {
  SUBIFDS        = 0x014A,
  EXIFIFDPOINTER = 0x8769,
}

fn t (tag: Tag) -> u16 {
  tag as u16
}

pub struct TiffEntry {
  tag: u16,
}

pub struct TiffIFD {
  entries: HashMap<u16,TiffEntry>,
  subifds: Vec<TiffIFD>,
}

impl TiffIFD {
  pub fn new(buf: &[u8], offset: usize, depth: u32) -> TiffIFD {
    let mut entries = HashMap::new();
    let mut subifds = Vec::new();

    let num = BEu16(buf, offset); // Directory entries in this IFD

    for i in 0..num {
      let entry_offset: usize = offset + 2 + (i as usize)*12;
      let entry = TiffEntry::new(buf, entry_offset, offset);

      if entry.tag == t(Tag::SUBIFDS) || entry.tag == t(Tag::EXIFIFDPOINTER) {
        //subifds.push(TiffIFD::new(buf, offset, depth+1);
      } else {
        entries.insert(entry.tag, entry);
      }
    }

    TiffIFD {
      entries: entries,
      subifds: subifds,
    }
  }
}

impl TiffEntry {
  pub fn new(buf: &[u8], offset: usize, parent_offset: usize) -> TiffEntry {
    TiffEntry {
      tag: BEu16(buf, offset),
    }
  }
}
