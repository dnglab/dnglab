// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::Job;
use crate::{AppError, Result};
use async_trait::async_trait;
use log::debug;
use rawler::{dng::original::extract_original, rawsource::RawSource};
use std::{
  fmt::Display,
  fs::remove_file,
  io::{BufWriter, Write},
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
    let dng = RawSource::new(&self.input)?;
    let mut target = BufWriter::new(File::create(&self.output)?);
    if let Err(err) = extract_original(&dng, &mut target, !self.skip_checks) {
      drop(target);
      if let Err(err) = remove_file(&self.output) {
        log::error!("Failed to delete original file after decompress error: {:?}", err);
      }
      return Err(err.into());
    }
    target.flush()?;
    drop(target);
    Ok(JobResult {
      job: self.clone(),
      duration: 0.0, // Overwritten with the measured elapsed time in `execute`.
      error: None,
    })
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
