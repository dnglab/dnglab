// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::Job;
use crate::{
  dnggen::{raw_to_dng, DngParams},
  AppError, Result,
};
use log::debug;
use std::{fmt::Display, io::Write};
use std::{fs::File, path::PathBuf, time::Instant};

/// Job for converting RAW to DNG
#[derive(Debug, Clone)]
pub struct Raw2DngJob {
  pub input: PathBuf,
  pub output: PathBuf,
  pub replace: bool,
  pub params: DngParams,
}

/// State of conversion 
#[derive(Debug)]
pub struct JobResult {
  pub job: Raw2DngJob,
  pub duration: f32,
  pub error: Option<AppError>,
}

impl Display for JobResult {
    /// Pretty print the conversion state
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    if let Some(error) = self.error.as_ref() {
      f.write_fmt(format_args!("Failed: '{}', {}", self.job.input.display(), error.to_string()))?;
    } else {
      f.write_fmt(format_args!(
        "Converted '{}' => '{}' (in {:.2}s)",
        self.job.input.display(),
        self.job.output.display(),
        self.duration
      ))?;
    }
    Ok(())
  }
}

impl Raw2DngJob {
  fn internal_exec(&self) -> Result<JobResult> {
    if self.output.exists() && !self.replace {
      return Err(AppError::DestExists(self.output.display().to_string()));
    }
    // File name for embedding
    let orig_filename = self
      .input
      .file_name()
      .ok_or(AppError::General("Input has no filename".into()))?
      .to_os_string()
      .to_string_lossy()
      .to_string();

    let mut raw_file = File::open(&self.input)?;
    let mut dng_file = File::create(&self.output)?;

    match raw_to_dng(&mut raw_file, &mut dng_file, &orig_filename, &self.params) {
      Ok(_) => {
        dng_file.flush()?;
        Ok(JobResult {
          job: self.clone(),
          duration: 0.0,
          error: None,
        })
      }
      Err(e) => Err(AppError::General(e.to_string())),
    }
  }
}

impl Job for Raw2DngJob {
  type Output = JobResult;

  fn execute(&self) -> Self::Output {
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
