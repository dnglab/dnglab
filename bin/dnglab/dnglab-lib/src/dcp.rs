// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! DNG camera profile files (DCP) for the commands

use std::collections::HashSet;
use std::fs::create_dir_all;
use std::path::{Path, PathBuf};

use clap::ArgMatches;
use rawler::RawlerError;
use rawler::decoders::RawDecodeParams;
use rawler::dng::dcp::CameraProfileFile;
use rawler::rawimage::RawImage;
use rawler::rawsource::RawSource;

use crate::{AppError, Result};

/// Error of a camera profile operation, without the "Decoder failed" prefix of [`AppError::DecoderFailed`]
fn profile_error(err: RawlerError) -> AppError {
  AppError::General(err.to_string())
}

/// Read the DCP file `path`
pub fn load_dcp(path: &Path) -> Result<CameraProfileFile> {
  if !path.exists() {
    return Err(AppError::NotFound(path.to_path_buf()));
  }
  CameraProfileFile::open(path).map_err(|err| AppError::General(format!("{}: {}", path.display(), profile_error(err))))
}

/// Read the DCP file of option `id`
pub fn dcp_option(options: &ArgMatches, id: &str) -> Result<Option<CameraProfileFile>> {
  options.get_one::<PathBuf>(id).map(|path| load_dcp(path)).transpose()
}

/// Read the DCP files of option `id`, which can be repeated
pub fn dcp_options(options: &ArgMatches, id: &str) -> Result<Vec<CameraProfileFile>> {
  options.get_many::<PathBuf>(id).into_iter().flatten().map(|path| load_dcp(path)).collect()
}

/// The ProfileEmbedPolicy of `dcp` must allow to embed it in a DNG file
pub fn check_embedding(dcp: &CameraProfileFile) -> Result<()> {
  dcp.check_embedding().map_err(profile_error)
}

/// Decode `input` without pixels, for its camera profiles
fn decode_camera_profiles(input: &Path) -> Result<RawImage> {
  let rawfile = RawSource::new(input)?;
  let decoder = rawler::get_decoder(&rawfile)?;
  Ok(decoder.raw_image(&rawfile, &RawDecodeParams::default(), true)?)
}

/// Write `dcp` to `path`, an existing file is only replaced if `replace` is set
fn write_dcp(dcp: &CameraProfileFile, path: &Path, replace: bool) -> Result<()> {
  if path.exists() && !replace {
    return Err(AppError::AlreadyExists(path.to_path_buf()));
  }
  let data = dcp.to_bytes().map_err(profile_error)?;
  std::fs::write(path, data)?;
  Ok(())
}

/// Write the as-shot camera profile of `input` to the DCP file `output`
pub fn export_dcp(input: &Path, output: &Path, replace: bool) -> Result<()> {
  let rawimage = decode_camera_profiles(input)?;
  let index = rawimage
    .as_shot_camera_profile_index()
    .ok_or_else(|| AppError::General(format!("{} has no camera profile", input.display())))?;
  let dcp = CameraProfileFile::from_raw_image(&rawimage, index).map_err(profile_error)?;
  dcp.check_copying().map_err(profile_error)?;
  write_dcp(&dcp, output, replace)
}

/// Write all camera profiles of `input` as DCP files to the directory `dir`,
/// returns the paths of the written files
///
/// Profiles that don't allow copying are skipped.
pub fn export_dcp_all(input: &Path, dir: &Path, replace: bool) -> Result<Vec<PathBuf>> {
  let rawimage = decode_camera_profiles(input)?;
  if rawimage.camera_profiles.is_empty() {
    return Err(AppError::General(format!("{} has no camera profile", input.display())));
  }
  create_dir_all(dir)?;
  let mut taken = HashSet::new();
  let mut written = Vec::new();
  for index in 0..rawimage.camera_profiles.len() {
    let dcp = CameraProfileFile::from_raw_image(&rawimage, index).map_err(profile_error)?;
    if let Err(err) = dcp.check_copying() {
      log::warn!("Camera profile {} is not exported: {}", index, profile_error(err));
      continue;
    }
    let path = dir.join(unique_file_name(&dcp.file_name(), &mut taken));
    write_dcp(&dcp, &path, replace)?;
    written.push(path);
  }
  if written.is_empty() {
    return Err(AppError::General(format!("No camera profile of {} can be exported", input.display())));
  }
  Ok(written)
}

