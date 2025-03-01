// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use clap::ArgMatches;
use futures::future::join_all;
use rawler::formats::tiff::reader::TiffReader;
use rawler::formats::tiff::{Entry, GenericTiffReader, Value};
use rawler::tags::DngTag;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::time::Instant;

use crate::filemap::{FileMap, MapMode};
use crate::jobs::Job;
use crate::jobs::extractraw::{ExtractRawJob, JobResult};
use crate::{AppError, Result};

const SUPPORTED_FILE_EXT: [&str; 1] = ["DNG"];

/// Entry point for Clap sub command `extract`
pub async fn extract(options: &ArgMatches) -> crate::Result<()> {
  let now = Instant::now();

  let proc = {
    let in_path: &PathBuf = options.get_one("INPUT").expect("INPUT not available");
    let out_path: &PathBuf = options.get_one("OUTPUT").expect("OUTPUT not available");
    MapMode::new(in_path, out_path)?
  };

  let recursive = options.get_flag("recursive");

  let mut jobs = Vec::new();

  match proc {
    // We have only one input file, so output must be a file, too.
    MapMode::File(sd) => {
      let job = generate_job(&sd, options, false)?;
      jobs.push(job);
    }
    // Input is directory, to process all files
    MapMode::Dir(sd) => {
      eprintln!("Scanning directory, please wait...");
      let list = sd.file_list(recursive, |file| {
        if let Some(ext) = file.extension().map(|ext| ext.to_string_lossy()) {
          is_ext_supported(ext)
        } else {
          false
        }
      })?;

      let tmp: Result<Vec<ExtractRawJob>> = list.par_iter().map(|entry| generate_job(entry, options, true)).collect();
      jobs.extend(tmp?.into_iter());
    }
  }

  let verbose = options.get_flag("verbose");

  let mut results: Vec<JobResult> = Vec::new();
  for chunks in jobs.chunks(8) {
    let mut temp: Vec<JobResult> = join_all(chunks.iter().map(|j| j.execute()))
      .await
      .into_iter()
      .map(|res| {
        if verbose {
          println!("Status: {}", res);
        }
        res
      })
      .collect();
    results.append(&mut temp);
  }

  let total = results.len();
  let success = results.iter().filter(|j| j.error.is_none()).count();
  let failure = results.iter().filter(|j| j.error.is_some()).count();

  if failure == 0 {
    println!("Extracted {}/{} files", success, total,);
  } else {
    eprintln!("Extracted {}/{} files, {} failed:", success, total, failure,);
    for failed in results.iter().filter(|j| j.error.is_some()) {
      eprintln!("   {}", failed.job.input.display());
    }
  }
  println!("Total time: {:.2}s", now.elapsed().as_secs_f32());
  Ok(())
}

/// Convert given raw file to dng file
fn generate_job(entry: &FileMap, options: &ArgMatches, use_orig_filename: bool) -> Result<ExtractRawJob> {
  let mut in_file = BufReader::new(File::open(&entry.src)?);
  let file = GenericTiffReader::new(&mut in_file, 0, 0, None, &[]).map_err(|e| AppError::General(e.to_string()))?;

  if !file.has_entry(DngTag::DNGVersion) {
    return Err(AppError::General("Input file is not a DNG".into()));
  }

  let orig_filename = get_original_name(&file).ok_or(AppError::General("No embedded raw file found".into()))?;

  let output = if entry.dest.is_dir() {
    let mut p = PathBuf::from(&entry.dest);
    p.push(orig_filename);
    p
  } else if use_orig_filename {
    PathBuf::from(&entry.dest).with_file_name(orig_filename)
  } else {
    PathBuf::from(&entry.dest)
  };

  Ok(ExtractRawJob {
    input: PathBuf::from(&entry.src),
    output,
    replace: options.get_flag("override"),
    skip_checks: options.get_flag("skipchecks"),
  })
}

/// Check if file extension is a supported extension
fn is_ext_supported<T: AsRef<str>>(ext: T) -> bool {
  let uc = ext.as_ref().to_uppercase();
  SUPPORTED_FILE_EXT.iter().any(|ext| ext.eq(&uc))
}

/// Extract DNG OriginalRawFileName from TIFF structure
fn get_original_name(file: &GenericTiffReader) -> Option<String> {
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
