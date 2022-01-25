// SPDX-License-Identifier: MIT
// Originally written by Guillaume Gomez under MIT license
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use self::Error::*;
use std::error;
use std::fmt::{self, Display, Formatter};
use std::io;
use std::result;
use std::str::Utf8Error;
use std::string::FromUtf8Error;

/// Generic Result type for library
pub type Result<T> = result::Result<T, Error>;

/// Possible errors for server
#[derive(Debug)]
pub enum Error {
  FromUtf8(FromUtf8Error),
  Io(io::Error),
  Msg(String),
  Utf8(Utf8Error),
}

impl Error {
  pub fn to_io_error(self) -> io::Error {
    match self {
      Io(error) => error,
      FromUtf8(_) | Msg(_) | Utf8(_) => io::ErrorKind::Other.into(),
    }
  }
}

impl Display for Error {
  fn fmt(&self, formatter: &mut Formatter) -> fmt::Result {
    match *self {
      FromUtf8(ref error) => error.fmt(formatter),
      Io(ref error) => error.fmt(formatter),
      Utf8(ref error) => error.fmt(formatter),
      Msg(ref msg) => write!(formatter, "{}", msg),
    }
  }
}

impl error::Error for Error {
  fn cause(&self) -> Option<&dyn error::Error> {
    let cause: &dyn error::Error = match *self {
      FromUtf8(ref error) => error,
      Io(ref error) => error,
      Utf8(ref error) => error,
      Msg(_) => return None,
    };
    Some(cause)
  }
}

impl From<io::Error> for Error {
  fn from(error: io::Error) -> Self {
    Io(error)
  }
}

impl<'a> From<&'a str> for Error {
  fn from(message: &'a str) -> Self {
    Msg(message.to_string())
  }
}

impl From<Utf8Error> for Error {
  fn from(error: Utf8Error) -> Self {
    Utf8(error)
  }
}

impl From<FromUtf8Error> for Error {
  fn from(error: FromUtf8Error) -> Self {
    FromUtf8(error)
  }
}
