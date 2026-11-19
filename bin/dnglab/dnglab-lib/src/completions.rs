// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

use clap::ArgMatches;
use clap_complete::Shell;

use crate::{AppError, app::create_app};

/// Print the shell completion script
pub async fn completions(options: &ArgMatches) -> crate::Result<()> {
  let shell = *options
    .get_one::<Shell>("shell")
    .ok_or_else(|| AppError::InvalidCmdSwitch("Missing shell".into()))?;
  let mut cmd = create_app().name("dnglab");
  clap_complete::generate(shell, &mut cmd, "dnglab", &mut std::io::stdout());
  Ok(())
}
