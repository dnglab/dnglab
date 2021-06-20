// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::fs::File;

use clap::ArgMatches;
use log::debug;

use rawler::formats::bmff::parse_file;

pub fn analyze(options: &ArgMatches<'_>) -> anyhow::Result<()> {
  let in_file = options.value_of("FILE").expect("FILE not available");

  debug!("Infile: {}", in_file);

  let mut in_f = File::open(in_file)?;

  let filebox = parse_file(&mut in_f).unwrap();

  let j = serde_json::to_string_pretty(&filebox)?;

  println!("{}", j);

  /*
  let mut data = Vec::new();
  in_f.read_to_end(&mut data);

  let ifds = tiff::TiffIFD::new_file(&data, &vec![TiffRootTag::ExifIFDPointer.into()]).unwrap();

  println!("{}", ifds);
  */

  Ok(())
}
