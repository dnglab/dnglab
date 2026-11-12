// SPDX-License-Identifier: MIT
// Originally written by Guillaume Gomez under MIT license
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::path::{Path, PathBuf};
use std::result;
use std::str::{self, FromStr, Utf8Error};
use std::string::FromUtf8Error;

/// Reason for the rejection of a command line
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParseError {
  /// Syntax error in parameters or arguments
  Syntax(&'static str),
  /// Command is not implemented for that parameter
  Parameter(&'static str),
  /// The command line is longer than the server accepts
  TooLong,
  /// The network protocol of EPRT is neither IPv4 nor IPv6
  NetworkProtocol,
}

impl From<Utf8Error> for ParseError {
  fn from(_: Utf8Error) -> Self {
    ParseError::Syntax("Parameter is not valid UTF-8")
  }
}

impl From<FromUtf8Error> for ParseError {
  fn from(_: FromUtf8Error) -> Self {
    ParseError::Syntax("Parameter is not valid UTF-8")
  }
}

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
  Eprt(u16),
  Pass(String),
  Pasv,
  Epsv(Option<String>),
  Pwd,
  Quit,
  Retr(PathBuf),
  Rmd(PathBuf),
  Dele(PathBuf),
  Size(PathBuf),
  Stor(PathBuf),
  Syst,
  Feat,
  Type(TransferType),
  /// MODE S, the stream mode
  Mode,
  /// STRU F, the file structure
  Stru,
  Allo,
  /// OPTS UTF8 ON
  OptsUtf8,
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
      Command::Eprt(_) => "EPRT",
      Command::Pwd => "PWD",
      Command::Feat => "FEAT",
      Command::Quit => "QUIT",
      Command::Retr(_) => "RETR",
      Command::Stor(_) => "STOR",
      Command::Syst => "SYST",
      Command::Type(_) => "TYPE",
      Command::Mode => "MODE",
      Command::Stru => "STRU",
      Command::Allo => "ALLO",
      Command::OptsUtf8 => "OPTS",
      Command::User(_) => "USER",
      Command::CdUp => "CDUP",
      Command::Mkd(_) => "MKD",
      Command::Rmd(_) => "RMD",
      Command::Dele(_) => "DELE",
      Command::Size(_) => "SIZE",
      Command::NoOp => "NOOP",
      Command::Unknown(_) => "UNKN", // doesn't exist
    }
  }
}

