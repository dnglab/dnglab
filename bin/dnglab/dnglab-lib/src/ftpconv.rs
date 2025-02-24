// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use clap::ArgMatches;
use embedftp::config::{Config, FtpCallback};
use embedftp::server::serve;
use rawler::decoders::{supported_extensions, RawDecodeParams};
use rawler::rawsource::RawSource;
use std::ffi::OsStr;
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use tokio::runtime::Handle;

use crate::jobs::raw2dng::copy_mtime_from_rawsource;
use crate::{PKG_NAME, PKG_VERSION};
use rawler::dng::convert::{convert_raw_source, ConvertParams};

#[derive(Clone)]
struct FtpState {
  params: ConvertParams,
  keep_orig: bool,
}

impl FtpCallback for FtpState {
  fn stor_file(&self, path: &Path, data: Arc<Vec<u8>>) -> std::io::Result<bool> {
    if let Some(ext) = path.extension().map(|ext| ext.to_string_lossy()) {
      if is_ext_supported(ext) {
        let original_filename = path.file_name().and_then(OsStr::to_str).unwrap_or_default();
        let rawfile = RawSource::new_from_shared_vec(data).with_path(original_filename);
        let out_path = path.with_extension("dng");
        let mut dng = BufWriter::new(File::create(out_path)?);
        convert_raw_source(&rawfile, &mut dng, original_filename, &self.params).map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))?;
        if self.params.keep_mtime {
          if let Err(err) = copy_mtime_from_rawsource(&rawfile, &dng.into_inner().expect("Get inner() failed"), None, &self.params) {
            log::warn!("Failed to set mtime, continue anyway: {}", err);
          }
        }
        return Ok(!self.keep_orig);
      }
    }
    Ok(false)
  }
}

/// Entry point for Clap sub command `ftpconvert`
pub async fn ftpserver(options: &ArgMatches) -> crate::Result<()> {
  let mut config = Config::new("foo").unwrap(); // TODO: Needs cleanup

  let params = ConvertParams {
    predictor: *options.get_one("predictor").expect("predictor has no default"),
    embedded: options.get_flag("embedded"),
    photometric_conversion: Default::default(),
    crop: *options.get_one("crop").expect("crop has no default"),
    preview: options.get_flag("preview"),
    thumbnail: options.get_flag("thumbnail"),
    compression: *options.get_one("compression").expect("compression has no default"),
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

  let out_path: &PathBuf = options.get_one("OUTPUT").expect("OUTPUT not available");

  serve(Handle::current(), out_path.to_path_buf(), config, state).await?;

  Ok(())
}

/// Check if file extension is a supported extension
fn is_ext_supported<T: AsRef<str>>(ext: T) -> bool {
  let uc = ext.as_ref().to_uppercase();
  supported_extensions().iter().any(|ext| ext.eq(&uc))
}
