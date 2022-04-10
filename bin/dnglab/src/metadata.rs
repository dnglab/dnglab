// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use clap::ArgMatches;
use log::debug;
use rawler::decoders::{RawDecodeParams, RawMetadata};
use rawler::RawFile;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
  file_name: String,
  file_size: u64,
  image_count: usize,
  raw: Vec<RawMetadata>,
}

pub fn get_metadata<P: AsRef<Path>>(path: P) -> anyhow::Result<Metadata> {
  let fs_meta = std::fs::metadata(&path)?;
  let bufread = BufReader::new(File::open(&path)?);
  let mut rawfile = RawFile::new(&path, bufread);

  // Get decoder or return
  let decoder = rawler::get_decoder(&mut rawfile)?;

  let image_count = decoder.raw_image_count()?;

  let mut raw = Vec::with_capacity(image_count);

  for i in 0..image_count {
    raw.push(decoder.raw_metadata(&mut rawfile, RawDecodeParams { image_index: i })?);
  }

  Ok(Metadata {
    file_name: String::from(path.as_ref().to_str().unwrap()),
    file_size: fs_meta.len(),
    image_count,
    raw,
  })
}

/// Analyze a given image
pub async fn metadata(options: &ArgMatches<'_>) -> anyhow::Result<()> {
  let in_file = options.value_of("FILE").expect("FILE not available");

  debug!("Infile: {}", in_file);

  let data = get_metadata(in_file)?;

  if options.is_present("yaml") {
    let yaml = serde_yaml::to_string(&data)?;
    println!("{}", yaml);
  } else if options.is_present("json") {
    let json = serde_json::to_string_pretty(&data)?;
    println!("{}", json);
  } else {
    let json = serde_json::to_string_pretty(&data)?;
    println!("{}", json);
  }

  Ok(())
}
