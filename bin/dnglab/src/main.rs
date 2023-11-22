// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

#![allow(
  clippy::expect_fun_call,
  clippy::or_fun_call,
  clippy::identity_op,
  clippy::let_and_return,
  clippy::if_same_then_else,
  clippy::eq_op,
  clippy::needless_range_loop,
  clippy::large_enum_variant
)]

mod analyze;
mod app;
mod cameras;
mod convert;
mod extract;
mod filemap;
mod ftpconv;
mod gui;
mod jobs;
mod lenses;
mod makedng;

use std::{
  net::AddrParseError,
  path::PathBuf,
  process::{ExitCode, Termination},
};

use fern::colors::{Color, ColoredLevelConfig};
use image::ImageError;
use rawler::{
  formats::{jfif::JfifError, tiff::TiffError},
  RawlerError,
};
use thiserror::Error;
use tokio::runtime::Builder;
//use log::debug;

const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
const PKG_NAME: &str = env!("CARGO_PKG_NAME");
const STACK_SIZE_MIB: usize = 4;

fn main() -> AppResult {
  let runtime = Builder::new_multi_thread()
    .enable_all()
    .thread_name("dnglab-tokio-worker")
    .thread_stack_size(STACK_SIZE_MIB * 1024 * 1024)
    .build()
    .unwrap();

  let result = runtime.block_on(main_async());
  if let Err(err) = &result {
    eprintln!("Error: {}", err);
  }
  AppResult(result)
}

/// Main entry function
///
/// We initialize the fern logger here, create a Clap command line
/// parser and check for the correct environment.
async fn main_async() -> crate::Result<()> {
  let app = app::create_app();
  let matches = app.try_get_matches().unwrap_or_else(|e| e.exit());

  let colors = ColoredLevelConfig::new().debug(Color::Magenta);
  fern::Dispatch::new()
    .chain(std::io::stderr())
    //.level(log::LevelFilter::Debug)
    .level({
      match matches.get_count("debug") {
        0 => log::LevelFilter::Error,
        1 => log::LevelFilter::Warn,
        2 => log::LevelFilter::Info,
        3 => log::LevelFilter::Debug,
        _ => log::LevelFilter::Trace,
      }
    })
    .format(move |out, message, record| {
      out.finish(format_args!(
        //"{}[{:6}][{}] {} ({}:{})",
        //chrono::Utc::now().format("[%Y-%m-%d %H:%M:%S%z]"),
        "[{:6}][{}] {} ({}:{})",
        colors.color(record.level()),
        record.target(),
        message,
        record.file().unwrap_or("<undefined>"),
        record.line().unwrap_or(0)
      ))
    })
    .apply()
    .expect("Invalid fern configuration, exiting");

  match matches.subcommand() {
    Some(("analyze", sc)) => analyze::analyze(sc).await,
    Some(("convert", sc)) => convert::convert(sc).await,
    Some(("makedng", sc)) => makedng::makedng(sc).await,
    Some(("extract", sc)) => extract::extract(sc).await,
    Some(("ftpserver", sc)) => ftpconv::ftpserver(sc).await,
    Some(("lenses", sc)) => lenses::lenses(sc).await,
    Some(("cameras", sc)) => cameras::cameras(sc).await,
    Some(("gui", sc)) => gui::gui(sc).await,
    _ => panic!("Unknown subcommand was used"),
  }
}

#[derive(Error, Debug)]
pub enum AppError {
  #[error("{}", _0)]
  General(String),
  #[error("Invalid arguments: {}", _0)]
  InvalidCmdSwitch(String),
  #[error("I/O error: {}", _0)]
  Io(#[from] std::io::Error),
  #[error("Not found: {}", _0.display())]
  NotFound(PathBuf),
  #[error("Already exists: {}", _0.display())]
  AlreadyExists(PathBuf),
  #[error("Decoder failed: {}", _0)]
  DecoderFailed(String),
  #[error(transparent)]
  Other(#[from] anyhow::Error),
}

impl From<serde_json::Error> for AppError {
  fn from(value: serde_json::Error) -> Self {
    anyhow::Error::new(value).into()
  }
}

impl From<serde_yaml::Error> for AppError {
  fn from(value: serde_yaml::Error) -> Self {
    anyhow::Error::new(value).into()
  }
}

impl From<AddrParseError> for AppError {
  fn from(value: AddrParseError) -> Self {
    anyhow::Error::new(value).into()
  }
}

impl From<RawlerError> for AppError {
  fn from(value: RawlerError) -> Self {
    match value {
      RawlerError::DecoderFailed(err) => Self::DecoderFailed(err),
      RawlerError::Unsupported { .. } => Self::General(value.to_string()),
    }
  }
}

impl From<ImageError> for AppError {
  fn from(value: ImageError) -> Self {
    anyhow::Error::new(value).into()
  }
}

impl From<JfifError> for AppError {
  fn from(value: JfifError) -> Self {
    anyhow::Error::new(value).into()
  }
}

impl From<TiffError> for AppError {
  fn from(value: TiffError) -> Self {
    anyhow::Error::new(value).into()
  }
}

pub type Result<T> = std::result::Result<T, AppError>;

pub struct AppResult(Result<()>);

impl Termination for AppResult {
  fn report(self) -> ExitCode {
    match self.0 {
      Ok(_) => ExitCode::SUCCESS,
      Err(AppError::InvalidCmdSwitch(_)) => ExitCode::from(1),
      Err(AppError::DecoderFailed(_)) => ExitCode::from(2),
      Err(AppError::General(_)) => ExitCode::from(3),
      Err(AppError::Io(_)) => ExitCode::from(4),
      Err(AppError::NotFound(_)) => ExitCode::from(5),
      Err(AppError::AlreadyExists(_)) => ExitCode::from(6),
      Err(AppError::Other(_)) => ExitCode::from(99),
    }
  }
}
