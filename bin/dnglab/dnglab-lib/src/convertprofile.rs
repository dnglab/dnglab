// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

use std::path::PathBuf;

use clap::{ArgMatches, parser::ValueSource};
use rawler::dng::{
  CropMode, DngCompression, DngVersion,
  convertprofile::{ConvertProfile, ConvertProfileRegistry, MultiFrameMode},
};

use crate::{AppError, Result};

/// Application name for profile search directories
const APP_NAME: &str = "dnglab";

/// Frames to convert from a raw file
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameSelection {
  /// Convert a single frame by index
  Single(usize),
  /// Convert every frame into a separate DNG file
  All,
}

/// Load built-in profiles and profiles from the profile directories
pub fn load_registry(options: &ArgMatches) -> Result<ConvertProfileRegistry> {
  Ok(ConvertProfileRegistry::load(
    APP_NAME,
    options.get_one::<PathBuf>("profile_path").map(PathBuf::as_path),
  )?)
}

/// Resolve the convert profile from command line options
///
/// If no profile is given, no profile is used and the defaults apply.
/// Values given explicitly on the command line override the profile values.
pub fn resolve_convert_profile(options: &ArgMatches) -> Result<(ConvertProfile, FrameSelection)> {
  let mut profile = match options.get_one::<String>("profile") {
    Some(name) => {
      let registry = load_registry(options)?;
      log::debug!("Using convert profile '{}' from {:?}", name, registry.source(name));
      registry.get(name)?.clone()
    }
    None => {
      log::debug!("No convert profile selected, using defaults");
      ConvertProfile::default()
    }
  };

  apply_cli_overrides(&mut profile, options);
  let frames = frame_selection(&profile, options)?;
  Ok((profile, frames))
}

/// Get a value only if it was given on the command line
fn from_cli<T: Clone + Send + Sync + 'static>(options: &ArgMatches, id: &str) -> Option<T> {
  if options.value_source(id) == Some(ValueSource::CommandLine) {
    options.get_one::<T>(id).cloned()
  } else {
    None
  }
}

fn apply_cli_overrides(profile: &mut ConvertProfile, options: &ArgMatches) {
  if let Some(value) = from_cli::<DngCompression>(options, "compression") {
    profile.compression = value;
  }
  if let Some(value) = from_cli::<u8>(options, "predictor") {
    profile.ljpeg92_predictor = value;
  }
  if let Some(value) = from_cli::<bool>(options, "preview") {
    profile.add_preview = value;
  }
  if let Some(value) = from_cli::<bool>(options, "thumbnail") {
    profile.add_thumbnail = value;
  }
  if let Some(value) = from_cli::<bool>(options, "force_preview") {
    profile.force_preview = value;
  }
  if let Some(value) = from_cli::<bool>(options, "force_thumbnail") {
    profile.force_thumbnail = value;
  }
  if let Some(value) = from_cli::<bool>(options, "embedded") {
    profile.embed_raw = value;
  }
  if let Some(value) = from_cli::<CropMode>(options, "crop") {
    profile.crop = value;
  }
  if let Some(value) = from_cli::<DngVersion>(options, "max_dng_backward_version") {
    profile.max_dng_backward_version = value;
  }
  if let Some(value) = from_cli::<String>(options, "artist") {
    profile.artist = Some(value);
  }
  if let Some(value) = from_cli::<String>(options, "copyright") {
    profile.copyright = Some(value);
  }
  if let Some(value) = from_cli::<bool>(options, "keep_mtime") {
    profile.keep_mtime = value;
  }
}

