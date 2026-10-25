use serde::Deserialize;
use serde::Serialize;
use toml::Value;

use crate::CFA;
use crate::RawlerError;
use crate::Result;
use crate::cfa::PlaneColor;
use crate::imgop::xyz::FlatColorMatrix;
use crate::imgop::xyz::Illuminant;

use std::collections::HashMap;

use super::BlackLevel;
use super::WhiteLevel;

/// Contains sanitized information about the raw image's properties
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Camera {
  pub make: String,
  pub model: String,
  pub mode: String,
  pub clean_make: String,
  pub clean_model: String,
  pub remark: Option<String>,
  pub filesize: usize,
  pub raw_width: usize,
  pub raw_height: usize,
  //pub orientation: Orientation,
  pub whitelevel: Option<Vec<u32>>,
  pub blacklevel: Option<Vec<u32>>,
  pub blackareah: Option<(usize, usize)>,
  pub blackareav: Option<(usize, usize)>,
  pub xyz_to_cam: [[f32; 3]; 4],
  pub color_matrix: HashMap<Illuminant, FlatColorMatrix>,
  /// Reduction matrices (3 x color planes) for cameras with more than three color planes
  pub reduction_matrix: HashMap<Illuminant, FlatColorMatrix>,
  pub cfa: CFA,
  pub plane_color: PlaneColor,
  // Active area relative to sensor size
  pub active_area: Option<[usize; 4]>,
  // Recommended area relative to sensor size
  pub crop_area: Option<[usize; 4]>,
  // Hint/Replacement for EXIF BITDEPTH info
  pub bps: Option<usize>,
  // The BPS of the output after decoding
  pub real_bps: usize,
  pub highres_width: usize,
  pub default_scale: DefaultScale,
  pub best_quality_scale: BestQualityScale,
  pub hints: Vec<String>,
  pub params: HashMap<String, Value>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DefaultScale(pub [[u32; 2]; 2]);

impl Default for DefaultScale {
  fn default() -> Self {
    Self([[1, 1], [1, 1]])
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BestQualityScale(pub [u32; 2]);

impl Default for BestQualityScale {
  fn default() -> Self {
    Self([1, 1])
  }
}

impl Camera {
  pub fn find_hint(&self, hint: &str) -> bool {
    self.hints.contains(&hint.to_string())
  }

  pub fn param_usize(&self, name: &str) -> Option<usize> {
    self.params.get(name).and_then(|p| p.as_integer()).map(|i| i as usize)
  }

  pub fn param_i32(&self, name: &str) -> Option<i32> {
    self.params.get(name).and_then(|p| p.as_integer()).map(|i| i as i32)
  }

  pub fn param_str(&self, name: &str) -> Option<&str> {
    self.params.get(name).and_then(|p| p.as_str())
  }

  pub fn make_blacklevel(&self, cpp: usize) -> Option<BlackLevel> {
    self.blacklevel.as_ref().map(|x| {
      if x.len() == 1 {
        BlackLevel::new(&vec![x[0]; cpp], 1, 1, cpp)
      } else if x.len() == self.cfa.width * self.cfa.height * cpp {
        BlackLevel::new(x, self.cfa.width, self.cfa.height, cpp)
      } else {
        panic!("Invalid blacklevel data")
      }
    })
  }

  pub fn make_whitelevel(&self, cpp: usize) -> Option<WhiteLevel> {
    self.whitelevel.as_ref().map(|x| {
      if x.len() == 1 {
        WhiteLevel(vec![x[0] as u32; cpp])
      } else if x.len() == cpp {
        WhiteLevel(x.clone())
      } else {
        panic!("Invalid whitelevel data")
      }
    })
  }

  pub fn update_from_toml(&mut self, ct: &toml::value::Table) {
    for (name, val) in ct {
      match name.as_ref() {
        n @ "make" => {
          self.make = val.as_str().unwrap_or_else(|| panic!("{} must be a string", n)).to_string();
        }
        n @ "model" => {
          self.model = val.as_str().unwrap_or_else(|| panic!("{} must be a string", n)).to_string();
        }
        n @ "mode" => {
          self.mode = val.as_str().unwrap_or_else(|| panic!("{} must be a string", n)).to_string();
        }
        n @ "clean_make" => {
          self.clean_make = val.as_str().unwrap_or_else(|| panic!("{} must be a string", n)).to_string();
        }
        n @ "clean_model" => {
          self.clean_model = val.as_str().unwrap_or_else(|| panic!("{} must be a string", n)).to_string();
        }
        n @ "remark" => {
          self.remark = Some(val.as_str().unwrap_or_else(|| panic!("{} must be a string", n)).to_string());
        }
        n @ "whitepoint" => {
          let white = val.as_integer().unwrap_or_else(|| panic!("{} must be an integer", n)) as u32;
          self.whitelevel = Some(vec![white]);
        }
        n @ "blackpoint" => {
          let black = val.as_integer().unwrap_or_else(|| panic!("{} must be an integer", n)) as u32;
          self.blacklevel = Some(vec![black]);
        }
        n @ "blackareah" => {
          let vals = val.as_array().unwrap_or_else(|| panic!("{} must be an array", n));
          self.blackareah = Some((
            vals[0].as_integer().expect("blackareah[0] must be an integer") as usize,
            vals[1].as_integer().expect("blackareah[1] must be an integer") as usize,
          ));
        }
        n @ "blackareav" => {
          let vals = val.as_array().unwrap_or_else(|| panic!("{} must be an array", n));
          self.blackareav = Some((
            vals[0].as_integer().expect("blackareav[0] must be an integer") as usize,
            vals[1].as_integer().expect("blackareav[1] must be an integer") as usize,
          ));
        }
        "color_matrix" => {
          if let Some(color_matrix) = val.as_table() {
            for (illu_str, matrix) in color_matrix.into_iter() {
              let illu = Illuminant::new_from_str(illu_str).expect("invalid illuminant string in color_matrix");
              let xyz_to_cam = matrix
                .as_array()
                .expect("color matrix must be array")
                .iter()
                .map(|a| a.as_float().expect("color matrix values must be float") as f32)
                .collect();
              self.color_matrix.insert(illu, xyz_to_cam);
            }
          } else {
            eprintln!("Invalid matrix spec for {}", self.clean_model);
          }
          assert!(!self.color_matrix.is_empty());
        }
        "reduction_matrix" => {
          if let Some(reduction_matrix) = val.as_table() {
            for (illu_str, matrix) in reduction_matrix.into_iter() {
              let illu = Illuminant::new_from_str(illu_str).expect("invalid illuminant string in reduction_matrix");
              let matrix: FlatColorMatrix = matrix
                .as_array()
                .expect("reduction matrix must be array")
                .iter()
                .map(|a| a.as_float().expect("reduction matrix values must be float") as f32)
                .collect();
              assert!(
                matrix.len().is_multiple_of(3) && matrix.len() > 9,
                "reduction matrix for {} must have 3 rows and more than 3 columns",
                self.clean_model
              );
              self.reduction_matrix.insert(illu, matrix);
            }
          } else {
            eprintln!("Invalid reduction matrix spec for {}", self.clean_model);
          }
        }
        n @ "active_area" => {
          let crop_vals = val.as_array().unwrap_or_else(|| panic!("{} must be an array", n));
          let mut crop = [0, 0, 0, 0];
          for (i, val) in crop_vals.iter().enumerate() {
            crop[i] = val.as_integer().expect("active_area values must be integers") as usize;
          }
          self.active_area = Some(crop);
        }
        n @ "crop_area" => {
          let crop_vals = val.as_array().unwrap_or_else(|| panic!("{} must be an array", n));
          let mut crop = [0, 0, 0, 0];
          for (i, val) in crop_vals.iter().enumerate() {
            crop[i] = val.as_integer().expect("crop_area values must be integers") as usize;
          }
          self.crop_area = Some(crop);
        }
        n @ "color_pattern" => {
          self.cfa = CFA::new(val.as_str().unwrap_or_else(|| panic!("{} must be a string", n)));
        }
        n @ "plane_color" => {
          self.plane_color = PlaneColor::new(val.as_str().unwrap_or_else(|| panic!("{} must be a string", n)));
        }
        n @ "bps" => {
          self.bps = Some(val.as_integer().unwrap_or_else(|| panic!("{} must be an integer", n)) as usize);
        }
        n @ "real_bps" => {
          self.real_bps = val.as_integer().unwrap_or_else(|| panic!("{} must be an integer", n)) as usize;
        }
        n @ "filesize" => {
          self.filesize = val.as_integer().unwrap_or_else(|| panic!("{} must be an integer", n)) as usize;
        }
        n @ "raw_width" => {
          self.raw_width = val.as_integer().unwrap_or_else(|| panic!("{} must be an integer", n)) as usize;
        }
        n @ "raw_height" => {
          self.raw_height = val.as_integer().unwrap_or_else(|| panic!("{} must be an integer", n)) as usize;
        }
        n @ "highres_width" => {
          self.highres_width = val.as_integer().unwrap_or_else(|| panic!("{} must be an integer", n)) as usize;
        }
        n @ "default_scale" => {
          let scale_vals = val.as_array().unwrap_or_else(|| panic!("{} must be an array", n));
          let scale_h = scale_vals[0].as_array().expect("must be array");
          let scale_v = scale_vals[1].as_array().expect("must be array");
          let scale = [
            [
              scale_h[0].as_integer().expect("must be integer") as u32,
              scale_h[1].as_integer().expect("must be integer") as u32,
            ],
            [
              scale_v[0].as_integer().expect("must be integer") as u32,
              scale_v[1].as_integer().expect("must be integer") as u32,
            ],
          ];
          self.default_scale = DefaultScale(scale);
        }
        n @ "best_quality_scale" => {
          let scale_vals = val.as_array().unwrap_or_else(|| panic!("{} must be an array", n));
          self.best_quality_scale = BestQualityScale([
            scale_vals[0].as_integer().expect("must be integer") as u32,
            scale_vals[1].as_integer().expect("must be integer") as u32,
          ]);
        }
        n @ "hints" => {
          self.hints = Vec::new();
          for hint in val.as_array().unwrap_or_else(|| panic!("{} must be an array", n)) {
            self.hints.push(hint.as_str().expect("hints must be a string").to_string());
          }
        }
        n @ "params" => {
          for (name, val) in val.as_table().unwrap_or_else(|| panic!("{} must be a table", n)) {
            self.params.insert(name.clone(), val.clone());
          }
        }
        "model_aliases" => {}
        "modes" => {} // ignore
        key => {
          panic!("Unknown key: {}", key);
        }
      }
    }
  }

  pub fn new() -> Camera {
    Camera {
      make: "".to_string(),
      model: "".to_string(),
      mode: "".to_string(),
      clean_make: "".to_string(),
      clean_model: "".to_string(),
      remark: None,
      filesize: 0,
      raw_width: 0,
      raw_height: 0,
      whitelevel: None,
      blacklevel: None,
      blackareah: None,
      blackareav: None,
      xyz_to_cam: [[0.0; 3]; 4],
      color_matrix: HashMap::new(),
      reduction_matrix: HashMap::new(),
      cfa: CFA::new(""),
      plane_color: PlaneColor::default(),
      active_area: None,
      crop_area: None,
      bps: None,
      real_bps: 16,
      highres_width: usize::MAX,
      default_scale: DefaultScale::default(),
      best_quality_scale: BestQualityScale::default(),
      hints: Vec::new(),
      params: HashMap::new(),
      //orientation: Orientation::Unknown,
    }
  }
}

/// Camera definition of a decoder.
///
/// Decoders are created for cameras missing in the camera database, too, so metadata
/// and embedded previews stay available. Such cameras get a generic definition with
/// make and model from the file, and raw decoding fails with `RawlerError::Unsupported`.
#[derive(Debug, Clone)]
pub(crate) struct CameraLookup {
  camera: Camera,
  supported: bool,
}

impl CameraLookup {
  pub(crate) fn new_supported(camera: Camera) -> Self {
    Self { camera, supported: true }
  }

  pub(crate) fn new_unknown(camera: Camera) -> Self {
    Self { camera, supported: false }
  }

  /// Camera definition for raw decoding, fails for cameras missing in the camera database
  pub(crate) fn supported(&self) -> Result<&Camera> {
    if self.supported {
      Ok(&self.camera)
    } else {
      Err(RawlerError::unsupported(&self.camera, "Unknown camera"))
    }
  }

  /// Camera definition for metadata (clean make/model and lens lookup).
  /// For unknown cameras, this is the generic definition.
  pub(crate) fn metadata(&self) -> &Camera {
    &self.camera
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const REDUCTION: [f32; 12] = [0.9, 0.1, 0.0, 0.0, 0.0, 0.5, 0.0, 0.5, 0.0, 0.0, 1.0, 0.0];

  fn camera_from_toml(content: &str) -> Camera {
    let table: toml::value::Table = toml::from_str(content).expect("valid TOML");
    let mut camera = Camera::new();
    camera.update_from_toml(&table);
    camera
  }

  #[test]
  fn reduction_matrix_from_toml() {
    let camera = camera_from_toml(
      "[reduction_matrix]\nD65 = [0.9, 0.1, 0.0, 0.0, 0.0, 0.5, 0.0, 0.5, 0.0, 0.0, 1.0, 0.0]\nA = [0.9, 0.1, 0.0, 0.0, 0.0, 0.5, 0.0, 0.5, 0.0, 0.0, 1.0, 0.0]\n",
    );
    assert_eq!(camera.reduction_matrix.len(), 2);
    assert_eq!(camera.reduction_matrix.get(&Illuminant::D65), Some(&REDUCTION.to_vec()));
    assert_eq!(camera.reduction_matrix.get(&Illuminant::A), Some(&REDUCTION.to_vec()));
  }

  #[test]
  #[should_panic(expected = "must have 3 rows and more than 3 columns")]
  fn reduction_matrix_needs_more_than_three_columns() {
    camera_from_toml("[reduction_matrix]\nD65 = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]\n");
  }
}
