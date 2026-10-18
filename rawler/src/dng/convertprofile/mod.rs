// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Convert profiles
//!
//! A convert profile is a named set of DNG conversion parameters,
//! stored in TOML files. A single file can hold multiple profiles:
//!
//! ```toml
//! [convertprofile.myprofile]
//! version = 1
//! title = "My profile"
//! embed_raw = false
//! artist = "Jane Doe"
//! ```
//!
//! Each profile carries its own schema `version`, so profiles of
//! different schema versions can be mixed in the same file.
//! After loading, every profile is migrated to the latest schema,
//! exposed as [`ConvertProfile`].
//!
//! # Listing available profiles
//!
//! [`ConvertProfileRegistry::load`] loads the built-in profiles and all
//! profiles from the default search directories:
//!
//! ```no_run
//! use rawler::dng::convertprofile::ConvertProfileRegistry;
//!
//! let registry = ConvertProfileRegistry::load("myapp", None)?;
//! for (name, profile, source) in registry.iter() {
//!   println!("{}: {} ({:?})", name, profile.title, source);
//! }
//! let compat = registry.get("builtin_compat")?;
//! # Ok::<(), rawler::dng::convertprofile::ConvertProfileError>(())
//! ```

pub mod v1;

use std::{
  collections::{BTreeMap, HashMap},
  path::{Path, PathBuf},
};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

use super::convert::ConvertParams;

pub use v1::MultiFrameMode;

/// Latest convert profile schema version
pub const CONVERT_PROFILE_LATEST_VERSION: u32 = 1;

/// TOML key for the table of convert profiles
const CONVERT_PROFILE_KEY: &str = "convertprofile";

const BUILTIN_CONVERT_PROFILES: &str = include_str!("../../../data/profiles/builtin_convertprofiles.toml");

/// Convert profile in the latest schema version
pub type ConvertProfile = v1::ConvertProfileV1;

/// Errors for loading and resolving convert profiles
#[derive(Debug, thiserror::Error)]
pub enum ConvertProfileError {
  #[error("I/O error on '{}': {source}", path.display())]
  Io { path: PathBuf, source: std::io::Error },

  #[error("Failed to parse convert profiles{}: {source}", path.as_ref().map(|p| format!(" in '{}'", p.display())).unwrap_or_default())]
  Parse { path: Option<PathBuf>, source: toml::de::Error },

  #[error("Failed to serialize convert profiles: {0}")]
  Serialize(#[from] toml::ser::Error),

  #[error("Invalid convert profile '{profile}': {reason}")]
  Invalid { profile: String, reason: String },

  #[error("Convert profile '{name}' is defined in both '{}' and '{}'", first.display(), second.display())]
  Duplicate { name: String, first: PathBuf, second: PathBuf },

  #[error("Convert profile '{name}' not found, available profiles: {}", available.join(", "))]
  NotFound { name: String, available: Vec<String> },
}

pub type Result<T> = std::result::Result<T, ConvertProfileError>;

/// Convert profile as stored on disk, tagged by its schema version
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VersionedConvertProfile {
  V1(v1::ConvertProfileV1),
}

impl VersionedConvertProfile {
  /// Schema version of this profile
  pub fn version(&self) -> u32 {
    match self {
      Self::V1(_) => 1,
    }
  }

  /// Migrate the profile to the latest schema version
  pub fn into_latest(self) -> ConvertProfile {
    match self {
      Self::V1(profile) => profile,
    }
  }
}

impl From<ConvertProfile> for VersionedConvertProfile {
  fn from(profile: ConvertProfile) -> Self {
    Self::V1(profile)
  }
}

impl Serialize for VersionedConvertProfile {
  fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    struct Envelope<'a, T> {
      version: u32,
      #[serde(flatten)]
      profile: &'a T,
    }
    match self {
      Self::V1(profile) => Envelope {
        version: self.version(),
        profile,
      }
      .serialize(serializer),
    }
  }
}

