// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! dnglab configuration file
//!
//! ```toml
//! version = 1
//! profile_path = "profiles"
//!
//! [convert]
//! default_profile = "builtin_latest"
//! ```

use std::path::{Path, PathBuf};

use clap::ArgMatches;
use serde::{Deserialize, Serialize};

/// Latest configuration schema version
pub const CONFIG_LATEST_VERSION: u32 = 1;

/// File name of the configuration file
pub const CONFIG_FILE_NAME: &str = "config.toml";

/// Configuration in the latest schema version
pub type DnglabConfig = DnglabConfigV1;

/// Errors for loading the configuration file
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
  #[error("I/O error on config file '{}': {source}", path.display())]
  Io { path: PathBuf, source: std::io::Error },

  #[error("Failed to parse config file '{}': {source}", path.display())]
  Parse { path: PathBuf, source: toml::de::Error },

  #[error("Config file '{}' has no `version`", path.display())]
  MissingVersion { path: PathBuf },

  #[error("Config file '{}' has unsupported version {version}, latest supported version is {}", path.display(), CONFIG_LATEST_VERSION)]
  UnsupportedVersion { path: PathBuf, version: String },
}

/// Configuration, schema version 1
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DnglabConfigV1 {
  /// Directory with profile files, used if `--profile-path` is not given
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub profile_path: Option<PathBuf>,
  #[serde(default)]
  pub convert: ConvertConfigV1,
}

/// Configuration for `convert` and `ftpserver`, schema version 1
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConvertConfigV1 {
  /// Convert profile used if `--profile` is not given
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub default_profile: Option<String>,
}

impl DnglabConfig {
  /// Parse configuration from a TOML string
  ///
  /// `path` is the location of the configuration file. It is used for
  /// error messages and to resolve relative paths.
  pub fn from_toml_str(content: &str, path: &Path) -> Result<Self, ConfigError> {
    let parse_err = |source| ConfigError::Parse {
      path: path.to_path_buf(),
      source,
    };
    let mut table: toml::Table = toml::from_str(content).map_err(parse_err)?;
    let mut config: Self = match table.remove("version") {
      Some(toml::Value::Integer(1)) => toml::Value::Table(table).try_into().map_err(parse_err)?,
      Some(version) => {
        return Err(ConfigError::UnsupportedVersion {
          path: path.to_path_buf(),
          version: version.to_string(),
        });
      }
      None => return Err(ConfigError::MissingVersion { path: path.to_path_buf() }),
    };
    if let (Some(profile_path), Some(base_dir)) = (config.profile_path.as_mut(), path.parent()) {
      if profile_path.is_relative() {
        *profile_path = base_dir.join(&*profile_path);
      }
    }
    Ok(config)
  }

  /// Read configuration from a file
  pub fn from_path(path: &Path) -> Result<Self, ConfigError> {
    let content = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
      path: path.to_path_buf(),
      source,
    })?;
    Self::from_toml_str(&content, path)
  }

  /// Default location of the configuration file in the user configuration directory
  pub fn default_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("dnglab").join(CONFIG_FILE_NAME))
  }
}

/// Load the configuration given by `--config`, or from the default location
///
/// Returns the configuration and the path of the file it was loaded from.
/// If `--config` is not given and no file exists at the default location,
/// the default configuration is returned.
pub fn load_config(options: &ArgMatches) -> crate::Result<(DnglabConfig, Option<PathBuf>)> {
  let path = match options.get_one::<PathBuf>("config") {
    Some(path) => path.clone(),
    None => match DnglabConfig::default_path() {
      Some(path) if path.is_file() => path,
      _ => return Ok((DnglabConfig::default(), None)),
    },
  };
  log::debug!("Loading config file '{}'", path.display());
  Ok((DnglabConfig::from_path(&path)?, Some(path)))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn parse(content: &str) -> Result<DnglabConfig, ConfigError> {
    DnglabConfig::from_toml_str(content, Path::new("/etc/dnglab/config.toml"))
  }

  #[test]
  fn full_config() {
    let config = parse("version = 1\nprofile_path = \"/opt/profiles\"\n\n[convert]\ndefault_profile = \"builtin_latest\"\n").expect("parse");
    assert_eq!(config.profile_path, Some(PathBuf::from("/opt/profiles")));
    assert_eq!(config.convert.default_profile.as_deref(), Some("builtin_latest"));
  }

  #[test]
  fn empty_config_has_defaults() {
    assert_eq!(parse("version = 1\n").expect("parse"), DnglabConfig::default());
  }

  #[test]
  fn relative_profile_path_is_resolved() {
    let config = parse("version = 1\nprofile_path = \"profiles\"\n").expect("parse");
    assert_eq!(config.profile_path, Some(PathBuf::from("/etc/dnglab/profiles")));
  }

  #[test]
  fn missing_version_is_rejected() {
    assert!(matches!(parse("[convert]\n"), Err(ConfigError::MissingVersion { .. })));
  }

  #[test]
  fn unsupported_version_is_rejected() {
    assert!(matches!(parse("version = 2\n"), Err(ConfigError::UnsupportedVersion { .. })));
    assert!(matches!(parse("version = \"1\"\n"), Err(ConfigError::UnsupportedVersion { .. })));
  }

  #[test]
  fn unknown_key_is_rejected() {
    assert!(matches!(parse("version = 1\n[convert]\nprofile = \"x\"\n"), Err(ConfigError::Parse { .. })));
    assert!(matches!(parse("version = 1\nfoo = 1\n"), Err(ConfigError::Parse { .. })));
  }
}