/// `name` or, if it is taken, "<stem> (2).dcp", "<stem> (3).dcp", ... Names
/// are compared case-insensitive, like file systems of Windows and macOS do.
fn unique_file_name(name: &str, taken: &mut HashSet<String>) -> String {
  let stem = name.strip_suffix(".dcp").unwrap_or(name);
  let mut candidate = name.to_string();
  let mut i = 2;
  while !taken.insert(candidate.to_lowercase()) {
    candidate = format!("{} ({}).dcp", stem, i);
    i += 1;
  }
  candidate
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::app::create_app;
  use rawler::camera_profile::CameraProfile;

  /// Arguments of the subcommand of `args`
  fn subcommand(args: &[&str]) -> ArgMatches {
    let mut argv = vec!["dnglab"];
    argv.extend_from_slice(args);
    let matches = create_app().try_get_matches_from(argv).expect("valid arguments");
    matches.subcommand().expect("subcommand").1.clone()
  }

  fn paths(matches: &ArgMatches, id: &str) -> Vec<PathBuf> {
    matches.get_many::<PathBuf>(id).into_iter().flatten().cloned().collect()
  }

  #[test]
  fn dcp_arguments() {
    let convert = subcommand(&[
      "convert",
      "--dcp-file",
      "a.dcp",
      "--extra-dcp-file",
      "b.dcp",
      "--extra-dcp-file",
      "c.dcp",
      "in.CR2",
      "out.dng",
    ]);
    assert_eq!(paths(&convert, "dcp_file"), [PathBuf::from("a.dcp")]);
    assert_eq!(paths(&convert, "extra_dcp_file"), [PathBuf::from("b.dcp"), PathBuf::from("c.dcp")]);

    let process = subcommand(&["process-raw", "--dcp-file", "a.dcp", "in.CR2", "out.tif"]);
    assert_eq!(paths(&process, "dcp_file"), [PathBuf::from("a.dcp")]);

    let analyze = subcommand(&["analyze", "--export-dcp", "a.dcp", "--export-dcp-all", "dir", "-f", "in.CR2"]);
    assert_eq!(paths(&analyze, "export_dcp"), [PathBuf::from("a.dcp")]);
    assert_eq!(paths(&analyze, "export_dcp_all"), [PathBuf::from("dir")]);
    assert!(analyze.get_flag("override"));
  }

  #[test]
  fn load_dcp_files() -> Result<()> {
    let convert = subcommand(&["convert", "in.CR2", "out.dng"]);
    assert!(dcp_option(&convert, "dcp_file")?.is_none());
    assert!(dcp_options(&convert, "extra_dcp_file")?.is_empty());

    let path = std::env::temp_dir().join(format!("dnglab-dcp-test-{}.dcp", std::process::id()));
    let missing = path.to_string_lossy().to_string();
    let convert = subcommand(&["convert", "--dcp-file", &missing, "in.CR2", "out.dng"]);
    assert!(matches!(dcp_option(&convert, "dcp_file"), Err(AppError::NotFound(_))));

    let mut profile = CameraProfile {
      name: Some("Test".into()),
      ..Default::default()
    };
    profile.color_matrix[0] = Some(vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    let dcp = CameraProfileFile {
      unique_camera_model: Some("DNGLab Test".into()),
      profile,
    };
    write_dcp(&dcp, &path, false)?;
    let loaded = dcp_option(&convert, "dcp_file");
    let exists = write_dcp(&dcp, &path, false);
    let replaced = write_dcp(&dcp, &path, true);
    let _ = std::fs::remove_file(&path);
    assert_eq!(loaded?, Some(dcp));
    assert!(matches!(exists, Err(AppError::AlreadyExists(_))));
    replaced
  }

  #[test]
  fn unique_file_names() {
    let mut taken = HashSet::new();
    let names: Vec<String> = ["A.dcp", "B.dcp", "a.dcp", "A.dcp", "A (2).dcp"]
      .into_iter()
      .map(|name| unique_file_name(name, &mut taken))
      .collect();
    assert_eq!(names, ["A.dcp", "B.dcp", "a (2).dcp", "A (3).dcp", "A (2) (2).dcp"]);
  }
}
