// SPDX-License-Identifier: MIT
// Originally written by Guillaume Gomez under MIT license
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use crate::error::{Error, Result};
use std::path::{Path, PathBuf};
use std::str::{self, FromStr};

/// FTP commands and their arguments
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
  Auth,
  Cwd(PathBuf),
  List(Option<PathBuf>),
  Nlst(Option<PathBuf>),
  Mkd(PathBuf),
  NoOp,
  Port(u16),
  Pass(String),
  Pasv,
  Epsv(Option<String>),
  Pwd,
  Quit,
  Retr(PathBuf),
  Rmd(PathBuf),
  Dele(PathBuf),
  Stor(PathBuf),
  Syst,
  Feat,
  Type(TransferType),
  CdUp,
  Unknown(String),
  User(String),
}

impl AsRef<str> for Command {
  fn as_ref(&self) -> &str {
    match *self {
      Command::Auth => "AUTH",
      Command::Cwd(_) => "CWD",
      Command::List(_) => "LIST",
      Command::Nlst(_) => "NLST",
      Command::Pass(_) => "PASS",
      Command::Pasv => "PASV",
      Command::Epsv(_) => "EPSV",
      Command::Port(_) => "PORT",
      Command::Pwd => "PWD",
      Command::Feat => "FEAT",
      Command::Quit => "QUIT",
      Command::Retr(_) => "RETR",
      Command::Stor(_) => "STOR",
      Command::Syst => "SYST",
      Command::Type(_) => "TYPE",
      Command::User(_) => "USER",
      Command::CdUp => "CDUP",
      Command::Mkd(_) => "MKD",
      Command::Rmd(_) => "RMD",
      Command::Dele(_) => "DELE",
      Command::NoOp => "NOOP",
      Command::Unknown(_) => "UNKN", // doesn't exist
    }
  }
}

impl Command {
  pub fn new(input: Vec<u8>) -> Result<Self> {
    let (command, data) = match input.iter().position(|&byte| byte == b' ') {
      Some(index) => {
        let (c, d) = input.split_at(index);
        (c.to_ascii_uppercase(), Ok(&d[1..]))
      }
      None => (input.to_ascii_uppercase(), Err(Error::Msg("no command parameter".to_string()))),
    };

    let command = match command.as_slice() {
      b"AUTH" => Command::Auth,
      b"CWD" => Command::Cwd(data.and_then(|bytes| Ok(Path::new(str::from_utf8(bytes)?).to_path_buf()))?),
      b"LIST" => Command::List(data.and_then(|bytes| Ok(Path::new(str::from_utf8(bytes)?).to_path_buf())).ok()),
      b"NLST" => Command::Nlst(data.and_then(|bytes| Ok(Path::new(str::from_utf8(bytes)?).to_path_buf())).ok()),
      b"PASV" => Command::Pasv,
      b"EPSV" => Command::Epsv(data.and_then(|bytes| Ok(Some(str::from_utf8(bytes)?.to_owned()))).unwrap_or(None)),
      b"PORT" => {
        let addr = data?
          .split(|&byte| byte == b',')
          .filter_map(|bytes| str::from_utf8(bytes).ok().and_then(|string| u8::from_str(string).ok()))
          .collect::<Vec<u8>>();
        if addr.len() != 6 {
          return Err("Invalid address/port".into());
        }

        let port = (addr[4] as u16) << 8 | (addr[5] as u16);
        // TODO: check if the port isn't already used already by another connection...
        if port <= 1024 {
          return Err("Port can't be less than 10025".into());
        }
        Command::Port(port)
      }
      b"PWD" => Command::Pwd,
      b"FEAT" => Command::Feat,
      b"QUIT" => Command::Quit,
      b"RETR" => Command::Retr(data.and_then(|bytes| Ok(Path::new(str::from_utf8(bytes)?).to_path_buf()))?),
      b"STOR" => Command::Stor(data.and_then(|bytes| Ok(Path::new(str::from_utf8(bytes)?).to_path_buf()))?),
      b"SYST" => Command::Syst,
      b"TYPE" => {
        let error = Err("command not implemented for that parameter".into());
        let data = data?;
        if data.is_empty() {
          return error;
        }
        match TransferType::from(data[0]) {
          TransferType::Unknown => return error,
          typ => Command::Type(typ),
        }
      }
      b"CDUP" => Command::CdUp,
      b"MKD" => Command::Mkd(data.and_then(|bytes| Ok(Path::new(str::from_utf8(bytes)?).to_path_buf()))?),
      b"RMD" => Command::Rmd(data.and_then(|bytes| Ok(Path::new(str::from_utf8(bytes)?).to_path_buf()))?),
      b"DELE" => Command::Dele(data.and_then(|bytes| Ok(Path::new(str::from_utf8(bytes)?).to_path_buf()))?),
      b"USER" => Command::User(data.and_then(|bytes| String::from_utf8(bytes.to_vec()).map_err(Into::into))?),
      b"PASS" => Command::Pass(data.and_then(|bytes| String::from_utf8(bytes.to_vec()).map_err(Into::into))?),
      b"NOOP" => Command::NoOp,
      s => Command::Unknown(str::from_utf8(s).unwrap_or("").to_owned()),
    };
    Ok(command)
  }
}

fn _to_uppercase(data: &mut [u8]) {
  for byte in data {
    if *byte >= b'a' && *byte <= b'z' {
      *byte -= 32;
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransferType {
  Ascii,
  Image,
  Unknown,
}

impl From<u8> for TransferType {
  fn from(c: u8) -> TransferType {
    match c {
      b'A' => TransferType::Ascii,
      b'I' => TransferType::Image,
      _ => TransferType::Unknown,
    }
  }
}
