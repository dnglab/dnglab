// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

use std::path::PathBuf;

use clap::{ArgMatches, parser::ValueSource};
use rawler::dng::{
  ApplyDngOpcodes, CropMode, DngCompression, DngVersion,
  convertprofile::{ConvertProfile, ConvertProfileRegistry, MultiFrameMode},
  opcodes::OpcodeSelector,
};

use crate::{
  AppError, Result,
  config::{DnglabConfig, load_config},
};

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

/// Load built-in profiles and profiles from the profile directory
///
/// The directory is `--profile-path`, else `profile_path` from the config
/// file, else the default search directories.
fn load_registry(options: &ArgMatches, config: &DnglabConfig) -> Result<ConvertProfileRegistry> {
  let profile_dir = options.get_one::<PathBuf>("profile_path").or(config.profile_path.as_ref());
  Ok(ConvertProfileRegistry::load(APP_NAME, profile_dir.map(PathBuf::as_path))?)
}

/// Resolve the convert profile from command line options and config file
///
/// The profile is `--profile`, else `convert.default_profile` from the
/// config file. If neither is given, no profile is used and the defaults
/// apply, built-in profiles are never used implicitly. The profile directory
/// is `--profile-path`, else `profile_path` from the config file, else the
/// default search directories. Values given explicitly on the command line
/// override the profile values.
pub fn resolve_convert_profile(options: &ArgMatches) -> Result<(ConvertProfile, FrameSelection)> {
  let (config, config_path) = load_config(options)?;
  let mut profile = match (options.get_one::<String>("profile"), &config.convert.default_profile) {
    (Some(name), _) => {
      let registry = load_registry(options, &config)?;
      log::debug!("Using convert profile '{}' from {:?}", name, registry.source(name));
      registry.get(name)?.clone()
    }
    (None, Some(name)) => {
      let registry = load_registry(options, &config)?;
      log::debug!("Using default convert profile '{}' from {:?}", name, registry.source(name));
      registry
        .get(name)
        .map_err(|err| {
          AppError::General(format!(
            "Default convert profile from config file '{}': {}",
            config_path.as_deref().map(|p| p.display().to_string()).unwrap_or_default(),
            err
          ))
        })?
        .clone()
    }
    (None, None) => {
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

/// Get all values only if they were given on the command line
fn many_from_cli<T: Clone + Send + Sync + 'static>(options: &ArgMatches, id: &str) -> Option<Vec<T>> {
  if options.value_source(id) == Some(ValueSource::CommandLine) {
    options.get_many::<T>(id).map(|values| values.cloned().collect())
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
  if let Some(value) = from_cli::<u8>(options, "lossy_jpeg_quality") {
    profile.lossy_jpeg_quality = value;
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
  if let Some(value) = from_cli::<ApplyDngOpcodes>(options, "apply_dng_opcodes") {
    profile.apply_dng_opcodes = value;
  }
  if let Some(value) = many_from_cli::<OpcodeSelector>(options, "remove_dng_opcodes") {
    profile.remove_dng_opcodes = value;
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
  use std::sync::atomic::{AtomicUsize, Ordering};

  /// Parse convert arguments, `--profile-path` defaults to an empty
  /// directory so tests don't depend on the user configuration.
  /// Temporary directory, removed on drop
  struct TempDir(PathBuf);

  impl TempDir {
    fn new() -> Self {
      static COUNTER: AtomicUsize = AtomicUsize::new(0);
      let id = COUNTER.fetch_add(1, Ordering::Relaxed);
      let path = std::env::temp_dir().join(format!("dnglab-convertprofile-{}-{}", std::process::id(), id));
      std::fs::create_dir_all(&path).expect("create dir");
      Self(path)
    }

    fn write(&self, name: &str, content: &str) -> String {
      let path = self.0.join(name);
      std::fs::write(&path, content).expect("write");
      path.to_string_lossy().to_string()
    }

    fn path(&self) -> String {
      self.0.to_string_lossy().to_string()
    }
  }

  impl Drop for TempDir {
    fn drop(&mut self) {
      let _ = std::fs::remove_dir_all(&self.0);
    }
  }

  /// Parse arguments, independent of the user configuration:
  /// `--config` defaults to an empty config and `--profile-path`
  /// to an empty directory, unless given.
  fn matches(tmp: &TempDir, args: &[&str]) -> ArgMatches {
    let config = tmp.write("empty-config.toml", "version = 1\n");
    let empty = tmp.0.join("empty-profiles");
    std::fs::create_dir_all(&empty).expect("create dir");
    let empty_path = empty.to_string_lossy().to_string();
    let mut argv = vec!["dnglab"];
    if !args.contains(&"--config") {
      argv.extend_from_slice(&["--config", &config]);
      if !args.contains(&"--profile-path") {
        argv.extend_from_slice(&["--profile-path", &empty_path]);
      }
    }
    argv.extend_from_slice(args);
    let matches = create_app().try_get_matches_from(argv).expect("valid arguments");
    matches.subcommand_matches("convert").expect("convert subcommand").clone()
  }

  /// Like [`matches`], the returned [`TempDir`] must be kept alive while the matches are used
  fn convert_matches(args: &[&str]) -> (TempDir, ArgMatches) {
    let tmp = TempDir::new();
    let options = matches(&tmp, args);
    (tmp, options)
  }

  #[test]
  fn cli_overrides_profile() {
    let (_tmp, options) = convert_matches(&["convert", "-p", "builtin_latest", "-c", "uncompressed", "in.raw", "out.dng"]);
    let (profile, frames) = resolve_convert_profile(&options).expect("profile");
    // From profile
    assert_eq!(profile.max_dng_backward_version, DngVersion::V1_6);
    assert_eq!(frames, FrameSelection::All);
    // From command line
    assert_eq!(profile.compression, DngCompression::Uncompressed);
  }

  #[test]
  fn cli_overrides_lossy_jpeg_quality() {
    let tmp = TempDir::new();
    tmp.write("mine.toml", "[convertprofile.mine]\nversion = 1\ntitle = \"T\"\nlossy_jpeg_quality = 60\n");
    let path = tmp.path();
    let (_tmp, options) = convert_matches(&["--profile-path", &path, "convert", "-p", "mine", "-c", "lossy", "in.raw", "out.dng"]);
    let (profile, _) = resolve_convert_profile(&options).expect("profile");
    assert_eq!(profile.compression, DngCompression::Lossy);
    assert_eq!(profile.lossy_jpeg_quality, 60);
    let args = [
      "--profile-path",
      &path,
      "convert",
      "-p",
      "mine",
      "--lossy-jpeg-quality",
      "80",
      "in.raw",
      "out.dng",
    ];
    let (_tmp, options) = convert_matches(&args);
    let (profile, _) = resolve_convert_profile(&options).expect("profile");
    assert_eq!(profile.lossy_jpeg_quality, 80);
  }

  #[test]
  fn cli_overrides_dng_opcodes() {
    let tmp = TempDir::new();
    tmp.write(
      "mine.toml",
      "[convertprofile.mine]\nversion = 1\ntitle = \"T\"\napply_dng_opcodes = \"list3\"\nremove_dng_opcodes = [\"GainMap\"]\n",
    );
    let path = tmp.path();
    let (_tmp, options) = convert_matches(&["--profile-path", &path, "convert", "-p", "mine", "in.raw", "out.dng"]);
    let (profile, _) = resolve_convert_profile(&options).expect("profile");
    assert_eq!(profile.apply_dng_opcodes, ApplyDngOpcodes::List3);
    assert_eq!(profile.remove_dng_opcodes, ["GainMap".parse::<OpcodeSelector>().expect("selector")]);
    // The command line replaces the profile values
    let args = [
      "--profile-path",
      &path,
      "convert",
      "-p",
      "mine",
      "--apply-dng-opcodes",
      "list1",
      "--remove-dng-opcodes",
      "3:WarpRectilinear,2:GainMap",
      "--remove-dng-opcodes",
      "51966",
      "in.raw",
      "out.dng",
    ];
    let (_tmp, options) = convert_matches(&args);
    let (profile, _) = resolve_convert_profile(&options).expect("profile");
    assert_eq!(profile.apply_dng_opcodes, ApplyDngOpcodes::List1);
    let removed: Vec<String> = profile.remove_dng_opcodes.iter().map(ToString::to_string).collect();
    assert_eq!(removed, ["3:WarpRectilinear", "2:GainMap", "51966"]);
  }

  #[test]
  fn cli_defaults_do_not_override_profile() {
    let (_tmp, options) = convert_matches(&["convert", "-p", "builtin_archive", "in.raw", "out.dng"]);
    let (profile, frames) = resolve_convert_profile(&options).expect("profile");
    assert!(profile.keep_mtime);
    assert_eq!(frames, FrameSelection::All);
  }

  #[test]
  fn no_profile_uses_defaults() {
    let (_tmp, options) = convert_matches(&["convert", "in.raw", "out.dng"]);
    let (profile, frames) = resolve_convert_profile(&options).expect("profile");
    assert_eq!(profile, ConvertProfile::default());
    assert_eq!(frames, FrameSelection::Single(0));

    let (_tmp, options) = convert_matches(&["convert", "--embed-raw", "false", "in.raw", "out.dng"]);
    let (profile, _) = resolve_convert_profile(&options).expect("profile");
    assert!(!profile.embed_raw);
  }

  #[test]
  fn explicit_index_overrides_multiframe() {
    let (_tmp, options) = convert_matches(&["convert", "-p", "builtin_archive", "--image-index", "2", "in.raw", "out.dng"]);
    let (_, frames) = resolve_convert_profile(&options).expect("profile");
    assert_eq!(frames, FrameSelection::Single(2));
  }

  #[test]
  fn global_profile_path() {
    let tmp = TempDir::new();
    tmp.write("mine.toml", "[convertprofile.mine]\nversion = 1\ntitle = \"T\"\ncopyright = \"(c) Test\"\n");
    let path = tmp.path();
    let (_tmp, options) = convert_matches(&["--profile-path", &path, "convert", "-p", "mine", "in.raw", "out.dng"]);
    let (profile, _) = resolve_convert_profile(&options).expect("profile");
    assert_eq!(profile.copyright.as_deref(), Some("(c) Test"));
  }

  /// Config with `profile_path = "profiles"` and default profile `mine`
  fn config_with_profiles(tmp: &TempDir) -> String {
    std::fs::create_dir_all(tmp.0.join("profiles")).expect("create dir");
    tmp.write(
      "profiles/mine.toml",
      "[convertprofile.mine]\nversion = 1\ntitle = \"T\"\ncopyright = \"(c) Mine\"\n",
    );
    tmp.write(
      "config.toml",
      "version = 1\nprofile_path = \"profiles\"\n\n[convert]\ndefault_profile = \"mine\"\n",
    )
  }

  #[test]
  fn config_default_profile() {
    let tmp = TempDir::new();
    let config = config_with_profiles(&tmp);
    let options = matches(&tmp, &["--config", &config, "convert", "in.raw", "out.dng"]);
    let (profile, _) = resolve_convert_profile(&options).expect("profile");
    assert_eq!(profile.copyright.as_deref(), Some("(c) Mine"));
  }

  #[test]
  fn cli_profile_beats_config_default() {
    let tmp = TempDir::new();
    let config = config_with_profiles(&tmp);
    let options = matches(&tmp, &["--config", &config, "convert", "-p", "builtin_latest", "in.raw", "out.dng"]);
    let (profile, _) = resolve_convert_profile(&options).expect("profile");
    assert_eq!(profile.max_dng_backward_version, DngVersion::V1_6);
  }

  #[test]
  fn cli_profile_path_beats_config() {
    let tmp = TempDir::new();
    let config = config_with_profiles(&tmp);
    let other = TempDir::new();
    other.write("mine.toml", "[convertprofile.mine]\nversion = 1\ntitle = \"T\"\ncopyright = \"(c) Other\"\n");
    let other_path = other.path();
    let options = matches(&tmp, &["--config", &config, "--profile-path", &other_path, "convert", "in.raw", "out.dng"]);
    let (profile, _) = resolve_convert_profile(&options).expect("profile");
    assert_eq!(profile.copyright.as_deref(), Some("(c) Other"));
  }

  #[test]
  fn unknown_config_default_profile_fails() {
    let tmp = TempDir::new();
    let empty = TempDir::new();
    let config = tmp.write(
      "config.toml",
      &format!("version = 1\nprofile_path = '{}'\n\n[convert]\ndefault_profile = \"nope\"\n", empty.path()),
    );
    let options = matches(&tmp, &["--config", &config, "convert", "in.raw", "out.dng"]);
    let err = resolve_convert_profile(&options).expect_err("must fail");
    assert!(err.to_string().contains("config.toml"), "{}", err);
  }

  #[test]
  fn missing_config_file_fails() {
    let tmp = TempDir::new();
    let missing = tmp.0.join("missing.toml").to_string_lossy().to_string();
    let options = matches(&tmp, &["--config", &missing, "convert", "in.raw", "out.dng"]);
    assert!(resolve_convert_profile(&options).is_err());
  }

  #[test]
  fn unknown_profile_fails() {
    let (_tmp, options) = convert_matches(&["convert", "-p", "doesnotexist", "in.raw", "out.dng"]);
    assert!(resolve_convert_profile(&options).is_err());
  }
}
