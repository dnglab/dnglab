// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::Job;
use crate::{AppError, Result};
use async_trait::async_trait;
use log::debug;
use rawler::{
  dng::original::{OriginalCompressed, OriginalDigest},
  formats::tiff::{reader::TiffReader, GenericTiffReader, Value},
  tags::DngTag,
};
use std::{
  fmt::Display,
  io::{BufReader, BufWriter, Cursor, Write},
};
use std::{fs::File, path::PathBuf, time::Instant};

/// Job for converting RAW to DNG
#[derive(Debug, Clone)]
pub struct ExtractRawJob {
  pub input: PathBuf,
  pub output: PathBuf,
  pub replace: bool,
  pub skip_checks: bool,
}

/// State of conversion
#[derive(Debug)]
pub struct JobResult {
  pub job: ExtractRawJob,
  pub duration: f32,
  pub error: Option<AppError>,
}

impl Display for JobResult {
  /// Pretty print the extraction state
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    if let Some(error) = self.error.as_ref() {
      f.write_fmt(format_args!("Failed: '{}', {}", self.job.input.display(), error))?;
    } else {
      f.write_fmt(format_args!(
        "Extracted '{}' => '{}' (in {:.2}s)",
        self.job.input.display(),
        self.job.output.display(),
        self.duration
      ))?;
    }
    Ok(())
  }
}

impl ExtractRawJob {
  fn internal_exec(&self) -> Result<JobResult> {
    if self.output.exists() && !self.replace {
      return Err(AppError::AlreadyExists(self.output.clone()));
    }
    let dng_file = File::open(&self.input)?;

    let mut in_file = BufReader::new(dng_file);
    let file = GenericTiffReader::new(&mut in_file, 0, 0, None, &[]).map_err(|e| AppError::General(e.to_string()))?;

    if !file.has_entry(DngTag::DNGVersion) {
      debug!("Input is not a DNG file");
      return Err(AppError::General("Input file is not a DNG".into()));
    }
    if let Some(orig_data) = file.get_entry(DngTag::OriginalRawFileData) {
      if let Value::Undefined(val) = &orig_data.value {
        let digest = file
          .get_entry(DngTag::OriginalRawFileDigest)
          .map(|entry| entry.value.get_data().as_slice())
          .and_then(|data| OriginalDigest::try_from(data).ok());

        let original = OriginalCompressed::new(&mut Cursor::new(val), digest)?;
        let mut stream = BufWriter::new(File::create(&self.output)?);
        original.decompress(&mut stream, !self.skip_checks)?;
        stream.flush()?;
        Ok(JobResult {
          job: self.clone(),
          duration: 0.0, // TODO: fixme
          error: None,
        })
      } else {
        Err(AppError::General("No embedded raw data found".into()))
      }
    } else {
      Err(AppError::General("No embedded raw file found".into()))
    }
  }
}

#[async_trait]
impl Job for ExtractRawJob {
  type Output = JobResult;

  async fn execute(&self) -> Self::Output {
    debug!("Job running: input: {:?}, output: {:?}", self.input, self.output);
    let now = Instant::now();
    match self.internal_exec() {
      Ok(mut stat) => {
        stat.duration = now.elapsed().as_secs_f32();
        stat
      }
      Err(e) => JobResult {
        job: self.clone(),
        duration: now.elapsed().as_secs_f32(),
        error: Some(e),
      },
    }
  }
}
