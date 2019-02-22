#[macro_use]
extern crate afl;
extern crate rawloader;

fn main() {
  rawloader::force_initialization();

  fuzz_nohook!(|data: &[u8]| {
    rawloader::decode(&mut &data[..]).ok();
  });
}
