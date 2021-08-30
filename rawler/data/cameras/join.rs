use std::fs::File;
use std::io::prelude::*;
use std::env;
use std::path::Path;

extern crate glob;
use self::glob::glob;
extern crate toml;
use toml::Value;
extern crate rustc_version;
use rustc_version::{version, Version};
  
fn main() {
  let out_dir = env::var("OUT_DIR").unwrap();
  let dest_path = Path::new(&out_dir).join("cameras.toml");
  let mut out = File::create(dest_path).unwrap();

  for entry in glob("./data/cameras/*/**/*.toml").expect("Failed to read glob pattern") {
    out.write_all(b"[[cameras]]\n").unwrap();
    let path = entry.unwrap();
    let mut f = File::open(path.clone()).unwrap();
    let mut toml = String::new();
    f.read_to_string(&mut toml).unwrap();

    {
      match toml.parse::<Value>() {
        Ok(_) => {
        },
        Err(e) => panic!("Error parsing {:?}: {:?}", path, e),
      };
    }

    out.write_all(&toml.into_bytes()).unwrap();
    out.write_all(b"\n").unwrap();
  }

  // Check for a minimum version
  if version().unwrap() < Version::parse("1.31.0").unwrap() {
      println!("cargo:rustc-cfg=needs_chunks_exact");
  }
}
