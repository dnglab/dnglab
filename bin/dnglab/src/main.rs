// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

mod analyze;
mod app;
mod cameras;
mod convert;
mod dnggen;
mod extract;
mod filemap;
mod ftpconv;
mod gui;
mod jobs;

use clap::AppSettings;
use fern::colors::{Color, ColoredLevelConfig};
use thiserror::Error;
//use log::debug;

const PKG_VERSION: &'static str = env!("CARGO_PKG_VERSION");
const PKG_NAME: &'static str = env!("CARGO_PKG_NAME");

/// Main entry function
///
/// We initialize the fern logger here, create a Clap command line
/// parser and check for the correct environment.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
  let app = app::create_app().setting(AppSettings::ArgRequiredElseHelp);
  let matches = app.get_matches_safe().unwrap_or_else(|e| e.exit());

  let colors = ColoredLevelConfig::new().debug(Color::Magenta);
  fern::Dispatch::new()
    .chain(std::io::stderr())
    //.level(log::LevelFilter::Debug)
    .level({
      match matches.occurrences_of("debug") {
        0 => log::LevelFilter::Error,
        1 => log::LevelFilter::Warn,
        2 => log::LevelFilter::Info,
        3 => log::LevelFilter::Debug,
        4 | _ => log::LevelFilter::Trace,
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
    ("analyze", Some(sc)) => analyze::analyze(sc).await,
    ("convert", Some(sc)) => convert::convert(sc).await,
    ("extract", Some(sc)) => extract::extract(sc).await,
    ("ftpconvert", Some(sc)) => ftpconv::ftpconvert(sc).await,
    ("cameras", Some(sc)) => cameras::cameras(sc).await,
    ("gui", sc) => gui::gui(sc).await,
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
