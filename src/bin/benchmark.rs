use std::env;
extern crate rawloader;
extern crate time;
use rawloader::decoders;

fn usage() {
  println!("benchmark <file>");
  std::process::exit(1);
}

static STEP_ITERATIONS: u32 = 10;
static MIN_ITERATIONS: u32 = 100;
static MIN_TIME: u64 = 10000000000;

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
  let mut iterations = 0;
  loop {
    for _ in 0..STEP_ITERATIONS {
      match rawloader.decode_safe(file) {
        Ok(_) => {},
        Err(e) => error(&e),
      }
    }
    iterations += STEP_ITERATIONS;
    let to_time = time::precise_time_ns();
    if iterations >= MIN_ITERATIONS && (to_time-from_time) >= MIN_TIME {
      println!("Average decode time: {} ms ({} iterations)", (to_time-from_time)/(iterations as u64)/1000000, iterations);
      break;
    }
  }
}