impl Command {
  pub fn new(input: Vec<u8>) -> result::Result<Self, ParseError> {
    let (command, data) = match input.iter().position(|&byte| byte == b' ') {
      Some(index) => {
        let (c, d) = input.split_at(index);
        (c.to_ascii_uppercase(), Ok(&d[1..]))
      }
      None => (input.to_ascii_uppercase(), Err(ParseError::Syntax("Missing parameter"))),
    };

    let command = match command.as_slice() {
      b"AUTH" => Command::Auth,
      b"CWD" => Command::Cwd(data.and_then(|bytes| Ok(Path::new(str::from_utf8(bytes)?).to_path_buf()))?),
      b"LIST" => Command::List(data.and_then(list_path).ok()),
      b"NLST" => Command::Nlst(data.and_then(list_path).ok()),
      b"PASV" => Command::Pasv,
      b"EPSV" => Command::Epsv(data.and_then(|bytes| Ok(Some(str::from_utf8(bytes)?.to_owned()))).unwrap_or(None)),
      b"PORT" => {
        let addr = data?
          .split(|&byte| byte == b',')
          .filter_map(|bytes| str::from_utf8(bytes).ok().and_then(|string| u8::from_str(string).ok()))
          .collect::<Vec<u8>>();
        if addr.len() != 6 {
          return Err(ParseError::Syntax("Invalid address/port"));
        }

        let port = ((addr[4] as u16) << 8) | (addr[5] as u16);
        Command::Port(data_port(port)?)
      }
      b"EPRT" => Command::Eprt(eprt_port(data?)?),
      b"PWD" => Command::Pwd,
      b"FEAT" => Command::Feat,
      b"QUIT" => Command::Quit,
      b"RETR" => Command::Retr(data.and_then(|bytes| Ok(Path::new(str::from_utf8(bytes)?).to_path_buf()))?),
      b"STOR" => Command::Stor(data.and_then(|bytes| Ok(Path::new(str::from_utf8(bytes)?).to_path_buf()))?),
      b"SYST" => Command::Syst,
      b"TYPE" => {
        let error = Err(ParseError::Parameter("Command not implemented for that parameter"));
        let data = data?;
        if data.is_empty() {
          return error;
        }
        match TransferType::from(data[0].to_ascii_uppercase()) {
          TransferType::Unknown => return error,
          typ => Command::Type(typ),
        }
      }
      // Stream mode and file structure are the only ones, as for most servers
      b"MODE" => match data? {
        [b'S' | b's'] => Command::Mode,
        _ => return Err(ParseError::Parameter("Only the stream mode is implemented")),
      },
      b"STRU" => match data? {
        [b'F' | b'f'] => Command::Stru,
        _ => return Err(ParseError::Parameter("Only the file structure is implemented")),
      },
      b"ALLO" => Command::Allo,
      b"OPTS" => {
        if data?.eq_ignore_ascii_case(b"UTF8 ON") {
          Command::OptsUtf8
        } else {
          return Err(ParseError::Syntax("Option not supported"));
        }
      }
      b"CDUP" => Command::CdUp,
      b"MKD" => Command::Mkd(data.and_then(|bytes| Ok(Path::new(str::from_utf8(bytes)?).to_path_buf()))?),
      b"RMD" => Command::Rmd(data.and_then(|bytes| Ok(Path::new(str::from_utf8(bytes)?).to_path_buf()))?),
      b"DELE" => Command::Dele(data.and_then(|bytes| Ok(Path::new(str::from_utf8(bytes)?).to_path_buf()))?),
      b"SIZE" => Command::Size(data.and_then(|bytes| Ok(Path::new(str::from_utf8(bytes)?).to_path_buf()))?),
      b"USER" => Command::User(data.and_then(|bytes| String::from_utf8(bytes.to_vec()).map_err(Into::into))?),
      b"PASS" => Command::Pass(data.and_then(|bytes| String::from_utf8(bytes.to_vec()).map_err(Into::into))?),
      b"NOOP" => Command::NoOp,
      s => Command::Unknown(str::from_utf8(s).unwrap_or("").to_owned()),
    };
    Ok(command)
  }
}

/// Check the port of the client for the data connection in active mode
fn data_port(port: u16) -> result::Result<u16, ParseError> {
  // TODO: check if the port isn't already used already by another connection...
  if port <= 1024 {
    return Err(ParseError::Syntax("Port must be greater than 1024"));
  }
  Ok(port)
}

/// Port of the EPRT parameter "|protocol|address|port|". Its first character is the
/// delimiter. The address is not used, the server connects to the address of the client.
fn eprt_port(bytes: &[u8]) -> result::Result<u16, ParseError> {
  let syntax = ParseError::Syntax("Invalid address/port");
  let param = str::from_utf8(bytes)?;
  let delimiter = param.chars().next().ok_or(syntax)?;
  // The delimiters at both ends give an empty first and last field
  let fields: Vec<&str> = param.split(delimiter).collect();
  let ["", protocol, _address, port, ""] = fields.as_slice() else {
    return Err(syntax);
  };
  if !matches!(*protocol, "1" | "2") {
    return Err(ParseError::NetworkProtocol);
  }
  data_port(port.parse().map_err(|_| syntax)?)
}

/// Path parameter of LIST and NLST. Some clients send options of "ls" like "-la"
/// before the path, they are skipped.
fn list_path(bytes: &[u8]) -> result::Result<PathBuf, ParseError> {
  let mut path = str::from_utf8(bytes)?;
  while path.starts_with('-') {
    path = path.split_once(' ').map_or("", |(_option, path)| path.trim_start());
  }
  Ok(PathBuf::from(path))
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
