use std::collections::HashMap;
use decoders::basics::*;

enum Tag {
  SUBIFDS        = 0x014A,
  EXIFIFDPOINTER = 0x8769,
}
                          // 0-1-2-3-4-5-6-7-8-9-10-11-12-13
const DATASIZES:  [u8;14] = [0,1,1,2,4,8,1,1,2,4, 8, 4, 8, 4];
const DATASHIFTS: [u8;14] = [0,0,0,1,2,3,0,0,1,2, 3, 2, 3, 2];

fn t (tag: Tag) -> u16 {
  tag as u16
}

pub struct TiffEntry<'a> {
  tag: u16,
  typ: u16,
  count: u32,
  data: &'a [u8],
}

pub struct TiffIFD<'a> {
  entries: HashMap<u16,TiffEntry<'a>>,
  subifds: Vec<TiffIFD<'a>>,
}

impl<'a> TiffIFD<'a> {
  pub fn new(buf: &'a[u8], offset: usize, depth: u32) -> TiffIFD<'a> {
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

impl<'a> TiffEntry<'a> {
  pub fn new(buf: &'a[u8], offset: usize, parent_offset: usize) -> TiffEntry<'a> {
    let tag = BEu16(buf, offset);
    let mut typ = BEu16(buf, offset+2);
    let count = BEu32(buf, offset+4);

    // If we don't know the type assume byte data
    if typ == 0 || typ > 13 {
      typ = 1;
    }

    let bytesize: usize = (count as usize) << DATASHIFTS[typ as usize];
    let doffset: usize = if bytesize <= 4 {
      (offset + 8)
    } else {
      BEu32(buf, offset+8) as usize
    };

    TiffEntry {
      tag: tag,
      typ: typ,
      count: count,
      data: &buf[doffset .. doffset+bytesize-1],
    }
  }
}
