use std::env;
use std::fs::File;
use std::error::Error;

extern crate rawloader;
extern crate time;

fn usage() {
  println!("benchmark <file>");
  std::process::exit(1);
}

static ITERATIONS: u64 = 50;

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
    Err(e) => {error(e.description());return},
  };
  let buffer = match rawloader::Buffer::new(&mut f) {
    Ok(val) => val,
    Err(e) => {error(&e); return},
  };
  let rawloader = rawloader::RawLoader::new();
  let from_time = time::precise_time_ns();
  {
    for _ in 0..ITERATIONS {
      let decoder = match rawloader.get_decoder(&buffer) {
        Ok(val) => val,
        Err(e) => {error(&e); return},
      };
      match decoder.image(false) {
        Ok(_) => {},
        Err(e) => error(&e),
      }
    }
  }
  let to_time = time::precise_time_ns();

  let avgtime = ((to_time-from_time)/ITERATIONS/1000) as f64 / 1000.0;
  println!("Average decode time: {} ms ({} iterations)", avgtime, ITERATIONS);
}
