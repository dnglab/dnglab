// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::fmt::Debug;

pub mod extractraw;
pub mod raw2dng;

pub trait Job: Clone + Debug + Send {
  type Output: Debug + Send;

  /// Execute the job
  fn execute(&self) -> Self::Output;
}
