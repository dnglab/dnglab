// SPDX-License-Identifier: MIT
// Originally written by Guillaume Gomez under MIT license
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::{net::IpAddr, path::Path, sync::Arc};

/// Server configuration
#[derive(Clone)]
pub struct Config {
  pub server_port: u16,
  pub server_addr: IpAddr,
  pub users: Vec<User>,
  pub anonymous: bool,
  pub greeting: String,
  /// Maximum size of an uploaded file in bytes. An upload is held in memory.
  pub max_upload_size: u64,
  /// Maximum number of clients that are connected at the same time
  pub max_sessions: usize,
}

/// This callback provides filtering for specific FTP
/// commands, for example to inject a custom STOR handler.
///
/// `stor_file` is called for every STOR command received from the
/// client. Implementations may run blocking work — e.g. file I/O or
/// CPU-bound conversion — but should wrap such work in
/// [`tokio::task::spawn_blocking`] to avoid stalling the runtime.
pub trait FtpCallback {
  fn stor_file(&self, _path: &Path, _data: Arc<Vec<u8>>) -> impl std::future::Future<Output = std::io::Result<bool>> + Send {
    async { Ok(false) }
  }
}

#[derive(Clone, Debug)]
pub struct User {
  pub name: String,
  pub password: String,
}

impl Config {
  pub fn new<P: AsRef<Path>>(_file_path: P) -> Option<Config> {
    Some(Self {
      server_port: 8054,
      server_addr: "::1".parse().expect("Failed to parse IPv6 addr"),
      users: Vec::new(),
      anonymous: true,
      greeting: String::from("Welcome to this FTP server!"),
      max_upload_size: 2 * 1024 * 1024 * 1024,
      max_sessions: 32,
    })
  }
}
