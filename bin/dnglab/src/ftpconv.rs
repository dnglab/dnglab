// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use clap::ArgMatches;
use embedftp::server::serve;
use embedftp::config::{Config, FtpCallback};
use rawler::Buffer;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::runtime::Handle;

use crate::app::convert_bool;
use crate::dnggen::raw_to_dng_internal;
use crate::{
  dnggen::{ConvertParams, DngCompression},
  AppError, PKG_NAME, PKG_VERSION,
};

const SUPPORTED_FILE_EXT: [&'static str; 1] = ["CR3"];

#[derive(Clone)]
struct FtpState {
  params: ConvertParams,
  keep_orig: bool,
}

impl FtpCallback for FtpState {
  fn stor_file(&self, path: PathBuf, data: &Vec<u8>) -> bool {
    if let Some(ext) = path.extension().map(|ext| ext.to_string_lossy()) {
      if is_ext_supported(&ext) {
        // TODO: Large input file bug, we need to test the raw file before open it
        let in_buffer = Arc::new(Buffer::from(data.clone()));

        let params = self.params.clone();

        let orig_filename = path.file_name().unwrap().to_str().unwrap();

        let dng_content = raw_to_dng_internal(in_buffer, orig_filename.into(), &params).unwrap();

        let out_path = path.with_extension("dng");

        let mut file = File::create(out_path).unwrap();
        file.write_all(&dng_content).unwrap();

        if self.keep_orig {
          return false;
        } else {
          return true;
        }
      }
    }

    false
  }
}

/// Entry point for Clap sub command `ftpconvert`
pub async fn ftpconvert(options: &ArgMatches<'_>) -> anyhow::Result<()> {
  let mut config = Config::new("foo").unwrap();

  let params = ConvertParams {
    predictor: options.value_of("predictor").unwrap_or("1").parse::<u8>().unwrap(),
    embedded: convert_bool(options.value_of("embedded"), true).unwrap(),
    crop: convert_bool(options.value_of("crop"), true).unwrap(),
    preview: convert_bool(options.value_of("preview"), true).unwrap(),
    thumbnail: convert_bool(options.value_of("thumbnail"), true).unwrap(),
    compression: match options.value_of("compression") {
      Some("lossless") => DngCompression::Lossless,
      Some("none") => DngCompression::Uncompressed,
      Some(s) => {
        println!("Unknown compression: {}", s);
        return Err(AppError::InvalidArgs.into());
      }
      None => DngCompression::Lossless,
    },
    artist: options.value_of("artist").map(|v| String::from(v)),
    software: format!("{} {}", PKG_NAME, PKG_VERSION),
    index: 0,
  };
  let keep_orig = options.value_of("keep_orig").unwrap_or("yes") == "yes";

  let state = FtpState { params, keep_orig };

  config.server_port = options.value_of("ftp_port").unwrap_or("2121").parse().unwrap();
  config.server_addr = options.value_of("ftp_listen").unwrap_or("127.0.0.1").parse().unwrap();

  let out_path = PathBuf::from(options.value_of("OUTPUT").expect("OUTPUT not available"));

  serve(Handle::current(), out_path, config, state).await.unwrap();

  Ok(())
}

/// Check if file extension is a supported extension
fn is_ext_supported<T: AsRef<str>>(ext: T) -> bool {
  let uc = ext.as_ref().to_uppercase();
  SUPPORTED_FILE_EXT.iter().any(|ext| ext.eq(&uc))
}
