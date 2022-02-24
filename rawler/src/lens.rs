// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::fmt::Display;

use lazy_static::lazy_static;
use log::debug;
use toml::Value;

use crate::{decoders::Camera, formats::tiff::Rational};

pub static LENSES_TOML: &'static str = include_str!("../data/lenses.toml");

const FAIL: &'static str = "Invalid lens database";

lazy_static! {
  static ref LENSES_DB: Vec<LensDescription> = build_lens_database().expect(FAIL);
}

/// Resolver for Lens information
#[derive(Default, Debug, Clone)]
pub struct LensResolver {
  /// Name of the lens model, if known
  lens_model: Option<String>,
  /// Name of the lens make, if known
  lens_make: Option<String>,
  /// Lens ID, if known
  lens_id: Option<LensId>,
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

#[allow(dead_code)]
struct LensMatcher<'a> {
  /// Name of the lens model, if known
  lens_model: Option<&'a str>,
  /// Name of the lens make, if known
  lens_make: Option<&'a str>,
  /// Lens ID, if known
  lens_id: Option<LensId>,
  /// Lens EXIF info, if known
  lens_info: Option<[Rational; 4]>,
  /// Camera make, if known
  camera_make: Option<&'a str>,
  /// Camera model, if known
  camera_model: Option<&'a str>,
  /// Focal lenth for taken photo
  #[allow(dead_code)]
  focal_len: Option<Rational>,
}

impl LensResolver {
  /// Create new empty LensResolver
  pub fn new() -> Self {
    Self::default()
  }

  pub fn with_camera(mut self, camera: &Camera) -> Self {
    self.camera_make = Some(camera.clean_make.clone());
    self.camera_model = Some(camera.clean_model.clone());
    self
  }

  pub fn with_lens_model<T: AsRef<str>>(mut self, lens_model: T) -> Self {
    self.lens_model = Some(lens_model.as_ref().into());
    self
  }

  pub fn with_lens_make<T: AsRef<str>>(mut self, lens_make: T) -> Self {
    self.lens_make = Some(lens_make.as_ref().into());
    self
  }

  pub fn with_lens_id(mut self, lens_id: LensId) -> Self {
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

  fn lens_matcher<'a>(&'a self) -> LensMatcher {
    LensMatcher {
      lens_model: self.lens_model.as_deref(),
      lens_make: self.lens_make.as_deref(),
      lens_id: self.lens_id.clone(),
      lens_info: self.lens_info.clone(),
      camera_make: self.camera_make.as_deref(),
      camera_model: self.camera_model.as_deref(),
      focal_len: self.focal_len.clone(),
    }
  }

  /// Resolve to a final LensDescription
  ///
  /// Returns None, if resolver was unable to find a lens.
  pub fn resolve(&self) -> Option<&'static LensDescription> {
    let first_try = self.resolve_internal();
    if first_try.is_some() {
      first_try
    } else {
      let second_try = match self.lens_matcher() {
        // Pentax *ist D and DS reports some lens as id=4 while it should be 7.
        LensMatcher {
          camera_model: Some("*ist DS"),
          lens_id: Some((4, subid)),
          ..
        }
        | LensMatcher {
          camera_model: Some("*ist D"),
          lens_id: Some((4, subid)),
          ..
        } => self.clone().with_lens_id((7, subid)).resolve_internal(),
        _ => None,
      };
      if second_try.is_none() {
        eprintln!(
          "Unknown lens id: '{}' for camera model {}. Please open an issue at https://github.com/dnglab/dnglab/issues and provide the RAW file",
          self, self.camera_model.as_ref().unwrap_or(&"<unset>".to_string())
        );
      }
      second_try
    }
  }

  /// Resolve the lens internally.
  fn resolve_internal(&self) -> Option<&'static LensDescription> {
    if let Some(lens_model) = self.lens_model.as_ref().filter(|s| !s.is_empty()) {
      debug!("Lens model: {}", lens_model);
      if let Some(db_entry) = LENSES_DB.iter().find(|entry| entry.identifiers.name == Some(lens_model.into())) {
        return Some(db_entry);
      }
    } else if let Some(lens_id) = self.lens_id.as_ref() {
      let matches: Vec<&LensDescription> = LENSES_DB
        .iter()
        .filter(|entry| entry.identifiers.id.is_some() && entry.identifiers.id == Some(lens_id.to_owned()))
        .collect();
      match matches.len() {
        1 => return Some(matches[0]),
        c if c > 1 => return Some(matches[0]), // TODO: fixme
        _ => {
          debug!("Lens not found"); // TODO
        }
      }
    } else {
      debug!("Lens information is empty");
    }
    /*     eprintln!(
      "Unknown lens id: '{}'. Please open an issue at https://github.com/dnglab/dnglab/issues and provide the RAW file",
      self
    ); */
    None
  }
}

impl Display for LensResolver {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    if let Some(id) = &self.lens_id {
      f.write_fmt(format_args!("ID: {}:{}", id.0, id.1))?;
    }
    if let Some(name) = &self.lens_model {
      f.write_fmt(format_args!("Model: {}", name))?;
    }
    Ok(())
  }
}

pub type LensId = (u32, u32);

#[derive(Debug)]
pub struct LensIdentifier {
  pub name: Option<String>,
  pub id: Option<LensId>,
}

impl LensIdentifier {
  pub(crate) fn new(name: Option<String>, id: Option<LensId>) -> Self {
    if name.is_some() || id.is_some() {
      Self { name, id }
    } else {
      panic!("LensIdentifier must contain a name or id");
    }
  }
}

/// Description of a lens
#[derive(Debug)]
pub struct LensDescription {
  /// Identifiers
  pub identifiers: LensIdentifier,
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
fn build_lens_database() -> Option<Vec<LensDescription>> {
  let toml = match LENSES_TOML.parse::<Value>() {
    Ok(val) => val,
    Err(e) => panic!("{}", format!("Error parsing lenses.toml: {:?}", e)),
  };

  let mut lenses = Vec::new();

  for lens in toml.get("lenses")?.as_array()? {
    //let key = lens.get("key")?.as_str()?.into();
    let id_name = lens.get("key").and_then(Value::as_str).map(String::from);
    let id_val1 = lens.get("lens_id").and_then(Value::as_integer).map(|v| v as u32);
    let id_val2 = lens.get("lens_subid").and_then(Value::as_integer).map(|v| v as u32);
    let id_id = id_val1.map(|id| (id, id_val2.unwrap_or(0)));
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
    lenses.push(LensDescription {
      identifiers: LensIdentifier::new(id_name, id_id),
      lens_name: lens_name.unwrap_or(&format!("{} {}", lens_make, lens_model)).into(),
      lens_make,
      lens_model,
      focal_range: [focal_range[0], focal_range[1]],
      aperture_range: [aperture_range[0], aperture_range[1]],
    });
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