impl<'de> Deserialize<'de> for VersionedConvertProfile {
  fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
    // serde can't dispatch internally tagged enums on integer tags,
    // so we read the table first and dispatch on `version` manually.
    let mut table = toml::Table::deserialize(deserializer)?;
    let version = match table.remove("version") {
      Some(toml::Value::Integer(version)) => version,
      Some(other) => return Err(D::Error::custom(format!("invalid `version` '{}', expected an integer", other))),
      None => return Err(D::Error::missing_field("version")),
    };
    match version {
      1 => toml::Value::Table(table).try_into().map(Self::V1).map_err(D::Error::custom),
      _ => Err(D::Error::custom(format!(
        "unsupported convert profile version {}, latest supported version is {}",
        version, CONVERT_PROFILE_LATEST_VERSION
      ))),
    }
  }
}

impl ConvertProfile {
  /// Check the profile for invalid values
  pub fn validate(&self, name: &str) -> Result<()> {
    if self.title.trim().is_empty() {
      return Err(ConvertProfileError::Invalid {
        profile: name.to_string(),
        reason: "title must not be empty".to_string(),
      });
    }
    if !(1..=7).contains(&self.ljpeg92_predictor) {
      return Err(ConvertProfileError::Invalid {
        profile: name.to_string(),
        reason: format!("ljpeg92_predictor must be in range 1..=7, got {}", self.ljpeg92_predictor),
      });
    }
    if self.force_preview && !self.add_preview {
      log::warn!("Convert profile '{}': force_preview has no effect, add_preview is disabled", name);
    }
    if self.force_thumbnail && !self.add_thumbnail {
      log::warn!("Convert profile '{}': force_thumbnail has no effect, add_thumbnail is disabled", name);
    }
    Ok(())
  }

  /// Build conversion parameters from this profile
  ///
  /// The image index is always 0, splitting multi-frame
  /// files according to [`ConvertProfile::multiframe`] is up to the caller.
  pub fn to_convert_params(&self, software: impl Into<String>) -> ConvertParams {
    ConvertParams {
      embedded: self.embed_raw,
      compression: self.compression,
      crop: self.crop,
      predictor: self.ljpeg92_predictor,
      preview: self.add_preview,
      thumbnail: self.add_thumbnail,
      force_preview: self.force_preview,
      force_thumbnail: self.force_thumbnail,
      artist: self.artist.clone(),
      copyright: self.copyright.clone(),
      software: software.into(),
      keep_mtime: self.keep_mtime,
      max_dng_backward_version: self.max_dng_backward_version,
      ..Default::default()
    }
  }
}

/// A TOML file containing convert profiles
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConvertProfileFile {
  #[serde(default)]
  pub convertprofile: BTreeMap<String, VersionedConvertProfile>,
}

impl ConvertProfileFile {
  /// Parse convert profiles from a TOML string
  pub fn from_toml_str(content: &str) -> Result<Self> {
    Self::parse(content, None)
  }

  /// Read convert profiles from a TOML file
  pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
    let path = path.as_ref();
    let content = std::fs::read_to_string(path).map_err(|source| ConvertProfileError::Io {
      path: path.to_path_buf(),
      source,
    })?;
    Self::parse(&content, Some(path))
  }

  fn parse(content: &str, path: Option<&Path>) -> Result<Self> {
    let parse_err = |source| ConvertProfileError::Parse {
      path: path.map(Path::to_path_buf),
      source,
    };
    let table: toml::Table = toml::from_str(content).map_err(parse_err)?;
    // Unknown top-level tables may be other profile types from newer versions
    for key in table.keys().filter(|key| *key != CONVERT_PROFILE_KEY) {
      log::warn!(
        "Ignoring unknown key '{}' in profile file{}",
        key,
        path.map(|p| format!(" '{}'", p.display())).unwrap_or_default()
      );
    }
    toml::from_str(content).map_err(parse_err)
  }

  /// Serialize to a TOML string, every profile keeps its schema version
  pub fn to_toml_string(&self) -> Result<String> {
    Ok(toml::to_string(self)?)
  }

  /// Add a profile in latest schema version
  pub fn insert(&mut self, name: impl Into<String>, profile: ConvertProfile) {
    self.convertprofile.insert(name.into(), profile.into());
  }

  /// Iterate over all profiles, migrated to the latest schema version
  pub fn into_profiles(self) -> impl Iterator<Item = (String, ConvertProfile)> {
    self.convertprofile.into_iter().map(|(name, profile)| (name, profile.into_latest()))
  }
}

