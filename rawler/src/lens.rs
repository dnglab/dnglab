// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use lazy_static::lazy_static;
use log::debug;
use std::collections::HashMap;
use toml::Value;

use crate::formats::tiff::Rational;

pub static LENSES_TOML: &'static str = include_str!("../data/lenses.toml");

const FAIL: &'static str = "Invalid lens database";

lazy_static! {
  static ref LENSES_DB: HashMap<String, LensDescription> = build_lens_database().expect(FAIL);
}

/// Resolver for Lens information
#[derive(Default, Debug)]
pub struct LensResolver {
  /// Name of the lens model, if known
  lens_model: Option<String>,
  /// Name of the lens make, if known
  lens_make: Option<String>,
  /// Lens ID, if known
  lens_id: Option<u16>,
  /// Lens EXIF info, if known
  lens_info: Option<[Rational; 4]>,
  /// Camera make, if known
  camera_make: Option<String>,
  /// Camera model, if known
  camera_model: Option<String>,
  /// Focal lenth for taken photo
  #[allow(dead_code)]
  focal_len: Option<Rational>,
}

impl LensResolver {
  /// Create new empty LensResolver
  pub fn new() -> Self {
    Self::default()
  }

  pub fn with_lens_model<T: AsRef<str>>(mut self, lens_model: T) -> Self {
    self.lens_model = Some(lens_model.as_ref().into());
    self
  }

  pub fn with_lens_make<T: AsRef<str>>(mut self, lens_make: T) -> Self {
    self.lens_make = Some(lens_make.as_ref().into());
    self
  }

  pub fn with_lens_id(mut self, lens_id: u16) -> Self {
    self.lens_id = Some(lens_id);
    self
  }

  pub fn with_lens_info(mut self, lens_info: [Rational; 4]) -> Self {
    self.lens_info = Some(lens_info);
    self
  }

  pub fn with_camera_make<T: AsRef<str>>(mut self, camera_make: T) -> Self {
    self.camera_make = Some(camera_make.as_ref().into());
    self
  }

  pub fn with_camera_model<T: AsRef<str>>(mut self, camera_model: T) -> Self {
    self.camera_model = Some(camera_model.as_ref().into());
    self
  }

  /// Resolve to a final LensDescription
  ///
  /// Returns None, if resolver was unable to find a lens.
  pub fn resolve(self) -> Option<&'static LensDescription> {
    if let Some(lens_model) = self.lens_model.as_ref().filter(|s| !s.is_empty()) {
      debug!("Lens model: {}", lens_model);
      if let Some(db_entry) = LENSES_DB.get(lens_model) {
        return Some(db_entry);
      } else {
        eprintln!(
          "Unknown lens model: '{}'. Please open an issue at https://github.com/dnglab/dnglab/issues and provide the RAW file",
          lens_model
        );
        return None;
      }
    } else {
      debug!("Lens information is empty");
    }
    None
  }
}

/// Description of a lens
#[derive(Debug)]
pub struct LensDescription {
  /// Lens make
  pub lens_make: String,
  /// Lens model (without make)
  pub lens_model: String,
  /// Focal range (min, max)
  pub focal_range: [Rational; 2],
  /// Aperture range (for min focal and max focal)
  pub aperture_range: [Rational; 2],
  /// Full qualified model name (with make)
  pub lens_name: String,
}

/// Internal function to parse and build global lens database
fn build_lens_database() -> Option<HashMap<String, LensDescription>> {
  let toml = match LENSES_TOML.parse::<Value>() {
    Ok(val) => val,
    Err(e) => panic!("{}", format!("Error parsing lenses.toml: {:?}", e)),
  };

  let mut lenses = HashMap::new();

  for lens in toml.get("lenses")?.as_array()? {
    let key = lens.get("key")?.as_str()?.into();
    let lens_make = lens.get("make")?.as_str()?.into();
    let lens_model = lens.get("model")?.as_str()?.into();
    let focal_range: Vec<Rational> = lens
      .get("focal_range")?
      .as_array()?
      .iter()
      .map(|v| {
        let v = v.as_array().expect(FAIL);
        let r1 = v.get(0).expect("Invalid lens database").as_integer().expect(FAIL) as u32;
        let r2 = v.get(1).expect(FAIL).as_integer().expect(FAIL) as u32;
        Rational::new(r1, r2)
      })
      .collect();
    let aperture_range: Vec<Rational> = lens
      .get("aperture_range")?
      .as_array()?
      .iter()
      .map(|v| {
        let v = v.as_array().expect(FAIL);
        let r1 = v.get(0).expect(FAIL).as_integer().expect(FAIL) as u32;
        let r2 = v.get(1).expect(FAIL).as_integer().expect(FAIL) as u32;
        Rational::new(r1, r2)
      })
      .collect();
    let lens_name = lens.get("name").map(|s| s.as_str().expect(FAIL));
    lenses.insert(
      key,
      LensDescription {
        lens_name: lens_name.unwrap_or(&format!("{} {}", lens_make, lens_model)).into(),
        lens_make,
        lens_model,
        focal_range: [focal_range[0], focal_range[1]],
        aperture_range: [aperture_range[0], aperture_range[1]],
      },
    );
  }
  Some(lenses)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn resolve_single_lens() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let resolver = LensResolver::new().with_lens_make("Canon").with_lens_model("RF15-35mm F2.8 L IS USM");
    let lens = resolver.resolve();
    assert!(lens.is_some());
    assert_eq!(lens.expect("No lens").lens_make, "Canon");
    assert_eq!(lens.expect("No lens").lens_model, "RF 15-35mm F2.8L IS USM");
    assert_eq!(lens.expect("No lens").lens_name, "Canon RF 15-35mm F2.8L IS USM");
    Ok(())
  }
}
