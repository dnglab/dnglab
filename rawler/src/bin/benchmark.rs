use std::env;
use std::fs::File;
use std::time::Instant;

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
    Err(e) => {error(&e.to_string());return},
  };
  let buffer = match rawler::Buffer::new(&mut f) {
    Ok(val) => val,
    Err(e) => {error(&e); return},
  };
  let rawloader = rawler::RawLoader::new();
  let from_time = Instant::now();
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
  let duration = from_time.elapsed();

  let avgtime = ((duration.as_nanos() as u64)/ITERATIONS/1000) as f64 / 1000.0;
  println!("Average decode time: {} ms ({} iterations)", avgtime, ITERATIONS);
}
