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
}

/// This callback provides filtering for specific FTP
/// commands, for example to inject a custom STOR handler.
pub trait FtpCallback {
  fn stor_file(&self, _path: &Path, _data: Arc<Vec<u8>>) -> std::io::Result<bool> {
    Ok(false)
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
      users: vec![User {
        name: "anonymous".into(),
        password: "anonymous@example.com".into(),
      }],
      anonymous: true,
      greeting: String::from("Welcome to this FTP server!"),
    })
  }
}
