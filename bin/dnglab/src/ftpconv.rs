// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use clap::ArgMatches;
use embedftp::config::{Config, FtpCallback};
use embedftp::server::serve;
use rawler::decoders::supported_extensions;
use rawler::RawFile;
use std::fs::File;
use std::io::{BufWriter, Cursor};
use std::path::PathBuf;

use tokio::runtime::Handle;

use crate::{PKG_NAME, PKG_VERSION};
use rawler::dng::dngwriter::ConvertParams;
use rawler::dng::dngwriter::{raw_to_dng_internal};

#[derive(Clone)]
struct FtpState {
  params: ConvertParams,
  keep_orig: bool,
}

impl FtpCallback for FtpState {
  fn stor_file(&self, path: PathBuf, data: Vec<u8>) -> Option<Vec<u8>> {
    if let Some(ext) = path.extension().map(|ext| ext.to_string_lossy()) {
      if is_ext_supported(&ext) {
        let mut filebuf = RawFile::new(&path, Cursor::new(data.clone())); // TODO: prevent clone

        let params = self.params.clone();
        let orig_filename = path.file_name().unwrap().to_str().unwrap();

        let out_path = path.with_extension("dng");
        let mut buf_file = BufWriter::new(File::create(out_path).unwrap());

        raw_to_dng_internal(&mut filebuf, &mut buf_file, orig_filename.into(), &params).unwrap();

        if self.keep_orig {
          return Some(data);
        } else {
          return None;
        }
      }
    }

    Some(data)
  }
}

/// Entry point for Clap sub command `ftpconvert`
pub async fn ftpserver(options: &ArgMatches) -> anyhow::Result<()> {
  let mut config = Config::new("foo").unwrap();

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
  };
  let keep_orig = options.get_flag("keep_orig");

  let state = FtpState { params, keep_orig };

  config.server_port = *options.get_one("ftp_port").unwrap_or(&2121);
  config.server_addr = options.get_one::<String>("ftp_listen").unwrap_or(&"127.0.0.1".to_string()).parse().unwrap();

  let out_path: &PathBuf = options.get_one("OUTPUT").expect("OUTPUT not available");

  serve(Handle::current(), out_path.to_path_buf(), config, state).await.unwrap();

  Ok(())
}

/// Check if file extension is a supported extension
fn is_ext_supported<T: AsRef<str>>(ext: T) -> bool {
  let uc = ext.as_ref().to_uppercase();
  supported_extensions().iter().any(|ext| ext.eq(&uc))
}