/// Origin of a registered convert profile
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConvertProfileSource {
  Builtin,
  File(PathBuf),
}

/// Collection of named convert profiles from multiple sources
#[derive(Clone, Debug, Default)]
pub struct ConvertProfileRegistry {
  profiles: BTreeMap<String, (ConvertProfile, ConvertProfileSource)>,
}

impl ConvertProfileRegistry {
  /// Create an empty registry
  pub fn new() -> Self {
    Self::default()
  }

  /// Create a registry with all built-in profiles
  pub fn builtin() -> Self {
    let mut registry = Self::new();
    let file = ConvertProfileFile::from_toml_str(BUILTIN_CONVERT_PROFILES).expect("built-in convert profiles must be valid");
    for (name, profile) in file.into_profiles() {
      registry
        .add(name, profile, ConvertProfileSource::Builtin)
        .expect("built-in convert profiles must be valid");
    }
    registry
  }

  /// Load all available profiles
  ///
  /// The registry contains the built-in profiles and the profiles from
  /// `profile_path`. If `profile_path` is `None`, profiles are loaded from all
  /// existing directories returned by [`Self::default_search_dirs`].
  pub fn load(app_name: &str, profile_path: Option<&Path>) -> Result<Self> {
    let mut registry = Self::builtin();
    match profile_path {
      Some(dir) => {
        if !dir.is_dir() {
          return Err(ConvertProfileError::Io {
            path: dir.to_path_buf(),
            source: std::io::Error::from(std::io::ErrorKind::NotADirectory),
          });
        }
        registry.load_dir(dir)?;
      }
      None => {
        for dir in Self::default_search_dirs(app_name).into_iter().filter(|dir| dir.is_dir()) {
          log::debug!("Loading profiles from '{}'", dir.display());
          registry.load_dir(dir)?;
        }
      }
    }
    Ok(registry)
  }

  /// Default directories to search for profile files
  ///
  /// These are the shared `rawler` directory and the application specific
  /// directory, both in the per-user configuration directory of the platform.
  /// Profiles from later directories should override earlier ones.
  pub fn default_search_dirs(app_name: &str) -> Vec<PathBuf> {
    let Some(config) = dirs::config_dir() else {
      return Vec::new();
    };
    let mut dirs = vec![config.join("rawler").join("profiles")];
    if app_name != "rawler" {
      dirs.push(config.join(app_name).join("profiles"));
    }
    dirs
  }

  /// Add a profile, replacing an existing profile with the same name
  pub fn add(&mut self, name: impl Into<String>, profile: ConvertProfile, source: ConvertProfileSource) -> Result<()> {
    let name = name.into();
    profile.validate(&name)?;
    if let Some((_, previous)) = self.profiles.get(&name) {
      log::debug!("Convert profile '{}' from {:?} replaced by {:?}", name, previous, source);
    }
    self.profiles.insert(name, (profile, source));
    Ok(())
  }

  /// Load all profiles from a single TOML file
  pub fn load_file(&mut self, path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    for (name, profile) in ConvertProfileFile::from_path(path)?.into_profiles() {
      self.add(name, profile, ConvertProfileSource::File(path.to_path_buf()))?;
    }
    Ok(())
  }

  /// Load all profiles from `*.toml` files in a directory (not recursive)
  ///
  /// Files are processed in sorted order. A profile name defined in
  /// more than one file of the same directory is an error.
  pub fn load_dir(&mut self, dir: impl AsRef<Path>) -> Result<()> {
    let dir = dir.as_ref();
    let io_err = |source| ConvertProfileError::Io {
      path: dir.to_path_buf(),
      source,
    };
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(io_err)? {
      let path = entry.map_err(io_err)?.path();
      let is_toml = path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("toml"));
      if is_toml && path.is_file() {
        files.push(path);
      }
    }
    files.sort();

