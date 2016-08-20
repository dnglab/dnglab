use std::env;
use std::fs::File;
use std::io::Read;

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

  let decoder = decoders::get_decoder(&buffer).unwrap();
  println!("Found camera \"{}\" model \"{}\"", decoder.make(), decoder.model());
  let image = decoder.image();
  println!("Image size is {}x{}", image.width, image.height);

  let mut sum: u64 = 0;
  for i in 0..(image.width*image.height) {
    sum += image.data[i as usize] as u64;
  }
  println!("Image sum: {}", sum);
  let count: u64 = (image.width as u64) * (image.height as u64);
  println!("Image avg: {}", sum/count);
}
