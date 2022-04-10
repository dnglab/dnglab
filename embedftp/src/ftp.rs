// SPDX-License-Identifier: MIT
// Originally written by Guillaume Gomez under MIT license
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

/// Encapsulation of an answer response to the client.
#[derive(Debug)]
pub struct Answer {
  pub code: ResultCode,
  pub message: String,
  pub lines: Vec<String>,
}

impl Answer {
  /// Construct a new anwser
  pub fn new(code: ResultCode, message: &str) -> Self {
    Answer {
      code,
      message: message.to_string(),
      lines: Vec::default(),
    }
  }

  pub fn new_multiline(code: ResultCode, desc: &str, lines: &[String]) -> Self {
    Answer {
      code,
      message: desc.to_string(),
      lines: Vec::from(lines),
    }
  }
}

/// FTP status codes
#[derive(Debug, Clone, Copy)]
#[repr(u32)]
#[allow(dead_code)]
pub enum ResultCode {
  RestartMarkerReply = 110,
  ServiceReadInXXXMinutes = 120,
  DataConnectionAlreadyOpen = 125,
  FileStatusOk = 150,
  Ok = 200,
  CommandNotImplementedSuperfluousAtThisSite = 202,
  SystemStatus = 211,
  DirectoryStatus = 212,
  FileStatus = 213,
  HelpMessage = 214,
  SystemType = 215,
  ServiceReadyForNewUser = 220,
  ServiceClosingControlConnection = 221,
  DataConnectionOpen = 225,
  ClosingDataConnection = 226,
  EnteringPassiveMode = 227,
  ExtendedEnteringPassiveMode = 229,
  UserLoggedIn = 230,
  RequestedFileActionOkay = 250,
  PATHNAMECreated = 257,
  UserNameOkayNeedPassword = 331,
  NeedAccountForLogin = 332,
  RequestedFileActionPendingFurtherInformation = 350,
  ServiceNotAvailable = 421,
  CantOpenDataConnection = 425,
  ConnectionClosed = 426,
  FileBusy = 450,
  LocalErrorInProcessing = 451,
  InsufficientStorageSpace = 452,
  UnknownCommand = 500,
  InvalidParameterOrArgument = 501,
  CommandNotImplemented = 502,
  BadSequenceOfCommands = 503,
  CommandNotImplementedForThatParameter = 504,
  NotLoggedIn = 530,
  NeedAccountForStoringFiles = 532,
  FileNotFound = 550,
  PageTypeUnknown = 551,
  ExceededStorageAllocation = 552,
  FileNameNotAllowed = 553,
}
