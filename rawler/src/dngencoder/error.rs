// SPDX-License-Identifier: LGPL-2.1
// Copyright by image-tiff authors (see https://github.com/image-rs/image-tiff)
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::io;
use std::str;
use std::string;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DngError {
    #[error("I/O error while writing DNG")]
    Io(#[from] io::Error),

    #[error("Invalid value")]
    InvalidValue,

    /// An integer conversion to or from a platform size failed, either due to
    /// limits of the platform size or limits of the format.
    #[error("integer conversion to or from a platform size failed")]
    IntSize,
}

impl From<str::Utf8Error> for DngError {
    fn from(_err: str::Utf8Error) -> Self {
        Self::InvalidValue
    }
}

impl From<string::FromUtf8Error> for DngError {
    fn from(_err: string::FromUtf8Error) -> Self {
        Self::InvalidValue
    }
}

impl From<std::num::TryFromIntError> for DngError {
    fn from(_err: std::num::TryFromIntError) -> Self {
        Self::IntSize
    }
}

/// Result of an image encoding process
pub type DngResult<T> = Result<T, DngError>;
