// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use anyhow::Context;
use clap::ArgMatches;
use log::debug;
use rayon::prelude::*;
use std::{fs, io::Write};
use std::{
  fs::{read_dir, File},
  path::{Path, PathBuf},
  time::Instant,
};

use crate::{AppError, PKG_NAME, PKG_VERSION, dnggen::{DngCompression, DngParams, raw_to_dng}};

const SUPPORTED_FILE_EXT: [&'static str; 3] = ["CR3", "CR2", "CRW"];

/// Entry point for Clap sub command `convert`
pub fn convert(options: &ArgMatches<'_>) -> anyhow::Result<()> {
  let in_path = PathBuf::from(options.value_of("INPUT").expect("INPUT not available"));
  let out_path = PathBuf::from(options.value_of("OUTPUT").expect("OUTPUT not available"));
  if !in_path.exists() {
    println!("INPUT path '{}' not exists", in_path.display());
    return Err(AppError::InvalidArgs.into());
  }
  let in_md = fs::metadata(&in_path).context("Unable to determine metadata for given input")?;
  if in_md.is_file() {
    // Convert a single file
    return convert_file(&in_path, &build_output_path(&in_path, &out_path)?, options);
  } else if in_md.is_dir() {
    // Convert whole directory
    return convert_dir(&in_path, &out_path, options);
  } else {
    println!("INPUT is not a file nor directory");
    return Err(AppError::InvalidArgs.into());
  }
}

/// Convert whole directory into DNG files
fn convert_dir(in_path: &Path, out_path: &Path, options: &ArgMatches<'_>) -> anyhow::Result<()> {
  let in_files: Vec<PathBuf> = read_dir(in_path)?
    .map(|entry| entry.unwrap().path())
    .filter(|entry| fs::metadata(entry).map(|md| md.is_file()).unwrap_or(true))
    .filter(|file| match file.extension() {
      Some(ext) => is_ext_supported(ext.to_string_lossy()),
      None => false,
    })
    .collect();

  let results: Vec<bool> = in_files
    .par_iter()
    .map(|in_path| {
      if let Ok(out_path) = build_output_path(in_path, out_path) {
        convert_file(&in_path, &out_path, options).is_ok()
      } else {
        false
      }
    })
    .collect();

  let success_count = results.iter().filter(|r| **r).count();
  let failed_count = results.iter().filter(|r| !**r).count();

  if failed_count == 0 {
    println!("Finished {}/{}", success_count, results.len());
  } else {
    println!("Finished {}/{}, {} failed", success_count, results.len(), failed_count);
  }

  Ok(())
}

/// Convert given raw file to dng file
fn convert_file(in_file: &Path, out_file: &Path, options: &ArgMatches<'_>) -> anyhow::Result<()> {
  debug!("Infile: {:?}, Outfile: {:?}", in_file, out_file);
  let now = Instant::now();

  if out_file.exists() && !options.is_present("override") {
    println!("File {} already exists and --override was not given", out_file.to_str().unwrap_or_default());
    return Err(AppError::InvalidArgs.into());
  }

  let orig_filename = String::from(in_file.file_name().unwrap().to_os_string().to_str().unwrap());

  let mut raw_file = File::open(in_file)?;
  let mut dng_file = File::create(out_file)?;

  // Params for conversion process
  let params = DngParams {
    no_embedded: options.is_present("noembedded"),
    //compression: crate::dnggen::DngCompression::Lossless,
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

  match raw_to_dng(&mut raw_file, &mut dng_file, &orig_filename, &params) {
    Ok(_) => {
      dng_file.flush()?;
      println!(
        "Converted: '{}' => '{}' (in {:.2}s)",
        shorten_path(in_file),
        shorten_path(out_file),
        now.elapsed().as_secs_f32()
      );
      Ok(())
    }
    Err(e) => {
      println!("Failed: '{}' => '{}'", shorten_path(in_file), shorten_path(out_file),);
      Err(e.into())
    }
  }
}

/// Make a short path
fn shorten_path(path: &Path) -> String {
  let os_str = path.as_os_str();
  if os_str.len() <= 30 {
    String::from(os_str.to_string_lossy())
  } else {
    let full = String::from(os_str.to_string_lossy());
    //let a = &full[..full.len()-8];
    //let b = &full[full.len()-8..];
    //format!("{}...{}", a, b)
    full
  }
}

/// Build an output path for a given input path
fn build_output_path(in_path: &Path, out_path: &Path) -> anyhow::Result<PathBuf> {
  if out_path.exists() {
    let out_md = fs::metadata(out_path).context("Unable to determine metadata for given output")?;
    if out_md.is_file() {
      return Ok(PathBuf::from(out_path));
    } else if out_md.is_dir() {
      let new_filename = in_path.with_extension("DNG").file_name().unwrap().to_str().unwrap().to_string();
      let mut tmp = PathBuf::from(out_path);
      tmp.push(new_filename);
      return Ok(tmp);
    } else {
      return Err(AppError::InvalidArgs.into());
    }
  } else {
    match out_path.parent() {
      Some(parent) => {
        let out_md = fs::metadata(parent).context("Unable to determine metadata for given output")?;
        if out_md.is_dir() {
          // Ok, parent exists an is directory
          return Ok(PathBuf::from(out_path));
        } else {
          println!("Output or parent directory not exists");
          return Err(AppError::InvalidArgs.into());
        }
      }
      None => {
        println!("Output or parent directory not exists");
        return Err(AppError::InvalidArgs.into());
      }
    }
  }
}

/// Check if file extension is a supported extension
fn is_ext_supported<T: AsRef<str>>(ext: T) -> bool {
  let uc = ext.as_ref().to_uppercase();
  SUPPORTED_FILE_EXT.iter().any(|ext| ext.eq(&uc))
}
