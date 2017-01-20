#[macro_use] extern crate enum_primitive;
extern crate num;

#[macro_use] extern crate lazy_static;

extern crate itertools;

#[doc(hidden)] pub mod decoders;
pub use decoders::Image;
#[doc(hidden)] pub mod imageops;

lazy_static! {
  static ref LOADER: decoders::RawLoader = decoders::RawLoader::new();
}

pub fn decode(path: &str) -> Result<Image, String> {
  LOADER.decode_safe(path)
}
