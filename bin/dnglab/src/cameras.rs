// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::collections::BTreeMap;

use clap::ArgMatches;
use itertools::Itertools;
use rawler::global_loader;

#[derive(Default, Clone)]
struct CameraRemarks {
  modes: Vec<String>,
  remarks: Vec<String>,
}

/// Print list of supported cameras
pub async fn cameras(options: &ArgMatches) -> anyhow::Result<()> {
  let cameras = global_loader().get_cameras();

  let mut map = BTreeMap::<String, BTreeMap<String, CameraRemarks>>::new();

  for key in cameras.keys() {
    let make = map.entry(key.0.clone()).or_default();
    let model = make.entry(key.1.clone()).or_default();
    if !key.2.is_empty() {
      model.modes.push(key.2.clone());
    }

    if let Some(cam) = cameras.get(key) {
      if let Some(remark) = &cam.remark {
        model.remarks.push(remark.clone());
      }
    }
  }

  let max_make = map.keys().map(|k| k.len()).max().unwrap_or(0);
  let max_model = map.values().flat_map(|v| v.keys()).map(|k| k.len()).max().unwrap_or(0);

  if options.is_present("markdown") {
    println!("# Supported cameras\n");

    println!("| Make  | Model        | State   | Modes     | Remarks   |");
    println!("|-------|--------------|---------|-----------|-----------|");

    for make in map {
      for model in make.1 {
        let modes = if model.1.modes.is_empty() {
          "all".into()
        } else {
          model.1.modes.iter().sorted().join(", ")
        };
        let remarks = model.1.remarks.iter().join(", ");

        println!("|{:max_make$}  | {:max_model$} | ✅ Yes | {} | {} |", make.0, model.0, modes, remarks);
      }
    }
    println!();
  } else {
    for make in map {
      println!("{:-<80}", "");
      println!("{}: ({} total)", make.0, make.1.len());
      for model in make.1 {
        let modes = if model.1.modes.is_empty() {
          "all".into()
        } else {
          model.1.modes.iter().sorted().join(", ")
        };
        let remarks = model.1.remarks.iter().join(", ");
        if remarks.is_empty() {
          println!("{:max_make$}  {:max_model$}  {}", make.0, model.0, modes);
        } else {
          println!("{:max_make$}  {:max_model$}  {}  ({})", make.0, model.0, modes, remarks);
        }
      }
      println!();
    }
  }
  Ok(())
}
