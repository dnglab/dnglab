// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use clap::ArgMatches;
use rawler::tags::DngTag;
use rawler::formats::tiff::{Entry, TiffReader, Value};
use rayon::prelude::*;
use std::fs::{create_dir_all, File};
use std::io::BufReader;
use std::path::PathBuf;
use std::time::Instant;

use crate::filemap::{FileMap, MapMode};
use crate::jobs::extractraw::{ExtractRawJob, JobResult};
use crate::jobs::Job;
use crate::{AppError, Result};

const SUPPORTED_FILE_EXT: [&'static str; 1] = ["DNG"];

/// Entry point for Clap sub command `extract`
pub fn extract(options: &ArgMatches<'_>) -> anyhow::Result<()> {
  let now = Instant::now();
  let in_path = PathBuf::from(options.value_of("INPUT").expect("INPUT not available"));
  let out_path = PathBuf::from(options.value_of("OUTPUT").expect("OUTPUT not available"));
  let recursive = options.is_present("recursive");

  if !out_path.exists() {
    return Err(AppError::General(format!("Output path not exists")).into());
  }
  if !out_path.is_dir() {
    return Err(AppError::General(format!("Output path must be directory")).into());
  }

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
    println!("Extracted {}/{} files", success, total,);
  } else {
    eprintln!("Extracted {}/{} files, {} failed:", success, total, failure,);
    for failed in results.iter().filter(|j| j.error.is_some()) {
      eprintln!("   {}", failed.job.input.display().to_string());
    }
  }
  println!("Total time: {:.2}s", now.elapsed().as_secs_f32());
  Ok(())
}

/// Convert given raw file to dng file
fn generate_job(entry: &FileMap, options: &ArgMatches<'_>) -> Result<ExtractRawJob> {
  let mut in_file = BufReader::new(File::open(&entry.src)?);
  let file = TiffReader::new(&mut in_file, 0, None).map_err(|e| AppError::General(e.to_string()))?;

  if !file.has_entry(DngTag::DNGVersion) {
    return Err(AppError::General("Input file is not a DNG".into()));
  }

  let orig_filename = get_original_name(&file).ok_or(AppError::General("No embedded raw file found".into()))?;

  let output = PathBuf::from(&entry.dest).with_file_name(orig_filename);
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

  Ok(ExtractRawJob {
    input: PathBuf::from(&entry.src),
    output,
    replace: options.is_present("override"),
    skip_checks: options.is_present("skipchecks"),
  })
}

/// Check if file extension is a supported extension
fn is_ext_supported<T: AsRef<str>>(ext: T) -> bool {
  let uc = ext.as_ref().to_uppercase();
  SUPPORTED_FILE_EXT.iter().any(|ext| ext.eq(&uc))
}

/// Extract DNG OriginalRawFileName from TIFF structure
fn get_original_name(file: &TiffReader) -> Option<String> {
  if let Some(Entry {
    value: Value::Ascii(orig_name),
    ..
  }) = file.get_entry(DngTag::OriginalRawFileName)
  {
    Some(orig_name.strings()[0].clone())
  } else {
    None
  }
}
