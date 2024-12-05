// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::{
  fs::{self, read_dir},
  path::{Path, PathBuf},
};

use log::warn;

use crate::{AppError, Result};

#[derive(Debug, Clone)]
pub struct FileMap {
  pub src: PathBuf,
  pub dest: PathBuf,
}

#[derive(Debug, Clone)]
pub struct DirMap {
  pub src: PathBuf,
  pub dest: PathBuf,
}

/// Process mode
#[derive(Debug, Clone)]
pub enum MapMode {
  File(FileMap),
  Dir(DirMap),
}

impl FileMap {
  pub fn new(src: &Path, dest: &Path) -> Self {
    assert!(src.is_absolute());
    Self {
      src: PathBuf::from(src),
      dest: PathBuf::from(dest),
    }
  }
}

impl DirMap {
  /// Construct new DirMap instance from src and dest
  pub fn new(src: &Path, dest: &Path) -> Self {
    assert!(src.is_absolute());
    assert!(dest.is_absolute());
    assert!(src.is_dir());
    assert!(dest.is_dir());
    Self {
      src: PathBuf::from(src),
      dest: PathBuf::from(dest),
    }
  }

  /// Get file list of source/destination mapped files
  pub fn file_list<F>(&self, recursive: bool, filter: F) -> Result<Vec<FileMap>>
  where
    F: Fn(&Path) -> bool + Copy,
  {
    let mut result = Vec::new();
    let entries = read_filtered_dir(&self.src, recursive, filter)?;
    for entry in entries {
      let map = self.make_mapping(&entry)?;
      result.push(map);
    }
    Ok(result)
  }

  /// Map `input` path to output, add sub directories if required
  fn make_mapping(&self, input: &Path) -> Result<FileMap> {
    let sub_location = input.strip_prefix(&self.src).expect("Input path must be located inside source");
    let dest = self.dest.join(sub_location);
    Ok(FileMap {
      src: PathBuf::from(input),
      dest,
    })
  }
}

impl MapMode {
  /// Construct new ProcessMode from given input and output
  pub fn new(input: &Path, output: &Path) -> Result<MapMode> {
    if !input.exists() {
      return Err(AppError::NotFound(input.to_owned()));
    }
    let input_md = input.metadata()?;

    if input_md.is_file() {
      Ok(MapMode::File(FileMap::new(&input.canonicalize()?, output)))
    } else if input_md.is_dir() {
      if !output.exists() {
        return Err(AppError::NotFound(output.to_owned()));
      }
      let output_md = output.metadata()?;
      if !output_md.is_dir() {
        return Err(AppError::InvalidCmdSwitch(format!(
          "Output '{}' must be a directory, because input is a directory",
          output.display()
        )));
      }
      Ok(MapMode::Dir(DirMap::new(&input.canonicalize()?, &output.canonicalize()?)))
    } else {
      return Err(AppError::General(format!("Unable to determine type of {}", input.display())));
    }
  }
}

/// Read directory (optionally recursive) and filter entries
fn read_filtered_dir<F>(input: &Path, recursive: bool, filter: F) -> Result<Vec<PathBuf>>
where
  F: Fn(&Path) -> bool + Copy,
{
  let mut result = Vec::new();
  let dir = read_dir(input)?;
  for entry in dir {
    let entry = entry?;
    let in_md = fs::metadata(entry.path())?;
    if in_md.is_file() {
      if filter(&entry.path()) {
        result.push(entry.path());
      }
    } else if in_md.is_dir() {
      if recursive {
        result.extend(read_filtered_dir(&entry.path(), recursive, filter)?);
      }
    } else {
      // If we hit sockets, device files etc. just warn and ignore them.
      warn!("Unable to determine type of {}", entry.path().display());
    }
  }
  Ok(result)
}
