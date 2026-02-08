use std::{net::AddrParseError, path::PathBuf};

use image::ImageError;
use rawler::{
  RawlerError,
  formats::{jfif::JfifError, tiff::TiffError},
};
use thiserror::Error;
//use log::debug;

pub mod analyze;
pub mod app;
pub mod cameras;
pub mod convert;
pub mod extract;
pub mod filemap;
pub mod ftpconv;
pub mod gui;
pub mod jobs;
pub mod lenses;
pub mod makedng;

const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
const PKG_NAME: &str = env!("CARGO_PKG_NAME");

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
  #[error("Unsupported file: {}", _0)]
  UnsupportedFile(String),
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
      RawlerError::Unsupported { .. } => Self::UnsupportedFile(value.to_string()),
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
