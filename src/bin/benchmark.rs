use std::env;
extern crate rawloader;
extern crate time;
use rawloader::decoders;

fn usage() {
  println!("benchmark <file>");
  std::process::exit(1);
}

static ITERATIONS: u32 = 100;

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

  let rawloader = decoders::RawLoader::new();
  let from_time = time::precise_time_ns();
  for _ in 0..ITERATIONS {
    match rawloader.decode_safe(file) {
      Ok(_) => {},
      Err(e) => error(&e),
    }
  }
  let to_time = time::precise_time_ns();
  println!("Decoded {} times in {} ms", ITERATIONS, (to_time - from_time)/1000000);
}
