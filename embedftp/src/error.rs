// SPDX-License-Identifier: MIT
// Originally written by Guillaume Gomez under MIT license
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use self::Error::*;
use std::error;
use std::fmt::{self, Display, Formatter};
use std::io;
use std::result;

/// Generic Result type for library
pub type Result<T> = result::Result<T, Error>;

/// Possible errors for server
#[derive(Debug)]
pub enum Error {
  Io(io::Error),
  Msg(String),
}

impl Display for Error {
  fn fmt(&self, formatter: &mut Formatter) -> fmt::Result {
    match *self {
      Io(ref error) => error.fmt(formatter),
      Msg(ref msg) => write!(formatter, "{}", msg),
    }
  }
}

impl error::Error for Error {
  fn cause(&self) -> Option<&dyn error::Error> {
    let cause: &dyn error::Error = match *self {
      Io(ref error) => error,
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
