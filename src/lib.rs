#[macro_use] extern crate enum_primitive;
extern crate num;

#[macro_use] extern crate lazy_static;

extern crate itertools;

pub mod decoders;
use decoders::{RawLoader, Image};
pub mod imageops;

lazy_static! {
  static ref LOADER: RawLoader = decoders::RawLoader::new();
}

pub fn decode(path: &str) -> Result<Image, String> {
  LOADER.decode_safe(path)
}
