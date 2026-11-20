// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::collections::BTreeMap;

use clap::ArgMatches;
use itertools::Itertools;
use rawler::decoders::UnsupportedReason;
use rawler::global_loader;

#[derive(Default, Clone)]
struct CameraRemarks {
  modes: Vec<String>,
  remarks: Vec<String>,
  /// Number of modes, including the base camera
  entries: usize,
  /// Modes marked as unsupported, with the reason
  unsupported: Vec<(String, UnsupportedReason)>,
}

impl CameraRemarks {
  /// All modes of the camera are marked as unsupported
  fn is_unsupported(&self) -> bool {
    self.unsupported.len() == self.entries
  }

  /// Reasons of a camera marked as unsupported, or `None` if some mode is supported
  fn unsupported_reasons(&self) -> Option<String> {
    self
      .is_unsupported()
      .then(|| self.unsupported.iter().map(|(_, reason)| reason.to_string()).sorted().dedup().join(", "))
  }

  /// Remarks, followed by the unsupported modes of a partially supported camera
  fn remarks(&self) -> String {
    let mut remarks = self.remarks.clone();
    if !self.is_unsupported() {
      for (mode, reason) in self.unsupported.iter().sorted_by(|a, b| a.0.cmp(&b.0)) {
        let mode = if mode.is_empty() { "base mode" } else { mode };
        remarks.push(format!("{} unsupported ({})", mode, reason));
      }
    }
    remarks.join(", ")
  }
}

/// Print list of supported cameras
pub async fn cameras(options: &ArgMatches) -> crate::Result<()> {
  let cameras = global_loader().get_cameras();

  let mut map = BTreeMap::<String, BTreeMap<String, CameraRemarks>>::new();

  for key in cameras.keys() {
    let make = map.entry(key.0.clone()).or_default();
    let model = make.entry(key.1.clone()).or_default();
    if !key.2.is_empty() {
      model.modes.push(key.2.clone());
    }

    model.entries += 1;

    if let Some(cam) = cameras.get(key) {
      if let Some(remark) = &cam.remark {
        model.remarks.push(remark.clone());
      }
      if let Some(reason) = cam.unsupported {
        model.unsupported.push((key.2.clone(), reason));
      }
    }
  }

  let max_make = map.keys().map(|k| k.len()).max().unwrap_or(0);
  let max_model = map.values().flat_map(|v| v.keys()).map(|k| k.len()).max().unwrap_or(0);

  if options.get_flag("markdown") {
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
        let remarks = model.1.remarks();
        let state = if let Some(reasons) = model.1.unsupported_reasons() {
          format!("❌ No ({})", reasons)
        } else if model.1.unsupported.is_empty() {
          "✅ Yes".to_string()
        } else {
          "⚠️ Partial".to_string()
        };

        println!("|{:max_make$}  | {:max_model$} | {} | {} | {} |", make.0, model.0, state, modes, remarks);
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
        let remarks = match model.1.unsupported_reasons() {
          Some(reasons) => [format!("unsupported: {}", reasons), model.1.remarks()]
            .into_iter()
            .filter(|remark| !remark.is_empty())
            .join(", "),
          None => model.1.remarks(),
        };
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
