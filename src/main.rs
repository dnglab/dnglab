use std::env;
use std::fs::File;
use std::io::Read;
use std::mem;

mod decoders;
use decoders::Decoder;

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
}
