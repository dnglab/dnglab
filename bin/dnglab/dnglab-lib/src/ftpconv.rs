// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use clap::ArgMatches;
use embedftp::config::{Config, FtpCallback};
use embedftp::server::serve;
use rawler::decoders::supported_extensions;
use rawler::rawsource::RawSource;
use std::ffi::OsStr;
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::runtime::Handle;

use crate::jobs::raw2dng::copy_mtime_from_rawsource;
use crate::{PKG_NAME, PKG_VERSION};
use rawler::dng::convert::{ConvertParams, convert_raw_source};

#[derive(Clone)]
struct FtpState {
  params: ConvertParams,
  keep_orig: bool,
}

impl FtpCallback for FtpState {
  async fn stor_file(&self, path: &Path, data: Arc<Vec<u8>>) -> std::io::Result<bool> {
    let Some(ext) = path.extension().map(|e| e.to_string_lossy().to_string()) else {
      return Ok(false);
    };
    if !is_ext_supported(&ext) {
      return Ok(false);
    }
    let state = self.clone();
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || -> std::io::Result<bool> {
      let original_filename = path.file_name().and_then(OsStr::to_str).unwrap_or_default();
      let rawfile = RawSource::new_from_shared_vec(data).with_path(original_filename);
      let out_path = path.with_extension("dng");
      let mut dng = BufWriter::new(File::create(out_path)?);
      let _info = convert_raw_source(&rawfile, &mut dng, original_filename, &state.params).map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))?;
      if state.params.keep_mtime {
        if let Err(err) = copy_mtime_from_rawsource(
          &rawfile,
          &dng
            .into_inner()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("Can't access inner file: {e}")))?,
          None,
          &state.params,
        ) {
          log::warn!("Failed to set mtime, continue anyway: {}", err);
        }
      }
      Ok(!state.keep_orig)
    })
    .await
    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("join error: {e}")))?
  }
}

/// Entry point for Clap sub command `ftpconvert`
pub async fn ftpserver(options: &ArgMatches) -> crate::Result<()> {
  let mut config = Config::new("foo").ok_or_else(|| anyhow::anyhow!("failed to create FTP config"))?;

  let params = ConvertParams {
    predictor: *options
      .get_one("predictor")
      .ok_or_else(|| crate::AppError::InvalidCmdSwitch("predictor has no default".into()))?,
    embedded: options.get_flag("embedded"),
    photometric_conversion: Default::default(),
    crop: *options
      .get_one("crop")
      .ok_or_else(|| crate::AppError::InvalidCmdSwitch("crop has no default".into()))?,
    preview: options.get_flag("preview"),
    thumbnail: options.get_flag("thumbnail"),
    compression: *options
      .get_one("compression")
      .ok_or_else(|| crate::AppError::InvalidCmdSwitch("compression has no default".into()))?,
    artist: options.get_one("artist").cloned(),
    software: format!("{} {}", PKG_NAME, PKG_VERSION),
    index: 0,
    apply_scaling: false,
    keep_mtime: options.get_flag("keep_mtime"),
  };
  let keep_orig = options.get_flag("keep_orig");

  let state = FtpState { params, keep_orig };

  config.server_port = *options.get_one("ftp_port").unwrap_or(&2121);
  config.server_addr = options.get_one::<String>("ftp_listen").unwrap_or(&"127.0.0.1".to_string()).parse()?;

  let out_path: &PathBuf = options
    .get_one("OUTPUT")
    .ok_or_else(|| crate::AppError::InvalidCmdSwitch("OUTPUT not available".into()))?;

  serve(Handle::current(), out_path.to_path_buf(), config, state).await?;

  Ok(())
}

/// Check if file extension is a supported extension
fn is_ext_supported<T: AsRef<str>>(ext: T) -> bool {
  let uc = ext.as_ref().to_uppercase();
  supported_extensions().iter().any(|ext| ext.eq(&uc))
}
