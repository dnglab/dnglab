// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use crate::{
  filemap::{FileMap, MapMode},
  AppError, Result,
};
use clap::ArgMatches;
use log::debug;
use rawler::{
  dng::{original_decompress, original_digest},
  tags::DngTag,
  tiff::{Entry, TiffReader, Value},
};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::{
  fs::{create_dir_all, File},
  io::{BufReader, BufWriter, Write},
  ops::Deref,
  path::{Path, PathBuf},
  sync::{Arc, Mutex},
};

/// Extract original RAW from DNG
pub fn extract(options: &ArgMatches<'_>) -> anyhow::Result<()> {
  let in_path = PathBuf::from(options.value_of("INPUT").expect("INPUT not available"));
  let out_path = PathBuf::from(options.value_of("OUTPUT").expect("OUTPUT not available"));

  let proc = MapMode::new(&in_path, &out_path)?;

  // drop pathes to prevent use
  drop(in_path);
  drop(out_path);

  match proc {
    // We have only one input file, so output must be a file, too.
    MapMode::File(sd) => {
      let mut in_file = BufReader::new(File::open(&sd.src)?);
      let file = TiffReader::new(&mut in_file, 0, None)?;

      match extract_single_dng_file(&file, &sd.dest, options) {
        Ok(_) => {
          println!("Extracted '{}' => '{}'", sd.src.display(), sd.dest.display());
          Ok(())
        }
        Err(e) => {
          eprintln!("Failed to extract raw from '{}', error: {}", sd.src.display(), e);
          Err(e.into())
        }
      }
    }
    // Input is directory, to process all files
    MapMode::Dir(sd) => {
      let list = sd.file_list(true, |file| {
        file.extension().map(|ext| ext.to_string_lossy().to_ascii_lowercase() == "dng").unwrap_or(false)
      })?;
      let failed_files = Arc::new(Mutex::new(Vec::new()));

      let failed_files_clone = failed_files.clone();
      list.par_iter().for_each(move |entry| match process_list_entry(entry, options) {
        Ok(out_path) => {
          println!("Extracted '{}' => '{}'", entry.src.display(), out_path.display());
        }
        Err(e) => {
          let mut fails = failed_files_clone.lock().expect("Failed to get lock");
          fails.push(entry.src.display().to_string());
          eprintln!("Failed to extract raw from '{}', error: {}", entry.src.display(), e);
        }
      });

      let failed_files = failed_files.lock().expect("Failed to get lock");
      if failed_files.is_empty() {
        println!("Extracted {} files", list.len());
      } else {
        eprintln!(
          "Extracted {}/{} files, {} failed:",
          list.len() - failed_files.len(),
          list.len(),
          failed_files.len()
        );
        for failed in failed_files.deref() {
          eprintln!("   {}", failed);
        }
      }
      Ok(())
    }
  }
}

fn process_list_entry(entry: &FileMap, options: &ArgMatches<'_>) -> Result<PathBuf> {
  let mut in_file = BufReader::new(File::open(&entry.src)?);
  let file = TiffReader::new(&mut in_file, 0, None).map_err(|e| AppError::General(e.to_string()))?;
  match get_original_name(&file) {
    Some(orig_name) => {
      let out_path = entry.dest.with_file_name(orig_name);
      create_dir_all(out_path.parent().expect("Output path must have a parent"))?;
      extract_single_dng_file(&file, &out_path, options)?;
      Ok(out_path)
    }
    None => {
      eprintln!("Skipping file '{}', no embedded raw file", entry.src.display());
      Err(AppError::General("No embedded raw file found".into()))
    }
  }
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

/// Extract a single DNG files
pub fn extract_single_dng_file(file: &TiffReader, out_path: &Path, options: &ArgMatches<'_>) -> Result<()> {
  if !file.has_entry(DngTag::DNGVersion) {
    eprintln!("Input is not a DNG file");
    return Err(AppError::General("Input file is not a DNG".into()));
  }
  if let Some(orig_data) = file.get_entry(DngTag::OriginalRawFileData) {
    if let Value::Undefined(val) = &orig_data.value {
      debug!("Outpath: {:?}", out_path);
      if out_path.exists() && !options.is_present("override") {
        return Err(AppError::DestExists(out_path.display().to_string()));
      }
      let comp = original_decompress(val)?;
      if let Some(Entry {
        value: Value::Byte(orig_digest),
        ..
      }) = file.get_entry(DngTag::OriginalRawFileDigest)
      {
        let new_digest = original_digest(&val);
        debug!("Original calculated original data digest: {:x?}", orig_digest);
        debug!("Fresh calculated original data digest: {:x?}", new_digest);
        if !orig_digest.eq(&new_digest) {
          if options.is_present("skipchecks") {
            eprintln!("Warning: digest verifiation for embedded data failed, output file may be corrupt!");
          } else {
            return Err(AppError::General("Embedded digest mismatch".into()));
          }
        }
      } else {
        return Err(AppError::General("No embedded raw digest found".into()));
      }

      let mut out_file = BufWriter::new(File::create(out_path)?);
      out_file.write(&comp)?;
      out_file.flush()?;
    }
  } else {
    return Err(AppError::General("No embedded raw file found".into()));
  }

  Ok(())
}