/// Explicit `--image-index` wins over the profile multiframe mode
fn frame_selection(profile: &ConvertProfile, options: &ArgMatches) -> Result<FrameSelection> {
  match from_cli::<String>(options, "index") {
    Some(index) if index.eq_ignore_ascii_case("all") => Ok(FrameSelection::All),
    Some(index) => index
      .parse::<usize>()
      .map(FrameSelection::Single)
      .map_err(|_| AppError::InvalidCmdSwitch(format!("Invalid image index '{}', expected a number or 'all'", index))),
    None => Ok(match profile.multiframe {
      MultiFrameMode::First => FrameSelection::Single(0),
      MultiFrameMode::Split => FrameSelection::All,
    }),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::app::create_app;

  /// Parse convert arguments, `--profile-path` defaults to an empty
  /// directory so tests don't depend on the user configuration.
  fn convert_matches(args: &[&str]) -> ArgMatches {
    let empty = std::env::temp_dir().join(format!("dnglab-convertprofile-empty-{}", std::process::id()));
    std::fs::create_dir_all(&empty).expect("create dir");
    let empty = empty.to_string_lossy().to_string();
    let mut argv = vec!["dnglab"];
    if !args.contains(&"--profile-path") {
      argv.extend_from_slice(&["--profile-path", &empty]);
    }
    argv.extend_from_slice(args);
    let matches = create_app().try_get_matches_from(argv).expect("valid arguments");
    matches.subcommand_matches("convert").expect("convert subcommand").clone()
  }

  #[test]
  fn cli_overrides_profile() {
    let options = convert_matches(&["convert", "-p", "builtin_latest", "-c", "uncompressed", "in.raw", "out.dng"]);
    let (profile, frames) = resolve_convert_profile(&options).expect("profile");
    // From profile
    assert_eq!(profile.max_dng_backward_version, DngVersion::V1_6);
    assert_eq!(frames, FrameSelection::All);
    // From command line
    assert_eq!(profile.compression, DngCompression::Uncompressed);
  }

  #[test]
  fn cli_defaults_do_not_override_profile() {
    let options = convert_matches(&["convert", "-p", "builtin_archive", "in.raw", "out.dng"]);
    let (profile, frames) = resolve_convert_profile(&options).expect("profile");
    assert!(profile.keep_mtime);
    assert_eq!(frames, FrameSelection::All);
  }

  #[test]
  fn no_profile_uses_defaults() {
    let options = convert_matches(&["convert", "in.raw", "out.dng"]);
    let (profile, frames) = resolve_convert_profile(&options).expect("profile");
    assert_eq!(profile, ConvertProfile::default());
    assert_eq!(frames, FrameSelection::Single(0));

    let options = convert_matches(&["convert", "--embed-raw", "false", "in.raw", "out.dng"]);
    let (profile, _) = resolve_convert_profile(&options).expect("profile");
    assert!(!profile.embed_raw);
  }

  #[test]
  fn explicit_index_overrides_multiframe() {
    let options = convert_matches(&["convert", "-p", "builtin_archive", "--image-index", "2", "in.raw", "out.dng"]);
    let (_, frames) = resolve_convert_profile(&options).expect("profile");
    assert_eq!(frames, FrameSelection::Single(2));
  }

  #[test]
  fn global_profile_path() {
    let dir = std::env::temp_dir().join(format!("dnglab-convertprofile-mine-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create dir");
    std::fs::write(
      dir.join("mine.toml"),
      "[convertprofile.mine]\nversion = 1\ntitle = \"T\"\ncopyright = \"(c) Test\"\n",
    )
    .expect("write");
    let path = dir.to_string_lossy().to_string();
    let options = convert_matches(&["--profile-path", &path, "convert", "-p", "mine", "in.raw", "out.dng"]);
    let result = resolve_convert_profile(&options);
    let _ = std::fs::remove_dir_all(&dir);
    let (profile, _) = result.expect("profile");
    assert_eq!(profile.copyright.as_deref(), Some("(c) Test"));
  }

  #[test]
  fn unknown_profile_fails() {
    let options = convert_matches(&["convert", "-p", "doesnotexist", "in.raw", "out.dng"]);
    assert!(resolve_convert_profile(&options).is_err());
  }
}
