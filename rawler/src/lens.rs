// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::fmt::Display;

use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use toml::Value;

use crate::{decoders::Camera, formats::tiff::Rational};

pub static LENSES_TOML: &str = include_str!(concat!(env!("OUT_DIR"), "/lenses.toml"));

const FAIL: &str = "Invalid lens database";

lazy_static! {
  static ref LENSES_DB: Vec<LensDescription> = build_lens_database().expect(FAIL);
}

pub fn get_lenses() -> &'static Vec<LensDescription> {
  &LENSES_DB
}

/// Resolver for Lens information
#[derive(Default, Debug, Clone)]
pub struct LensResolver {
  /// Unique lens keyname, if known
  lens_keyname: Option<String>,
  /// Name of the lens make, if known
  lens_make: Option<String>,
  /// Name of the lens model, if known
  lens_model: Option<String>,
  /// Lens ID, if known
  lens_id: Option<LensId>,
  /// Nikon ID
  nikon_id: Option<String>,
  /// Olympus ID
  olympus_id: Option<String>,
  /// Lens EXIF info, if known
  lens_info: Option<[Rational; 4]>,
  /// Camera make, if known
  camera_make: Option<String>,
  /// Camera model, if known
  camera_model: Option<String>,
  /// Mounts, if known
  mounts: Option<Vec<String>>,
  /// Focal lenth for taken photo
  focal_len: Option<Rational>,
  /// Aperture for taken photo
  aperture: Option<Rational>,
}

#[allow(dead_code)]
struct LensMatcher<'a> {
  /// Name of the lens model, if known
  lens_name: Option<&'a str>,
  /// Name of the lens make, if known
  lens_make: Option<&'a str>,
  /// Lens ID, if known
  lens_id: Option<LensId>,
  /// Nikon ID
  nikon_id: Option<String>,
  /// Olympus ID
  olympus_id: Option<String>,
  /// Lens EXIF info, if known
  lens_info: Option<[Rational; 4]>,
  /// Camera make, if known
  camera_make: Option<&'a str>,
  /// Camera model, if known
  camera_model: Option<&'a str>,
  /// Mounts, if known
  mounts: Option<&'a [String]>,
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
    // For cameras with fixed lens, an optional camera param can be specified
    // which is the key into the lens database.
    if let Some(key) = camera.param_str("fixed_lens_key").map(String::from) {
      self.lens_keyname = Some(key);
    }
    self
  }

  pub fn with_lens_keyname<T: AsRef<str>>(mut self, lens_name: Option<T>) -> Self {
    self.lens_keyname = lens_name.map(|v| v.as_ref().into());
    self
  }

  pub fn with_lens_make<T: AsRef<str>>(mut self, lens_make: Option<T>) -> Self {
    self.lens_make = lens_make.map(|v| v.as_ref().into());
    self
  }

  pub fn with_lens_model<T: AsRef<str>>(mut self, lens_model: Option<T>) -> Self {
    self.lens_model = lens_model.map(|v| v.as_ref().into());
    self
  }

  pub fn with_mounts(mut self, mounts: &[String]) -> Self {
    self.mounts = Some(mounts.to_vec());
    self
  }

  pub fn with_lens_id(mut self, lens_id: LensId) -> Self {
    self.lens_id = Some(lens_id);
    self
  }

  pub fn with_nikon_id(mut self, nikon_id: Option<String>) -> Self {
    self.nikon_id = nikon_id;
    self
  }

  pub fn with_olympus_id(mut self, olympus_id: Option<String>) -> Self {
    self.olympus_id = olympus_id;
    self
  }

  pub fn with_focal_len(mut self, focal_len: Option<Rational>) -> Self {
    self.focal_len = focal_len;
    self
  }

  pub fn with_aperture(mut self, aperture: Option<Rational>) -> Self {
    self.aperture = aperture;
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

  fn lens_matcher(&self) -> LensMatcher<'_> {
    LensMatcher {
      lens_name: self.lens_keyname.as_deref(),
      lens_make: self.lens_make.as_deref(),
      lens_id: self.lens_id,
      nikon_id: self.nikon_id.clone(),
      olympus_id: self.olympus_id.clone(),
      lens_info: self.lens_info,
      camera_make: self.camera_make.as_deref(),
      camera_model: self.camera_model.as_deref(),
      mounts: self.mounts.as_deref(),
      focal_len: self.focal_len,
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
        log::warn!("No lens definition found in database, search parameters: {}. {}", self, crate::ISSUE_HINT);
        if std::env::var("RAWLER_FAIL_NO_LENS").ok().map(|val| val == "1").unwrap_or(false) {
          panic!("No lens definition found in database, search parameters: {}.", self);
        }
      }
      second_try
    }
  }

  /// Resolve the lens internally.
  fn resolve_internal(&self) -> Option<&'static LensDescription> {
    // First try, if we have an exact name, we use just this
    if let Some(name) = self.lens_keyname.as_ref().filter(|s| !s.is_empty()) {
      if let Some(db_entry) = LENSES_DB.iter().find(|entry| entry.identifiers.name == Some(name.into())) {
        return Some(db_entry);
      }
    }

    // Nikon lens IDs are special, try this next
    if let Some(nikon_id) = &self.nikon_id {
      if let Some(db_entry) = LENSES_DB.iter().find(|entry| entry.identifiers.nikon_id == Some(nikon_id.clone())) {
        return Some(db_entry);
      }
    }

    // Olympus lens IDs are special, try this next
    if let Some(olympus_id) = &self.olympus_id {
      if let Some(db_entry) = LENSES_DB.iter().find(|entry| entry.identifiers.olympus_id == Some(olympus_id.clone())) {
        return Some(db_entry);
      }
    }

    // If we have a lens id (common) then we can filter as much as possible

    let matches: Vec<&LensDescription> = LENSES_DB
      .iter()
      .filter(|entry| self.mounts.as_ref().is_none_or(|mounts| mounts.contains(&entry.mount)))
      .filter(|entry| {
        self
          .lens_id
          .as_ref()
          .is_none_or(|id| entry.identifiers.id.as_ref().is_some_and(|entry_id| *entry_id == *id))
      })
      .filter(|entry| self.lens_make.as_ref().is_none_or(|make| entry.lens_make == *make))
      .filter(|entry| self.lens_model.as_ref().is_none_or(|model| entry.lens_model == *model))
      .filter(|entry| {
        self
          .focal_len
          .as_ref()
          .is_none_or(|focal| *focal >= entry.focal_range[0] && *focal <= entry.focal_range[1])
      })
      .filter(|entry| {
        self
          .aperture
          .as_ref()
          .is_none_or(|ap| *ap >= entry.aperture_range[0] || *ap <= entry.aperture_range[1])
      })
      .collect();
    match matches.len() {
      1 => return Some(matches[0]),
      c if c > 1 => {
        log::warn!(
          "Found multiple ({}) lens definitions, unable to determine which lens to use. {}",
          c,
          crate::ISSUE_HINT
        );
        for lens in matches {
          log::warn!("Possible lens: {} {}", lens.lens_make, lens.lens_model);
        }
      }
      _ => {}
    }

    None
  }
}

