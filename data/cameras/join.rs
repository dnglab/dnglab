use std::fs::File;
use std::io::prelude::*;

extern crate glob;
use self::glob::glob;
  
fn main() {
  let mut out = File::create("./data/cameras/all.toml").unwrap();

  out.write_all("[cameras]\n".as_bytes()).unwrap();

  let mut count = 1;

  for entry in glob("./data/cameras/*/**/*.toml").expect("Failed to read glob pattern") {
    out.write_all(&format!("[cameras.{}]\n",count).into_bytes()).unwrap();
    let path = entry.unwrap();
    let mut f = File::open(path).unwrap();
    let mut toml = String::new();
    f.read_to_string(&mut toml).unwrap();
    out.write_all(&toml.into_bytes()).unwrap();
    count += 1;
  }
}
