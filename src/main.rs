use std::env;
use std::fs::File;
use std::io::Read;
extern crate toml;

mod decoders;

fn usage() {
  println!("rawloader <file>");
}

fn main() {
  let args: Vec<_> = env::args().collect();
  if args.len() != 2 {
    usage();
    std::process::exit(2);
  }
  let file = &args[1];
  println!("Loading file \"{}\"", file);

  let mut f = File::open(file).unwrap();
  let mut buffer = Vec::new();
  f.read_to_end(&mut buffer).unwrap();

  println!("Total file is {} bytes in length", buffer.len());

  let rawloader = decoders::RawLoader::new("./data/cameras/");
  let decoder = rawloader.get_decoder(&buffer).unwrap();
  let camera = decoder.identify().unwrap();
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
}
