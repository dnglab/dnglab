// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use clap::ArgMatches;
use log::debug;
use std::{
  io::{BufWriter, Write},
  path::PathBuf,
};

use rawler::analyze::{analyze_file, extract_raw_pixels, raw_as_pgm};

/// Analyze a given image
pub fn analyze(options: &ArgMatches<'_>) -> anyhow::Result<()> {
  let in_file = options.value_of("FILE").expect("FILE not available");

  debug!("Infile: {}", in_file);

  if options.is_present("meta") {
    let analyze = analyze_file(&PathBuf::from(in_file)).unwrap();

    if options.is_present("yaml") {
      let yaml = serde_yaml::to_string(&analyze)?;
      println!("{}", yaml);
    } else {
      let json = serde_json::to_string_pretty(&analyze)?;
      println!("{}", json);
    }
  } else if options.is_present("pixel") {
    let (width, height, buf) = extract_raw_pixels(&PathBuf::from(in_file)).unwrap();

    dump_pgm(width, height, &buf)?;
  }

  Ok(())
}

/// Write image to STDOUT as PGM
fn dump_pgm(width: usize, height: usize, buf: &[u16]) -> std::io::Result<()> {
  let out = std::io::stdout();
  let mut writer = BufWriter::new(out);
  raw_as_pgm(width, height, &buf, &mut writer)?;
  writer.flush()
}
