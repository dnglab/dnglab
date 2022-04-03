// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use clap::ArgMatches;
use futures::future::join_all;
use std::fs::create_dir_all;
use std::path::PathBuf;
use std::time::Instant;

use crate::app::convert_bool;
use crate::filemap::{FileMap, MapMode};
use crate::jobs::raw2dng::{JobResult, Raw2DngJob};
use crate::jobs::Job;
use crate::{
  dnggen::{ConvertParams, DngCompression},
  AppError, Result, PKG_NAME, PKG_VERSION,
};

const SUPPORTED_FILE_EXT: [&str; 4] = ["CR3", "CR2", "CRM", "PEF"];

/// Entry point for Clap sub command `convert`
pub async fn convert(options: &ArgMatches<'_>) -> anyhow::Result<()> {
  let now = Instant::now();
  let in_path = PathBuf::from(options.value_of("INPUT").expect("INPUT not available"));
  let out_path = PathBuf::from(options.value_of("OUTPUT").expect("OUTPUT not available"));
  let recursive = options.is_present("recursive");

  let proc = MapMode::new(&in_path, &out_path)?;

  // List of jobs
  let mut jobs: Vec<Raw2DngJob> = Vec::new();

  // drop pathes to prevent use
  drop(in_path);
  drop(out_path);

  match proc {
    // We have only one input file, so output must be a file, too.
    MapMode::File(sd) => {
      jobs.append(&mut generate_job(&sd, options)?);
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
        jobs.append(&mut generate_job(&entry, options)?);
      }
    }
  }

  let verbose = options.is_present("verbose");

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
    println!("Converted {}/{} files", success, total,);
  } else {
    eprintln!("Converted {}/{} files, {} failed:", success, total, failure,);
    for failed in results.iter().filter(|j| j.error.is_some()) {
      eprintln!("   {}", failed.job.input.display());
    }
  }
  println!("Total time: {:.2}s", now.elapsed().as_secs_f32());
  Ok(())
}

/// Convert given raw file to dng file
fn generate_job(entry: &FileMap, options: &ArgMatches<'_>) -> Result<Vec<Raw2DngJob>> {
  let (do_batch, index) = match options.value_of("index") {
    Some(index) => {
      if index.to_lowercase().eq("all") {
        (true, 0)
      } else {
        (false, index.parse::<usize>().unwrap_or(0))
      }
    }
    None => (false, 0),
  };

  let batch_count = if do_batch { rawler::raw_image_count_file(&entry.src).unwrap() } else { 1 };

  let mut jobs = Vec::new();

  for i in 0..batch_count {
    // Params for conversion process
    let params = ConvertParams {
      predictor: options.value_of("predictor").unwrap_or("1").parse::<u8>().unwrap(),
      embedded: convert_bool(options.value_of("embedded"), true).unwrap(),
      crop: convert_bool(options.value_of("crop"), true).unwrap(),
      preview: convert_bool(options.value_of("preview"), true).unwrap(),
      thumbnail: convert_bool(options.value_of("thumbnail"), true).unwrap(),
      compression: match options.value_of("compression") {
        Some("lossless") => DngCompression::Lossless,
        Some("none") => DngCompression::Uncompressed,
        Some(s) => {
          println!("Unknown compression: {}", s);
          return Err(AppError::InvalidArgs);
        }
        None => DngCompression::Lossless,
      },
      artist: options.value_of("artist").map(String::from),
      software: format!("{} {}", PKG_NAME, PKG_VERSION),
      index: if do_batch { i } else { index },
    };

    let mut output = PathBuf::from(&entry.dest);
    if !output.set_extension("dng") {
      return Err(AppError::General("Unable to rename target to dng".into()));
    }

    if do_batch && batch_count > 1 {
      let file_name = String::from(output.file_stem().unwrap().to_string_lossy());
      output.set_file_name(format!("{}_{:04}.dng", file_name, i));
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
        return Err(AppError::General("Output path has no parent directory".to_string()));
      }
    }

    jobs.push(Raw2DngJob {
      input: PathBuf::from(&entry.src),
      output,
      replace: options.is_present("override"),
      params,
    });
  }
  Ok(jobs)
}

/// Check if file extension is a supported extension
fn is_ext_supported<T: AsRef<str>>(ext: T) -> bool {
  let uc = ext.as_ref().to_uppercase();
  SUPPORTED_FILE_EXT.iter().any(|ext| ext.eq(&uc))
}