impl Display for LensResolver {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let mut s = Vec::new();
    if let Some(mount) = &self.mounts {
      s.push(format!("Mounts: {:?}", mount));
    }
    if let Some(id) = &self.lens_id {
      s.push(format!("ID: '{}:{}'", id.0, id.1));
    }
    if let Some(name) = &self.lens_keyname {
      s.push(format!("Keyname: '{}'", name));
    }
    if let Some(name) = &self.lens_make {
      s.push(format!("Make: '{}", name));
    }
    if let Some(name) = &self.lens_model {
      s.push(format!("Model: '{}'", name));
    }
    if let Some(name) = &self.focal_len {
      s.push(format!("Focal len: '{}'", name));
    }
    if s.is_empty() { f.write_str("<EMPTY>") } else { f.write_str(&s.join(", ")) }
  }
}

pub type LensId = (u32, u32);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LensIdentifier {
  pub name: Option<String>,
  pub id: Option<LensId>,
  pub nikon_id: Option<String>,
  pub olympus_id: Option<String>,
}

impl LensIdentifier {
  pub(crate) fn new(name: Option<String>, id: Option<LensId>, nikon_id: Option<String>, olympus_id: Option<String>) -> Self {
    if name.is_some() || id.is_some() || nikon_id.is_some() || olympus_id.is_some() {
      Self {
        name,
        id,
        nikon_id,
        olympus_id,
      }
    } else {
      panic!("LensIdentifier must contain a name or id");
    }
  }
}

/// Description of a lens
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LensDescription {
  /// Identifiers
  pub identifiers: LensIdentifier,
  /// Lens mount
  pub mount: String,
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
    let nikon_id = lens.get("nikon_id").and_then(Value::as_str).map(String::from);
    let olympus_id = lens.get("olympus_id").and_then(Value::as_str).map(String::from);
    let mount = lens.get("mount").and_then(|val| val.as_str()).expect(FAIL);
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
      identifiers: LensIdentifier::new(id_name, id_id, nikon_id, olympus_id),
      lens_name: lens_name.unwrap_or(&format!("{} {}", lens_make, lens_model)).into(),
      mount: String::from(mount),
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
    let resolver = LensResolver::new()
      .with_lens_make(Some("Canon"))
      .with_lens_keyname(Some("RF15-35mm F2.8 L IS USM"));
    let lens = resolver.resolve();
    assert!(lens.is_some());
    assert_eq!(lens.expect("No lens").lens_make, "Canon");
    assert_eq!(lens.expect("No lens").lens_model, "RF 15-35mm F2.8L IS USM");
    assert_eq!(lens.expect("No lens").lens_name, "Canon RF 15-35mm F2.8L IS USM");
    Ok(())
  }
}
