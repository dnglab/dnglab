// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use clap::ArgMatches;
use rayon::prelude::*;
use std::fs::create_dir_all;
use std::path::PathBuf;
use std::time::Instant;

use crate::filemap::{FileMap, MapMode};
use crate::jobs::raw2dng::{JobResult, Raw2DngJob};
use crate::jobs::Job;
use crate::{
  dnggen::{DngCompression, DngParams},
  AppError, Result, PKG_NAME, PKG_VERSION,
};

const SUPPORTED_FILE_EXT: [&'static str; 1] = ["CR3"];

/// Entry point for Clap sub command `convert`
pub fn convert(options: &ArgMatches<'_>) -> anyhow::Result<()> {
  let now = Instant::now();
  let in_path = PathBuf::from(options.value_of("INPUT").expect("INPUT not available"));
  let out_path = PathBuf::from(options.value_of("OUTPUT").expect("OUTPUT not available"));
  let recursive = options.is_present("recursive");

  let proc = MapMode::new(&in_path, &out_path)?;

  // List of jobs
  let mut jobs = Vec::new();

  // drop pathes to prevent use
  drop(in_path);
  drop(out_path);

  match proc {
    // We have only one input file, so output must be a file, too.
    MapMode::File(sd) => {
      let job = generate_job(&sd, options)?;
      jobs.push(job);
    }
    // Input is directory, to process all files
    MapMode::Dir(sd) => {
      let list = sd.file_list(recursive, |file| {
        if let Some(ext) = file.extension().map(|ext| ext.to_string_lossy()) {
          is_ext_supported(&ext)
        } else {
          false
        }
      })?;
      for entry in list {
        let job = generate_job(&entry, options)?;
        jobs.push(job);
      }
    }
  }

  let verbose = options.is_present("verbose");

  let results: Vec<JobResult> = jobs
    .par_iter()
    .map(|job| {
      let res = job.execute();
      if verbose {
        println!("{}", res);
      }
      res
    })
    .collect();

  let total = results.len();
  let success = results.iter().filter(|j| j.error.is_none()).count();
  let failure = results.iter().filter(|j| j.error.is_some()).count();

  if failure == 0 {
    println!("Converted {}/{} files", success, total,);
  } else {
    eprintln!("Converted {}/{} files, {} failed:", success, total, failure,);
    for failed in results.iter().filter(|j| j.error.is_some()) {
      eprintln!("   {}", failed.job.input.display().to_string());
    }
  }
  println!("Total time: {:.2}s", now.elapsed().as_secs_f32());
  Ok(())
}

/// Convert given raw file to dng file
fn generate_job(entry: &FileMap, options: &ArgMatches<'_>) -> Result<Raw2DngJob> {
  // Params for conversion process
  let params = DngParams {
    no_embedded: options.is_present("noembedded"),
    compression: match options.value_of("compression") {
      Some("lossless") => DngCompression::Lossless,
      Some("none") => DngCompression::Uncompressed,
      Some(s) => {
        println!("Unknown compression: {}", s);
        return Err(AppError::InvalidArgs.into());
      }
      None => DngCompression::Lossless,
    },
    no_crop: options.is_present("nocrop"),
    software: format!("{} {}", PKG_NAME, PKG_VERSION),
  };

  let mut output = PathBuf::from(&entry.dest);
  if !output.set_extension("dng") {
    return Err(AppError::General("Unable to rename target to dng".into()));
  }

  match output.parent() {
    Some(parent) => {
      if !parent.exists() {
        create_dir_all(parent)?;
        if options.is_present("verbose") {
          println!("Creating output directory '{}'", parent.display());
        }
      }
    }
    None => {
      return Err(AppError::General(format!("Output path has no parent directory")));
    }
  }

  Ok(Raw2DngJob {
    input: PathBuf::from(&entry.src),
    output,
    replace: options.is_present("override"),
    params,
  })
}

/// Check if file extension is a supported extension
fn is_ext_supported<T: AsRef<str>>(ext: T) -> bool {
  let uc = ext.as_ref().to_uppercase();
  SUPPORTED_FILE_EXT.iter().any(|ext| ext.eq(&uc))
}
