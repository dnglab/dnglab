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
  let out_dir = env::var("OUT_DIR").expect("Missing ENV OUT_DIR");
  let dest_path = Path::new(&out_dir).join("cameras.toml");
  let mut out = File::create(dest_path).expect("Unable to create output file");

  for entry in glob("./data/cameras/*/**/*.toml").expect("Failed to read glob pattern") {
    //out.write_all(b"[[cameras]]\n").expect("Failed to write camera TOML");
    let path = entry.expect("Invalid glob entry");
    let mut f = File::open(&path).expect("failed to open camera definition file");
    let mut toml = String::new();
    f.read_to_string(&mut toml).expect("Failed to read camera definition file");

    {
      match toml.parse::<Value>() {
        Ok(_) => {
        },
        Err(e) => panic!("Error parsing {:?}: {:?}", path, e),
      };
    }

    out.write_all(&toml.into_bytes()).expect("Failed to write");
    out.write_all(b"\n").expect("Failed to write");
  }

  // Check for a minimum version
  if version().expect("version failed") < Version::parse("1.31.0").expect("version parse failed") {
      println!("cargo:rustc-cfg=needs_chunks_exact");
  }
}
