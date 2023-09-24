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

use fern::colors::{Color, ColoredLevelConfig};
use thiserror::Error;
use tokio::runtime::Builder;
//use log::debug;

const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
const PKG_NAME: &str = env!("CARGO_PKG_NAME");
const STACK_SIZE_MIB: usize = 4;

fn main() -> anyhow::Result<()> {
  let runtime = Builder::new_multi_thread()
    .enable_all()
    .thread_name("dnglab-tokio-worker")
    .thread_stack_size(STACK_SIZE_MIB * 1024 * 1024)
    .build()
    .unwrap();

  runtime.block_on(main_async())
}

/// Main entry function
///
/// We initialize the fern logger here, create a Clap command line
/// parser and check for the correct environment.
async fn main_async() -> anyhow::Result<()> {
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
  #[error("Invalid arguments")]
  InvalidArgs,
  #[error("Invalid arguments: {}", _0)]
  InvalidCmdSwitch(String),
  #[error("I/O error: {}", _0)]
  Io(#[from] std::io::Error),
  #[error("Path not exists: {}", _0)]
  NotExists(String),
  #[error("Destination already exists: {}", _0)]
  DestExists(String),
  #[error("Invalid format: {}", _0)]
  InvalidFormat(String),
  #[error("Decoder failed: {}", _0)]
  DecoderFail(String),
  #[error("{}", _0)]
  General(String),
}

pub type Result<T> = std::result::Result<T, AppError>;