    let mut seen: HashMap<String, PathBuf> = HashMap::new();
    for path in files {
      for (name, profile) in ConvertProfileFile::from_path(&path)?.into_profiles() {
        if let Some(first) = seen.get(&name) {
          return Err(ConvertProfileError::Duplicate {
            name,
            first: first.clone(),
            second: path,
          });
        }
        seen.insert(name.clone(), path.clone());
        self.add(name, profile, ConvertProfileSource::File(path.clone()))?;
      }
    }
    Ok(())
  }

  /// Get a profile by name
  pub fn get(&self, name: &str) -> Result<&ConvertProfile> {
    self
      .profiles
      .get(name)
      .map(|(profile, _)| profile)
      .ok_or_else(|| ConvertProfileError::NotFound {
        name: name.to_string(),
        available: self.names().map(String::from).collect(),
      })
  }

  /// Get the origin of a profile
  pub fn source(&self, name: &str) -> Option<&ConvertProfileSource> {
    self.profiles.get(name).map(|(_, source)| source)
  }

  /// Check if a profile exists
  pub fn contains(&self, name: &str) -> bool {
    self.profiles.contains_key(name)
  }

  /// Names of all profiles, sorted
  ///
  /// Use [`Self::iter`] to list profiles together with title and origin.
  pub fn names(&self) -> impl Iterator<Item = &str> {
    self.profiles.keys().map(String::as_str)
  }

  /// Iterate over all profiles, sorted by name
  ///
  /// This is the API to list available profiles, e.g. for a user interface.
  pub fn iter(&self) -> impl Iterator<Item = (&str, &ConvertProfile, &ConvertProfileSource)> {
    self.profiles.iter().map(|(name, (profile, source))| (name.as_str(), profile, source))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::dng::{CropMode, DngCompression, DngVersion};

  /// Temporary directory, removed on drop
  struct TempDir(PathBuf);

  impl TempDir {
    fn new() -> Self {
      let path = std::env::temp_dir().join(format!("rawler-convertprofile-{}", uuid::Uuid::new_v4()));
      std::fs::create_dir_all(&path).expect("create temp dir");
      Self(path)
    }

    fn write(&self, name: &str, content: &str) -> PathBuf {
      let path = self.0.join(name);
      std::fs::write(&path, content).expect("write temp file");
      path
    }
  }

  impl Drop for TempDir {
    fn drop(&mut self) {
      let _ = std::fs::remove_dir_all(&self.0);
    }
  }

  #[test]
  fn builtin_profiles_are_valid() {
    let registry = ConvertProfileRegistry::builtin();
    assert!(registry.contains("builtin_compat"));
    assert!(registry.contains("builtin_archive"));
    assert!(registry.contains("builtin_latest"));
    assert_eq!(registry.names().count(), 3);
    assert!(registry.iter().all(|(_, profile, _)| !profile.title.is_empty()));
    assert_eq!(registry.get("builtin_archive").expect("profile").multiframe, MultiFrameMode::Split);
    assert_eq!(registry.get("builtin_compat").expect("profile").max_dng_backward_version, DngVersion::V1_4);
    assert_eq!(registry.get("builtin_latest").expect("profile").max_dng_backward_version, DngVersion::V1_6);
  }

  #[test]
  fn default_profile_matches_default_params() {
    let params = ConvertProfile::default().to_convert_params("DNGLab");
    let default = ConvertParams::default();
    assert_eq!(params.embedded, default.embedded);
    assert_eq!(params.compression, default.compression);
    assert_eq!(params.photometric_conversion, default.photometric_conversion);
    assert_eq!(params.apply_scaling, default.apply_scaling);
    assert_eq!(params.crop, default.crop);
    assert_eq!(params.predictor, default.predictor);
    assert_eq!(params.preview, default.preview);
    assert_eq!(params.thumbnail, default.thumbnail);
    assert_eq!(params.force_preview, default.force_preview);
    assert_eq!(params.force_thumbnail, default.force_thumbnail);
    assert_eq!(params.artist, default.artist);
    assert_eq!(params.copyright, default.copyright);
    assert_eq!(params.software, default.software);
    assert_eq!(params.index, default.index);
    assert_eq!(params.keep_mtime, default.keep_mtime);
    assert_eq!(params.dng_version, default.dng_version);
    assert_eq!(params.max_dng_backward_version, default.max_dng_backward_version);
  }

  #[test]
  fn roundtrip() {
    let profile = ConvertProfile {
      title: "Test title".into(),
      description: Some("Test".into()),
      max_dng_backward_version: DngVersion::V1_1,
      compression: DngCompression::Uncompressed,
      ljpeg92_predictor: 3,
      crop: CropMode::ActiveArea,
      add_preview: false,
      add_thumbnail: true,
      force_preview: true,
      force_thumbnail: true,
      embed_raw: false,
      multiframe: MultiFrameMode::Split,
      keep_mtime: true,
      artist: Some("Jane Doe".into()),
      copyright: Some("(c) Jane Doe".into()),
    };
    let mut file = ConvertProfileFile::default();
    file.insert("test", profile.clone());
    let toml = file.to_toml_string().expect("serialize");
    assert!(toml.contains("[convertprofile.test]"));
    assert!(toml.contains("version = 1"));
    assert!(toml.contains("title = \"Test title\""));
    assert!(toml.contains("max_dng_backward_version = \"1.1\""));
    assert!(toml.contains("crop = \"activearea\""));
    let parsed = ConvertProfileFile::from_toml_str(&toml).expect("parse");
    assert_eq!(parsed, file);
    assert_eq!(parsed.into_profiles().next().map(|(_, p)| p), Some(profile));
  }

  #[test]
  fn partial_profile_uses_defaults() {
    let file = ConvertProfileFile::from_toml_str("[convertprofile.x]\nversion = 1\ntitle = \"T\"\nembed_raw = false\n").expect("parse");
    let (name, profile) = file.into_profiles().next().expect("profile");
    assert_eq!(name, "x");
    assert_eq!(
      profile,
      ConvertProfile {
        title: "T".into(),
        embed_raw: false,
        ..Default::default()
      }
    );
  }

  #[test]
  fn mixed_profiles_in_one_file() {
    let file = ConvertProfileFile::from_toml_str(
      "[convertprofile.a]\nversion = 1\ntitle = \"T\"\n\n[convertprofile.b]\nversion = 1\ntitle = \"T\"\nkeep_mtime = true\n",
    )
    .expect("parse");
    assert_eq!(file.convertprofile.len(), 2);
    assert!(file.convertprofile.values().all(|p| p.version() == 1));
  }

  #[test]
  fn missing_version_is_rejected() {
    let err = ConvertProfileFile::from_toml_str("[convertprofile.x]\nembed_raw = false\n").expect_err("must fail");
    assert!(err.to_string().contains("version"), "{}", err);
  }

  #[test]
  fn unsupported_version_is_rejected() {
    let err =
      ConvertProfileFile::from_toml_str("[convertprofile.ok]\nversion = 1\ntitle = \"T\"\n\n[convertprofile.future]\nversion = 2\n").expect_err("must fail");
    let msg = err.to_string();
    assert!(msg.contains("unsupported convert profile version 2"), "{}", msg);
    assert!(msg.contains("future"), "{}", msg);
  }

  #[test]
  fn missing_title_is_rejected() {
    let err = ConvertProfileFile::from_toml_str("[convertprofile.x]\nversion = 1\nembed_raw = false\n").expect_err("must fail");
    assert!(err.to_string().contains("missing field `title`"), "{}", err);
  }

  #[test]
  fn empty_title_is_rejected() {
    let dir = TempDir::new();
    let path = dir.write("a.toml", "[convertprofile.x]\nversion = 1\ntitle = \" \"\n");
    let mut registry = ConvertProfileRegistry::new();
    assert!(matches!(registry.load_file(path), Err(ConvertProfileError::Invalid { .. })));
  }

  #[test]
  fn unknown_field_is_rejected() {
    let err = ConvertProfileFile::from_toml_str("[convertprofile.x]\nversion = 1\ntitle = \"T\"\npreview = false\n").expect_err("must fail");
    assert!(err.to_string().contains("preview"), "{}", err);
  }

  #[test]
  fn invalid_predictor_is_rejected() {
    let dir = TempDir::new();
    dir.write("a.toml", "[convertprofile.x]\nversion = 1\ntitle = \"T\"\nljpeg92_predictor = 8\n");
    let mut registry = ConvertProfileRegistry::new();
    assert!(matches!(registry.load_dir(&dir.0), Err(ConvertProfileError::Invalid { .. })));
  }

  #[test]
  fn load_dir_merges_files() {
    let dir = TempDir::new();
    dir.write("a.toml", "[convertprofile.a]\nversion = 1\ntitle = \"T\"\n");
    dir.write("b.toml", "[convertprofile.b]\nversion = 1\ntitle = \"T\"\nembed_raw = false\n");
    dir.write("ignored.txt", "not a profile");
    let mut registry = ConvertProfileRegistry::builtin();
    registry.load_dir(&dir.0).expect("load");
    assert!(registry.contains("a"));
    assert!(!registry.get("b").expect("profile").embed_raw);
    assert!(registry.contains("builtin_compat"));
    assert_eq!(registry.source("b"), Some(&ConvertProfileSource::File(dir.0.join("b.toml"))));
  }

  #[test]
  fn duplicate_in_same_dir_is_rejected() {
    let dir = TempDir::new();
    dir.write("a.toml", "[convertprofile.x]\nversion = 1\ntitle = \"T\"\n");
    dir.write("b.toml", "[convertprofile.x]\nversion = 1\ntitle = \"T\"\n");
    let mut registry = ConvertProfileRegistry::new();
    match registry.load_dir(&dir.0) {
      Err(ConvertProfileError::Duplicate { name, first, second }) => {
        assert_eq!(name, "x");
        assert_eq!(first, dir.0.join("a.toml"));
        assert_eq!(second, dir.0.join("b.toml"));
      }
      other => panic!("unexpected result: {:?}", other),
    }
  }

  #[test]
  fn later_dir_overrides_earlier_dir() {
    let shared = TempDir::new();
    let app = TempDir::new();
    shared.write("p.toml", "[convertprofile.x]\nversion = 1\ntitle = \"T\"\nkeep_mtime = false\n");
    app.write("p.toml", "[convertprofile.x]\nversion = 1\ntitle = \"T\"\nkeep_mtime = true\n");
    let mut registry = ConvertProfileRegistry::new();
    registry.load_dir(&shared.0).expect("load");
    registry.load_dir(&app.0).expect("load");
    assert!(registry.get("x").expect("profile").keep_mtime);
  }

  #[test]
  fn user_profile_overrides_builtin() {
    let dir = TempDir::new();
    let path = dir.write("a.toml", "[convertprofile.builtin_compat]\nversion = 1\ntitle = \"Mine\"\n");
    let mut registry = ConvertProfileRegistry::builtin();
    registry.load_file(&path).expect("load");
    assert_eq!(registry.get("builtin_compat").expect("profile").title, "Mine");
    assert_eq!(registry.source("builtin_compat"), Some(&ConvertProfileSource::File(path)));
  }

  #[test]
  fn load_with_profile_path() {
    let dir = TempDir::new();
    dir.write("a.toml", "[convertprofile.mine]\nversion = 1\ntitle = \"Mine\"\n");
    let registry = ConvertProfileRegistry::load("test", Some(&dir.0)).expect("load");
    assert!(registry.contains("mine"));
    assert!(registry.contains("builtin_compat"));
    assert_eq!(registry.source("mine"), Some(&ConvertProfileSource::File(dir.0.join("a.toml"))));
  }

  #[test]
  fn load_with_invalid_profile_path() {
    let dir = TempDir::new();
    let file = dir.write("a.toml", "");
    assert!(matches!(ConvertProfileRegistry::load("test", Some(&file)), Err(ConvertProfileError::Io { .. })));
    assert!(matches!(
      ConvertProfileRegistry::load("test", Some(&dir.0.join("missing"))),
      Err(ConvertProfileError::Io { .. })
    ));
  }

  #[test]
  fn unknown_profile_lists_available() {
    let registry = ConvertProfileRegistry::builtin();
    match registry.get("doesnotexist") {
      Err(err @ ConvertProfileError::NotFound { .. }) => assert!(err.to_string().contains("builtin_compat")),
      other => panic!("unexpected result: {:?}", other),
    }
  }
}
