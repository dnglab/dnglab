extern crate byteorder;
use self::byteorder::{ByteOrder, BigEndian, LittleEndian};

pub fn BEu32(buf: &[u8], pos: usize) -> u32 {
  BigEndian::read_u32(&buf[pos .. pos+4])
}
