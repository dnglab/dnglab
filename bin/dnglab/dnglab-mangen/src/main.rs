// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Write the man pages of dnglab and its subcommands into a directory:
//! `cargo run -p dnglab-mangen -- <dir>`

use std::path::{Path, PathBuf};

use clap_mangen::Man;

fn main() -> std::io::Result<()> {
  let Some(outdir) = std::env::args_os().nth(1).map(PathBuf::from) else {
    eprintln!("Usage: dnglab-mangen <output directory>");
    std::process::exit(1);
  };
  std::fs::create_dir_all(&outdir)?;
  build_manpages(&outdir)
}

fn build_manpages(outdir: &Path) -> std::io::Result<()> {
  let name = "dnglab";
  let cmd = dnglab_lib::app::create_app().name("dnglab");
  let man = Man::new(cmd.clone());
  let mut buffer: Vec<u8> = Default::default();
  man.render(&mut buffer)?;

  std::fs::write(outdir.join("dnglab.1"), buffer)?;

  for subcommand in cmd.get_subcommands() {
    let subcommand_name = subcommand.get_name();
    let subcommand_name = format!("{name}-{subcommand_name}");
    let mut buffer: Vec<u8> = Default::default();
    let man = Man::new(subcommand.clone().name(&subcommand_name));
    man.render(&mut buffer)?;
    std::fs::write(outdir.join(format!("{subcommand_name}.1")), buffer)?;
  }
  Ok(())
}
