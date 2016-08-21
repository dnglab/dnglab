use std::env;
use std::fs::File;
use std::error::Error;
use std::io::prelude::*;
extern crate toml;

mod decoders;

fn usage() {
  println!("rawloader <file>");
  std::process::exit(1);
}

fn error(err: &str) {
  println!("ERROR: {}", err);
  std::process::exit(2);
}

fn main() {
  let args: Vec<_> = env::args().collect();
  if args.len() != 2 {
    usage();
  }
  let file = &args[1];
  println!("Loading file \"{}\"", file);

  let mut f = match File::open(file) {
    Ok(val) => val,
    Err(e) => {error(e.description());unreachable!()},
  };
  let mut buffer = Vec::new();
  if let Err(err) = f.read_to_end(&mut buffer) {
    error(err.description());
  }
  println!("Total file is {} bytes in length", buffer.len());

  let rawloader = decoders::RawLoader::new("./data/cameras/");
  let decoder = match rawloader.get_decoder(&buffer) {
    Ok(val) => val,
    Err(e) => {error(&e);unreachable!()},
  };

  let camera = match decoder.identify() {
    Ok(val) => val,
    Err(e) => {error(&e);unreachable!()},
  };
  println!("Found camera \"{}\" model \"{}\"", camera.make, camera.model);

  let image = decoder.image();
  println!("Image size is {}x{}", image.width, image.height);
  println!("WB coeffs are {},{},{},{}", image.wb_coeffs[0],
                                        image.wb_coeffs[1],
                                        image.wb_coeffs[2],
                                        image.wb_coeffs[3]);

  let mut sum: u64 = 0;
  for i in 0..(image.width*image.height) {
    sum += image.data[i as usize] as u64;
  }
  println!("Image sum: {}", sum);
  let count: u64 = (image.width as u64) * (image.height as u64);
  println!("Image avg: {}", sum/count);

  let mut f = match File::create(format!("{}.ppm",file)) {
    Ok(val) => val,
    Err(e) => {error(e.description());unreachable!()},
  };
  let preamble = format!("P6 {} {} {}\n", image.width, image.height, 4095).into_bytes();
  if let Err(err) = f.write_all(&preamble) {
    error(err.description());
  }
  for row in 0..image.height {
    let from: usize = (row as usize) * (image.width as usize);
    let to: usize = ((row+1) as usize) * (image.width as usize);
    let imgline = &image.data[from .. to];

    for pixel in imgline {
      let bytes = [(pixel>>4) as u8, (pixel&0x0f) as u8, (pixel>>4) as u8, (pixel&0x0f) as u8, (pixel>>4) as u8, (pixel&0x0f) as u8];
       if let Err(err) = f.write_all(&bytes) {
        error(err.description());
      }
    }
  }
}
