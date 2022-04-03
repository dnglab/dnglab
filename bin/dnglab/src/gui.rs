// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use clap::ArgMatches;
//use log::debug;

pub async fn gui(_options: Option<&ArgMatches<'_>>) -> anyhow::Result<()> {
  println!("GUI is not available yet");
  Ok(())
}
