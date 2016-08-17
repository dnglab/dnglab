use std::env;
use std::fs::File;
use std::io::Read;

extern crate byteorder;
use byteorder::{ByteOrder, BigEndian, LittleEndian};

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
  println!("buffer[0..3] is big endian {}", BigEndian::read_u32(&buffer[2..6]));
  println!("buffer[0..3] is little endian {}", LittleEndian::read_u32(&buffer[2..6]));

  let decoder = decoders::get_decoder(&buffer);
  println!("Found camera \"{}\" model \"{}\"", decoder.make(), decoder.model());
}
