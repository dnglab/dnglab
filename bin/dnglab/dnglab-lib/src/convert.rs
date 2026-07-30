// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use clap::ArgMatches;
use futures::future::join_all;
use rawler::decoders::supported_extensions;
use std::collections::HashSet;
use std::fs::create_dir_all;
use std::path::PathBuf;

use std::time::Instant;

use crate::filemap::{FileMap, MapMode};
use crate::jobs::Job;
use crate::jobs::raw2dng::{JobResult, Raw2DngJob};
use crate::{AppError, PKG_VERSION, Result};
use rawler::dng::DngCompression;
use rawler::dng::convert::ConvertParams;

/// Entry point for Clap sub command `convert`
pub async fn convert(options: &ArgMatches) -> crate::Result<()> {
  let now = Instant::now();

  let recursive = options.get_flag("recursive");

  let proc = {
    let in_path: &PathBuf = options
      .get_one("INPUT")
      .ok_or_else(|| AppError::InvalidCmdSwitch("INPUT not available".into()))?;
    let out_path: &PathBuf = options
      .get_one("OUTPUT")
      .ok_or_else(|| AppError::InvalidCmdSwitch("OUTPUT not available".into()))?;
    MapMode::new(in_path, out_path)?
  };

  // List of jobs
  let mut jobs: Vec<Raw2DngJob> = Vec::new();
  // Output paths already assigned to a job in this run; used to disambiguate
  // distinct source files that would otherwise produce the same DNG name
  // (e.g. FOO.CR3 and FOO.NEF -> FOO.dng / FOO_1.dng).
  let mut claimed: HashSet<PathBuf> = HashSet::new();

  match proc {
    // We have only one input file, so output must be a file, too.
    MapMode::File(sd) => {
      jobs.append(&mut generate_job(&sd, options, &mut claimed)?);
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
        jobs.append(&mut generate_job(&entry, options, &mut claimed)?);
      }
    }
  }

  let verbose = options.get_flag("verbose");
  let concurrency = resolve_concurrency(options.get_one::<usize>("jobs").copied().unwrap_or(0));

  let mut results: Vec<JobResult> = Vec::new();
  for chunks in jobs.chunks(concurrency) {
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
    eprintln!("Converted {}/{} files", success, total,);
  } else {
    eprintln!("Converted {}/{} files, {} failed:", success, total, failure,);
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
fn generate_job(entry: &FileMap, options: &ArgMatches, claimed: &mut HashSet<PathBuf>) -> Result<Vec<Raw2DngJob>> {
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
  let multi_frame = do_batch && batch_count > 1;
  let replace = options.get_flag("override");

  let input = PathBuf::from(&entry.src);
  let mut output = PathBuf::from(&entry.dest);

  // If output is a directory, append the source file name.
  if input.is_file() && output.exists() && output.is_dir() {
    output.push(entry.src.file_name().ok_or_else(|| AppError::General("Input path has no file name".into()))?);
  }

  let has_dng_ext = if let Some(ext) = output.extension() {
    ext.eq_ignore_ascii_case("dng")
  } else {
    false
  };
  if !has_dng_ext && !output.set_extension("dng") {
    return Err(AppError::General("Unable to rename target to dng".into()));
  }

  let base_stem = output
    .file_stem()
    .ok_or_else(|| AppError::General("Output path has no file stem".into()))?
    .to_string_lossy()
    .into_owned();

  // Pick the lowest disambiguation suffix `k` such that every frame's output
  // path is free within this run's already-claimed outputs.
  // For multi-frame sources, all frame outputs must be checked together so
  // they share the same suffix.
  //
  //   single frame, k=0:    FOO.dng
  //   single frame, k>0:    FOO_<k>.dng
  //   multi  frame, k=0:    FOO_<i>.dng                      (i = 0..batch_count)
  //   multi  frame, k>0:    FOO_<k>_<i>.dng
  let resolve_frame = |k: usize, frame: usize| -> PathBuf {
    let stem = if multi_frame {
      if k == 0 {
        format!("{}_{:04}", base_stem, frame)
      } else {
        format!("{}_{}_{:04}", base_stem, k, frame)
      }
    } else if k == 0 {
      base_stem.clone()
    } else {
      format!("{}_{}", base_stem, k)
    };
    output.with_file_name(format!("{}.dng", stem))
  };

  let collides = |path: &PathBuf| -> bool {
    claimed.contains(path)
    // If existing files should take into account...
    // if claimed.contains(path) {
    //   return true;
    // }
    // !replace && path.exists()
  };

  let mut suffix = 0usize;
  let final_outputs = loop {
    let candidates: Vec<PathBuf> = (0..batch_count).map(|i| resolve_frame(suffix, i)).collect();
    if !candidates.iter().any(collides) {
      break candidates;
    }
    suffix += 1;
  };

  // Reserve these outputs so subsequent generate_job calls don't pick them.
  for p in &final_outputs {
    claimed.insert(p.clone());
  }

  // Ensure the parent directory exists once for the resolved outputs.
  if let Some(parent) = final_outputs[0].parent() {
    if !parent.exists() {
      create_dir_all(parent)?;
      if options.get_flag("verbose") {
        println!("Creating output directory '{}'", parent.display());
      }
    }
  } else {
    return Err(AppError::General("Output path has no parent directory".to_string()));
  }

  let mut jobs = Vec::with_capacity(batch_count);
  for (i, out) in final_outputs.into_iter().enumerate() {
    let params = ConvertParams {
      predictor: *options
        .get_one("predictor")
        .ok_or_else(|| AppError::InvalidCmdSwitch("predictor has no default".into()))?,
      embedded: options.get_flag("embedded"),
      photometric_conversion: Default::default(),
      crop: *options
        .get_one("crop")
        .ok_or_else(|| AppError::InvalidCmdSwitch("crop has no default".into()))?,
      preview: options.get_flag("preview"),
      thumbnail: options.get_flag("thumbnail"),
      compression: {
        let mut c: DngCompression = *options
          .get_one("compression")
          .ok_or_else(|| AppError::InvalidCmdSwitch("compression has no default".into()))?;
        if let DngCompression::JxlLossy { ref mut distance, ref mut effort, ref mut decode_speed } = c {
          if let Some(&d) = options.get_one::<f32>("jxl_distance") {
            *distance = d;
          }
          if let Some(&e) = options.get_one::<u32>("jxl_effort") {
            *effort = e;
          }
          *decode_speed = options.get_one::<u32>("jxl_decode_speed").copied();
        }
        c
      },
      artist: options.get_one("artist").cloned(),
      software: format!("{} {}", "DNGLab", PKG_VERSION),
      index: if do_batch { i } else { index },
      apply_scaling: false,
      keep_mtime: options.get_flag("keep_mtime"),
    };
    jobs.push(Raw2DngJob {
      input: input.clone(),
      output: out,
      replace,
      params,
    });
  }
  Ok(jobs)
}

/// Decide how many files to convert in parallel.
///
/// Each in-flight job runs LJPEG tile compression on rayon's global pool,
/// which fans the work across every logical CPU. Stacking many jobs at once
/// on top of that quickly causes oversubscription (`jobs × cpus` workers
/// contending), heavy context-switching, and cache thrash. We therefore cap
/// outer concurrency low so rayon gets meaningful per-job parallelism.
///
/// `requested` is the value of the `--jobs` flag. `0` means "auto":
///   - default to `max(1, available_parallelism / 4)`, capped at 4.
/// Any non-zero `requested` value is honored verbatim.
fn resolve_concurrency(requested: usize) -> usize {
  if requested > 0 {
    return requested;
  }
  let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
  (cpus / 4).clamp(1, 4)
}

/// Check if file extension is a supported extension
fn is_ext_supported<T: AsRef<str>>(ext: T) -> bool {
  let uc = ext.as_ref().to_uppercase();
  supported_extensions().iter().any(|ext| ext.eq(&uc))
}
