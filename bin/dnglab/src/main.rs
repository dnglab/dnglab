// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

mod analyze;
mod app;
mod convert;
mod dnggen;
mod extract;
mod filemap;
mod gui;

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
fn main() -> anyhow::Result<()> {
  let app = app::create_app().setting(AppSettings::ArgRequiredElseHelp);
  let matches = app.get_matches_safe().unwrap_or_else(|e| e.exit());

  let colors = ColoredLevelConfig::new().debug(Color::Magenta);
  fern::Dispatch::new()
    .chain(std::io::stderr())
    //.level(log::LevelFilter::Debug)
    .level({
      if matches.is_present("debug") {
        log::LevelFilter::Trace
      } else {
        log::LevelFilter::Warn
      }
    })
    .format(move |out, message, record| {
      out.finish(format_args!(
        "{}[{:6}][{}] {} ({}:{})",
        chrono::Utc::now().format("[%Y-%m-%d %H:%M:%S%z]"),
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
    ("analyze", Some(sc)) => analyze::analyze(sc),
    ("convert", Some(sc)) => convert::convert(sc),
    ("extract", Some(sc)) => extract::extract(sc),
    ("gui", sc) => gui::gui(sc),
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
  #[error("Source not exists: {}", _0)]
  NotExists(String),
  #[error("Destination already exists: {}", _0)]
  DestExists(String),
  #[error("Invalid format: {}", _0)]
  InvalidFormat(String),
  #[error("Decoder failed: {}", _0)]
  DecoderFail(String),
  #[error("General error: {}", _0)]
  General(String),
}

pub type Result<T> = std::result::Result<T, AppError>;
