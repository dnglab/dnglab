use std::env;
use std::path::PathBuf;
use std::time::Instant;

use rawler::decoders::RawDecodeParams;
use rawler::rawsource::RawSource;

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

  let rawfile = RawSource::new(&PathBuf::from(file)).expect("failed to load raw file");
  let rawloader = rawler::RawLoader::new();

  let from_time = Instant::now();
  {
    for _ in 0..ITERATIONS {
      let decoder = match rawloader.get_decoder(&rawfile) {
        Ok(val) => val,
        Err(e) => {
          error(&e.to_string());
          return;
        }
      };
      match decoder.raw_image(&rawfile, &RawDecodeParams::default(), false) {
        Ok(_) => {}
        Err(e) => error(&e.to_string()),
      }
    }
  }
  let duration = from_time.elapsed();

  let avgtime = ((duration.as_nanos() as u64) / ITERATIONS / 1000) as f64 / 1000.0;
  println!("Average decode time: {} ms ({} iterations)", avgtime, ITERATIONS);
}
