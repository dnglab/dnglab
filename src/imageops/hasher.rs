extern crate sha2;
use self::sha2::Digest;

extern crate bincode;
extern crate serde;
use self::serde::Serialize;

use std;
use std::io::Write;
use std::fmt;
use std::fmt::Debug;

type HashType = self::sha2::Sha256;
pub type BufHash = [u8;32];

#[derive(Copy, Clone)]
pub struct BufHasher {
  hash: HashType,
}
impl BufHasher {
  pub fn new() -> BufHasher {
    BufHasher {
      hash: HashType::default(),
    }
  }
  pub fn result(&self) -> BufHash {
    let mut result = BufHash::default();
    for (i, byte) in self.hash.result().into_iter().enumerate() {
      result[i] = byte;
    }
    result
  }
}
impl Debug for BufHasher {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    write!(f, "BufHasher {{ {:?} }}", self.result())
  }
}

impl Write for BufHasher {
  fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
    self.hash.input(buf);
    Ok(buf.len())
  }
  fn flush(&mut self) -> std::io::Result<()> {Ok(())}
}

impl BufHasher {
  pub fn from_serialize<T>(&mut self, obj: &T) where T: Serialize {
    self::bincode::serialize_into(self, obj, self::bincode::Infinite).unwrap();
  }
}
