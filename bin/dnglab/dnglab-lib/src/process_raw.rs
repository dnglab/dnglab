// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

use clap::ArgMatches;
use futures::future::join_all;
use rawler::decoders::supported_extensions;
use rawler::{
  dng::opcodes::OpcodeSelector,
  imgop::develop::{ProcessedImageFormat, RawProcessingParams},
};
use std::fs::create_dir_all;
use std::path::{Path, PathBuf};

use std::time::Instant;

use crate::filemap::{FileMap, MapMode};
use crate::jobs::Job;
use crate::jobs::process_raw::{JobResult, Raw2Image};
use crate::{AppError, PKG_VERSION, Result};

/// Entry point for Clap sub command `process-raw`
pub async fn process_raw(options: &ArgMatches) -> crate::Result<()> {
  let now = Instant::now();

  let recursive = options.get_flag("recursive");

  let proc = {
    let in_path: &PathBuf = options
      .get_one("INPUT")
      .ok_or_else(|| crate::AppError::InvalidCmdSwitch("INPUT not available".into()))?;
    let out_path: &PathBuf = options
      .get_one("OUTPUT")
      .ok_or_else(|| crate::AppError::InvalidCmdSwitch("OUTPUT not available".into()))?;
    MapMode::new(in_path, out_path)?
  };

  // List of jobs
  let mut jobs: Vec<Raw2Image> = Vec::new();

  match proc {
    // We have only one input file, so output must be a file, too.
    MapMode::File(sd) => {
      jobs.append(&mut generate_job(&sd, options)?);
    }
    // Input is directory, to process all files
    MapMode::Dir(sd) => {
      let list = sd.file_list(recursive, |file| {
        if let Some(ext) = file.extension().map(|ext| ext.to_string_lossy()) {
          is_ext_supported(ext)
        } else {
          false
        }
      })?;
      for entry in list {
        jobs.append(&mut generate_job(&entry, options)?);
      }
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
    eprintln!("Processed {}/{} files", success, total,);
  } else {
    eprintln!("Processed {}/{} files, {} failed:", success, total, failure,);
    for failed in results.iter().filter(|j| j.error.is_some()) {
      eprintln!("   {}", failed.job.input.display());
    }
  }
  eprintln!("Total time: {:.2}s", now.elapsed().as_secs_f32());

  let first_error = results.into_iter().filter(|j| j.error.is_some()).map(|j| j.error).next();
  if let Some(Some(err)) = first_error {
    // In case of errors, return the first error in the queue
    Err(err)
  } else {
    Ok(())
  }
}

/// Convert given raw file to dng file
fn generate_job(entry: &FileMap, options: &ArgMatches) -> Result<Vec<Raw2Image>> {
  let (do_batch, index) = match options.get_one::<String>("index") {
    Some(index) => {
      if index.to_lowercase().eq("all") {
        (true, 0)
      } else {
        (false, index.parse::<usize>().unwrap_or(0))
      }
    }
    None => (false, 0),
  };

  let batch_count = if do_batch { rawler::raw_image_count_file(&entry.src)? } else { 1 };

  let mut jobs = Vec::new();

  for i in 0..batch_count {
    let input = PathBuf::from(&entry.src);
    let mut output = PathBuf::from(&entry.dest);

    // If output is a directry
    if input.is_file() && output.exists() && output.is_dir() {
      output.push(entry.src.file_name().ok_or_else(|| anyhow::anyhow!("source path has no file name"))?);
    }

    let format = output_format(options.get_one("format").copied(), &output);
    set_output_extension(&mut output, format);

    if do_batch && batch_count > 1 {
      let file_name = String::from(
        output
          .file_stem()
          .ok_or_else(|| anyhow::anyhow!("output path has no file stem"))?
          .to_string_lossy(),
      );
      let extension = output.extension().map(|ext| ext.to_string_lossy().into_owned()).unwrap_or_default();
      output.set_file_name(format!("{}_{:04}.{}", file_name, i, extension));
    }

    // Params for conversion process
    let params = RawProcessingParams {
      crop: *options
        .get_one("crop")
        .ok_or_else(|| crate::AppError::InvalidCmdSwitch("crop has no default".into()))?,
      thumbnail: true, //options.get_flag("thumbnail"),
      artist: options.get_one("artist").cloned(),
      software: format!("{} {}", "DNGLab", PKG_VERSION),
      index: if do_batch { i } else { index },
      keep_mtime: options.get_flag("keep_mtime"),
      remove_dng_opcodes: options
        .get_many::<OpcodeSelector>("remove_dng_opcodes")
        .map(|values| values.cloned().collect())
        .unwrap_or_default(),
      format,
      jpeg_quality: *options
        .get_one("jpeg_quality")
        .ok_or_else(|| crate::AppError::InvalidCmdSwitch("jpeg-quality has no default".into()))?,
    };

    match output.parent() {
      Some(parent) => {
        if !parent.exists() {
          create_dir_all(parent)?;
          if options.get_flag("verbose") {
            println!("Creating output directory '{}'", parent.display());
          }
        }
      }
      None => {
        return Err(AppError::General("Output path has no parent directory".to_string()));
      }
    }

    jobs.push(Raw2Image {
      input,
      output,
      replace: options.get_flag("override"),
      params,
    });
  }
  Ok(jobs)
}

/// Output format given by `format`, otherwise by the extension of `output`
fn output_format(format: Option<ProcessedImageFormat>, output: &Path) -> ProcessedImageFormat {
  format
    .or_else(|| output.extension().and_then(ProcessedImageFormat::from_extension))
    .unwrap_or_default()
}

/// Keep the extension of `output` if it belongs to `format`, otherwise use the default one
fn set_output_extension(output: &mut PathBuf, format: ProcessedImageFormat) {
  if output.extension().and_then(ProcessedImageFormat::from_extension) != Some(format) {
    output.set_extension(format.extension());
  }
}

/// Check if file extension is a supported extension
fn is_ext_supported<T: AsRef<str>>(ext: T) -> bool {
  let uc = ext.as_ref().to_uppercase();
  supported_extensions().iter().any(|ext| ext.eq(&uc))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn format_from_flag_or_extension() {
    for (flag, path, expected) in [
      (None, "out.jpg", ProcessedImageFormat::Jpeg),
      (None, "out.JPEG", ProcessedImageFormat::Jpeg),
      (None, "out.tiff", ProcessedImageFormat::Tiff),
      (None, "out.TIF", ProcessedImageFormat::Tiff),
      (None, "dir/IMG_0001.CR2", ProcessedImageFormat::Tiff),
      (None, "out", ProcessedImageFormat::Tiff),
      (Some(ProcessedImageFormat::Jpeg), "dir/IMG_0001.CR2", ProcessedImageFormat::Jpeg),
      (Some(ProcessedImageFormat::Tiff), "out.jpg", ProcessedImageFormat::Tiff),
    ] {
      assert_eq!(output_format(flag, Path::new(path)), expected, "{:?} {}", flag, path);
    }
  }

  #[test]
  fn output_extension_matches_format() {
    for (path, format, expected) in [
      ("out.jpeg", ProcessedImageFormat::Jpeg, "out.jpeg"),
      ("out.JPG", ProcessedImageFormat::Jpeg, "out.JPG"),
      ("out.tiff", ProcessedImageFormat::Tiff, "out.tiff"),
      ("out.jpg", ProcessedImageFormat::Tiff, "out.tif"),
      ("dir/IMG_0001.CR2", ProcessedImageFormat::Jpeg, "dir/IMG_0001.jpg"),
      ("dir/IMG_0001.CR2", ProcessedImageFormat::Tiff, "dir/IMG_0001.tif"),
    ] {
      let mut output = PathBuf::from(path);
      set_output_extension(&mut output, format);
      assert_eq!(output, PathBuf::from(expected));
    }
  }
}
