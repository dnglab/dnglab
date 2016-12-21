use std::fs::File;
use std::io::prelude::*;

extern crate glob;
use self::glob::glob;
  
fn main() {
  let mut out = File::create("./data/cameras/all.toml").unwrap();

  for entry in glob("./data/cameras/*/**/*.toml").expect("Failed to read glob pattern") {
    out.write_all(b"[[cameras]]\n").unwrap();
    let path = entry.unwrap();
    let mut f = File::open(path).unwrap();
    let mut toml = String::new();
    f.read_to_string(&mut toml).unwrap();
    out.write_all(&toml.into_bytes()).unwrap();
    out.write_all(b"\n").unwrap();
  }
}
