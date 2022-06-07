// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::collections::BTreeMap;

use clap::ArgMatches;
use rawler::lens::get_lenses;

/// Print list of supported lenses
pub async fn lenses(options: &ArgMatches) -> anyhow::Result<()> {
  let lenses = get_lenses();

  let mut map = BTreeMap::<(String, String), Vec<String>>::new();

  for lens in lenses.iter() {
    let mount = lens.mount.clone();
    let make = lens.lens_make.clone();
    map.entry((mount, make)).or_default().push(lens.lens_model.clone());
  }

  let max_mounts = map.keys().map(|k| k.0.len()).max().unwrap_or(0);
  let max_make = map.keys().map(|k| k.1.len()).max().unwrap_or(0);
  let max_model = map.values().flatten().map(|k| k.len()).max().unwrap_or(0);

  if options.is_present("markdown") {
    println!("# Supported lenses\n");

    println!("| Mount | Make  | Model        |");
    println!("|-------|-------|--------------|");

    for ((mount, make), models) in map {
      for model in models {
        println!("|{:max_mounts$}  |{:max_make$}  | {:max_model$} |", mount, make, model);
      }
    }
    println!();
  } else {
    for ((mount, make), models) in map {
      println!("{:-<80}", "");
      println!("{}/{}: ({} total)", mount, make, models.len());
      for model in models {
        println!("{:max_mounts$}  {:max_make$}  {}", mount, make, model);
      }
      println!();
    }
  }
  Ok(())
}
