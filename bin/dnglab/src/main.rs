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

use std::process::{ExitCode, Termination};

use app::LogLevel;
use dnglab_lib::*;
use fern::colors::{Color, ColoredLevelConfig};
use tokio::runtime::Builder;
//use log::debug;

const STACK_SIZE_MIB: usize = 4;

fn main() -> AppResult {
  let runtime = Builder::new_multi_thread()
    .enable_all()
    .thread_name("dnglab-tokio-worker")
    .thread_stack_size(STACK_SIZE_MIB * 1024 * 1024)
    .build()
    .expect("Failed to build tokio runtime");

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
async fn main_async() -> dnglab_lib::Result<()> {
  // Override version and name, as we don't want these information from dnglab-lib but from this binary.
  let app = app::create_app().version(env!("CARGO_PKG_VERSION")).name(env!("CARGO_PKG_NAME"));
  let matches = app.try_get_matches().unwrap_or_else(|e| e.exit());

  let loglevel = match matches.get_one::<LogLevel>("loglevel").unwrap_or(&LogLevel::Warn) {
    LogLevel::Error => log::LevelFilter::Error,
    LogLevel::Warn => log::LevelFilter::Warn,
    LogLevel::Info => log::LevelFilter::Info,
    LogLevel::Debug => log::LevelFilter::Debug,
    LogLevel::Trace => log::LevelFilter::Trace,
  };

  let colors = ColoredLevelConfig::new().debug(Color::Magenta);
  fern::Dispatch::new()
    .chain(std::io::stderr())
    //.level(log::LevelFilter::Debug)
    .level(loglevel)
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

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn check_version() {
    assert_eq!(app::create_app().get_version(), Some(env!("CARGO_PKG_VERSION")));
  }
}
