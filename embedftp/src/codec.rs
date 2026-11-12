// SPDX-License-Identifier: MIT
// Originally written by Guillaume Gomez under MIT license
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use crate::command::{Command, ParseError};
use crate::ftp::Answer;
use bytes::BytesMut;
use std::io::{self, Write};
use tokio_util::codec::{Decoder, Encoder};

/// Longest command line that is accepted, without the CRLF
const MAX_LINE_LENGTH: usize = 8192;

/// Codec for FTP commands
#[derive(Default)]
pub struct FtpCodec {
  /// Bytes at the start of the buffer that contain no CRLF
  searched: usize,
  /// The line is too long: it was answered, its bytes are dropped up to the CRLF.
  discarding: bool,
}

impl Decoder for FtpCodec {
  /// A line that is no valid command is an item too, the client gets a reply for it.
  type Item = Result<Command, ParseError>;
  type Error = io::Error;

  fn decode(&mut self, buf: &mut BytesMut) -> io::Result<Option<Self::Item>> {
    loop {
      // The last searched byte may be the CR of a CRLF
      let start = self.searched.saturating_sub(1);
      if let Some(index) = find_crlf(&buf[start..]).map(|pos| start + pos) {
        let line = buf.split_to(index);
        let _ = buf.split_to(2); // Remove \r\n.
        self.searched = 0;
        if self.discarding {
          self.discarding = false;
        } else if line.len() > MAX_LINE_LENGTH {
          return Ok(Some(Err(ParseError::TooLong)));
        } else {
          return Ok(Some(Command::new(line.to_vec())));
        }
      } else if buf.len() > MAX_LINE_LENGTH {
        // Drop the bytes of the line, but not a CR at the end
        let cr = usize::from(buf.ends_with(b"\r"));
        let _ = buf.split_to(buf.len() - cr);
        self.searched = 0;
        if self.discarding {
          return Ok(None);
        }
        self.discarding = true;
        return Ok(Some(Err(ParseError::TooLong)));
      } else {
        self.searched = buf.len();
        return Ok(None);
      }
    }
  }
}

impl Encoder<Answer> for FtpCodec {
  type Error = io::Error;

  fn encode(&mut self, answer: Answer, buf: &mut BytesMut) -> io::Result<()> {
    let mut buffer = vec![];
    if !answer.lines.is_empty() {
      write!(buffer, "{}-{}\r\n", answer.code as u32, answer.message)?;
      // The space keeps a line apart from a reply code. The feature lines of FEAT require it.
      for line in answer.lines {
        write!(buffer, " {}\r\n", line)?;
      }
      write!(buffer, "{} End\r\n", answer.code as u32)?;
    } else if answer.message.is_empty() {
      write!(buffer, "{}\r\n", answer.code as u32)?;
    } else {
      write!(buffer, "{} {}\r\n", answer.code as u32, answer.message)?;
    }
    buf.extend(&buffer);
    Ok(())
  }
}

fn find_crlf(buf: &[u8]) -> Option<usize> {
  buf.windows(2).position(|bytes| bytes == b"\r\n")
}

#[cfg(test)]
mod tests {
  use std::path::PathBuf;

  use super::{Answer, BytesMut, Command, Decoder, Encoder, FtpCodec};
  use crate::ftp::ResultCode;

  #[test]
  fn test_encoder() {
    let mut codec = FtpCodec::default();
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
    let mut codec = FtpCodec::default();
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
    assert_eq!(command, Some(Ok(Command::Pwd)));

    let mut buf = BytesMut::new();
    buf.extend(b"LIST /tmp\r\n");
    let result = codec.decode(&mut buf);
    assert!(result.is_ok());
    let command = result.expect("Codec failed");
    assert_eq!(command, Some(Ok(Command::List(Some(PathBuf::from("/tmp"))))));
  }
}
