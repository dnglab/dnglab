// SPDX-License-Identifier: MIT
// Originally written by Guillaume Gomez under MIT license
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use crate::command::Command;
use crate::error::Error;
use crate::ftp::Answer;
use bytes::BytesMut;
use std::io::{self, Write};
use tokio_util::codec::{Decoder, Encoder};

/// Codec for FTP commands
pub struct FtpCodec;

impl Decoder for FtpCodec {
  type Item = Command;
  type Error = io::Error;

  fn decode(&mut self, buf: &mut BytesMut) -> io::Result<Option<Command>> {
    if let Some(index) = find_crlf(buf) {
      let line = buf.split_to(index);
      let _ = buf.split_to(2); // Remove \r\n.
      Command::new(line.to_vec()).map(Some).map_err(Error::to_io_error)
    } else {
      Ok(None)
    }
  }
}

impl Encoder<Answer> for FtpCodec {
  type Error = io::Error;

  fn encode(&mut self, answer: Answer, buf: &mut BytesMut) -> io::Result<()> {
    let mut buffer = vec![];
    if !answer.lines.is_empty() {
      write!(buffer, "{}- {}\r\n", answer.code as u32, answer.message)?;
      for line in answer.lines {
        if let Some(true) = line.chars().next().map(|c| c.is_ascii_digit()) {
          write!(buffer, " {}\r\n", line)?;
        } else {
          write!(buffer, "{}\r\n", line)?;
        }
      }
      write!(buffer, "{} end\r\n", answer.code as u32)?;
    } else if answer.message.is_empty() {
      write!(buffer, "{}\r\n", answer.code as u32)?;
    } else {
      write!(buffer, "{} {}\r\n", answer.code as u32, answer.message)?;
    }
    buf.extend(&buffer);
    Ok(())
  }
}

fn find_crlf(buf: &mut BytesMut) -> Option<usize> {
  buf.windows(2).position(|bytes| bytes == b"\r\n")
}

#[cfg(test)]
mod tests {
  use std::path::PathBuf;

  use super::{Answer, BytesMut, Command, Decoder, Encoder, FtpCodec};
  use crate::ftp::ResultCode;

  #[test]
  fn test_encoder() {
    let mut codec = FtpCodec;
    let message = "bad sequence of commands";
    let answer = Answer::new(ResultCode::BadSequenceOfCommands, message);
    let mut buf = BytesMut::new();
    let result = codec.encode(answer, &mut buf);
    assert!(result.is_ok());
    assert_eq!(buf, format!("503 {}\r\n", message));

    let answer = Answer::new(ResultCode::CantOpenDataConnection, "");
    let mut buf = BytesMut::new();
    let result = codec.encode(answer, &mut buf);
    assert!(result.is_ok(), "Result is ok");
    assert_eq!(buf, "425\r\n".to_string(), "Buffer contains 425");
  }

  #[test]
  fn test_decoder() {
    let mut codec = FtpCodec;
    let mut buf = BytesMut::new();
    buf.extend(b"PWD");
    let result = codec.decode(&mut buf);
    assert!(result.is_ok());
    let command = result.expect("Codec failed");
    assert!(command.is_none());

    buf.extend(b"\r\n");
    let result = codec.decode(&mut buf);
    assert!(result.is_ok());
    let command = result.expect("Codec failed");
    assert_eq!(command, Some(Command::Pwd));

    let mut buf = BytesMut::new();
    buf.extend(b"LIST /tmp\r\n");
    let result = codec.decode(&mut buf);
    assert!(result.is_ok());
    let command = result.expect("Codec failed");
    assert_eq!(command, Some(Command::List(Some(PathBuf::from("/tmp")))));
  }
}
