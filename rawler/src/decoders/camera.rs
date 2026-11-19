use serde::Deserialize;
use serde::Serialize;
use toml::Value;

use crate::CFA;
use crate::RawlerError;
use crate::Result;
use crate::cfa::PlaneColor;
use crate::imgop::matrix::FlatMatrix;
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
  /// DNG UniqueCameraModel, `None` to build it from the clean make and model
  pub unique_camera_model: Option<String>,
  /// DNG LocalizedCameraModel, the market name of an alias of the camera
  pub localized_camera_model: Option<String>,
  pub remark: Option<String>,
  pub filesize: usize,
  pub raw_width: usize,
  pub raw_height: usize,
  //pub orientation: Orientation,
  pub whitelevel: Option<Vec<u32>>,
  pub blacklevel: Option<Vec<u32>>,
  pub blackareah: Option<(usize, usize)>,
  pub blackareav: Option<(usize, usize)>,
  pub color_matrix: HashMap<Illuminant, FlatMatrix>,
  /// Reduction matrices (3 x color planes) for cameras with more than three color planes
  pub reduction_matrix: HashMap<Illuminant, FlatMatrix>,
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

impl DefaultScale {
  /// Pixel aspect ratio (width / height), 1.0 for an invalid scale
  pub fn pixel_aspect_ratio(&self) -> f64 {
    let [[h_n, h_d], [v_n, v_d]] = self.0;
    let aspect = (h_n as f64 / h_d as f64) / (v_n as f64 / v_d as f64);
    if aspect.is_finite() && aspect > 0.0 { aspect } else { 1.0 }
  }

  /// Scale multiplied by the integer factors `(horizontal, vertical)`
  pub fn scaled(&self, (horizontal, vertical): (usize, usize)) -> Self {
    let [[h_n, h_d], [v_n, v_d]] = self.0;
    Self([[h_n * horizontal as u32, h_d], [v_n * vertical as u32, v_d]])
  }

  /// Scale divided by the integer factors `(horizontal, vertical)`
  pub fn unscaled(&self, (horizontal, vertical): (usize, usize)) -> Self {
    let divide = |[n, d]: [u32; 2], factor: u32| if n % factor == 0 { [n / factor, d] } else { [n, d * factor] };
    Self([divide(self.0[0], horizontal as u32), divide(self.0[1], vertical as u32)])
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BestQualityScale(pub [u32; 2]);

impl Default for BestQualityScale {
  fn default() -> Self {
    Self([1, 1])
  }
}

/// Value of a fraction, 1.0 for an invalid fraction
fn scale_factor([numerator, denominator]: [u32; 2]) -> f64 {
  let factor = numerator as f64 / denominator as f64;
  if factor.is_finite() && factor > 0.0 { factor } else { 1.0 }
}

impl Camera {
  pub fn find_hint(&self, hint: &str) -> bool {
    self.hints.contains(&hint.to_string())
  }

  /// Horizontal and vertical scale from the DefaultCrop to the image with
  /// square pixels of the best quality: DefaultScale * BestQualityScale
  pub fn best_quality_scale_factors(&self) -> (f64, f64) {
    let [horizontal, vertical] = self.default_scale.0.map(scale_factor);
    let best = scale_factor(self.best_quality_scale.0);
    (horizontal * best, vertical * best)
  }

  pub fn param_usize(&self, name: &str) -> Option<usize> {
    self.params.get(name).and_then(|p| p.as_integer()).and_then(|i| usize::try_from(i).ok())
  }

  pub fn param_i32(&self, name: &str) -> Option<i32> {
    self.params.get(name).and_then(|p| p.as_integer()).and_then(|i| i32::try_from(i).ok())
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
    // For messages about invalid values: the keys are sorted, the model may follow them
    let model = ct.get("model").and_then(|model| model.as_str()).unwrap_or(&self.model).to_string();
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
          self.whitelevel = Some(vec![toml_int(&model, n, val)]);
        }
        n @ "blackpoint" => {
          self.blacklevel = Some(vec![toml_int(&model, n, val)]);
        }
        n @ "blackareah" => {
          let [start, len] = toml_ints(&model, n, val);
          self.blackareah = Some((start, len));
        }
        n @ "blackareav" => {
          let [start, len] = toml_ints(&model, n, val);
          self.blackareav = Some((start, len));
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
              let matrix: FlatMatrix = matrix
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
          self.active_area = Some(toml_ints(&model, n, val));
        }
        n @ "crop_area" => {
          self.crop_area = Some(toml_ints(&model, n, val));
        }
        n @ "color_pattern" => {
          self.cfa = CFA::new(val.as_str().unwrap_or_else(|| panic!("{} must be a string", n)));
        }
        n @ "plane_color" => {
          self.plane_color = PlaneColor::new(val.as_str().unwrap_or_else(|| panic!("{} must be a string", n)));
        }
        n @ "bps" => {
          self.bps = Some(toml_int(&model, n, val));
        }
        n @ "real_bps" => {
          self.real_bps = toml_int(&model, n, val);
        }
        n @ "filesize" => {
          self.filesize = toml_int(&model, n, val);
        }
        n @ "raw_width" => {
          self.raw_width = toml_int(&model, n, val);
        }
        n @ "raw_height" => {
          self.raw_height = toml_int(&model, n, val);
        }
        n @ "highres_width" => {
          self.highres_width = toml_int(&model, n, val);
        }
        n @ "default_scale" => {
          let [horizontal, vertical] = toml_array(&model, n, val);
          self.default_scale = DefaultScale([toml_ints(&model, n, horizontal), toml_ints(&model, n, vertical)]);
        }
        n @ "best_quality_scale" => {
          self.best_quality_scale = BestQualityScale(toml_ints(&model, n, val));
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
      unique_camera_model: None,
      localized_camera_model: None,
      remark: None,
      filesize: 0,
      raw_width: 0,
      raw_height: 0,
      whitelevel: None,
      blacklevel: None,
      blackareah: None,
      blackareav: None,
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
    }
  }
}

/// Integer of the key `name` in the definition of camera `model`
///
/// # Panics
/// Panics if the value isn't an integer in the range of `T`
fn toml_int<T: TryFrom<i64>>(model: &str, name: &str, val: &Value) -> T {
  val.as_integer().and_then(|int| T::try_from(int).ok()).unwrap_or_else(|| {
    panic!(
      "{}: {} must be an integer in the range of {}, not {}",
      model,
      name,
      std::any::type_name::<T>(),
      val
    )
  })
}

/// Array of `N` values of the key `name` in the definition of camera `model`
///
/// # Panics
/// Panics if the value isn't an array of `N` values
fn toml_array<'a, const N: usize>(model: &str, name: &str, val: &'a Value) -> [&'a Value; N] {
  let vals = val
    .as_array()
    .filter(|vals| vals.len() == N)
    .unwrap_or_else(|| panic!("{}: {} must be an array of {} values, not {}", model, name, N, val));
  std::array::from_fn(|i| &vals[i])
}

/// Array of `N` integers of the key `name` in the definition of camera `model`
///
/// # Panics
/// Panics if the value isn't an array of `N` integers in the range of `T`
fn toml_ints<T: TryFrom<i64>, const N: usize>(model: &str, name: &str, val: &Value) -> [T; N] {
  toml_array::<N>(model, name, val).map(|val| toml_int(model, name, val))
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

  #[test]
  fn integers_from_toml() {
    let camera = camera_from_toml(
      "model = \"M\"\nwhitepoint = 4095\ncrop_area = [1, 2, 3, 4]\nblackareav = [0, 8]\ndefault_scale = [[2, 1], [1, 1]]\nbest_quality_scale = [3, 2]\n",
    );
    assert_eq!(camera.whitelevel, Some(vec![4095]));
    assert_eq!(camera.crop_area, Some([1, 2, 3, 4]));
    assert_eq!(camera.blackareav, Some((0, 8)));
    assert_eq!(camera.default_scale, DefaultScale([[2, 1], [1, 1]]));
    assert_eq!(camera.best_quality_scale, BestQualityScale([3, 2]));
  }

  #[test]
  #[should_panic(expected = "M: crop_area must be an array of 4 values")]
  fn areas_need_four_values() {
    camera_from_toml("model = \"M\"\ncrop_area = [1, 2, 3]\n");
  }

  #[test]
  #[should_panic(expected = "M: active_area must be an integer in the range of usize, not -2")]
  fn areas_need_positive_values() {
    camera_from_toml("model = \"M\"\nactive_area = [0, -2, 0, 0]\n");
  }

  #[test]
  #[should_panic(expected = "M: whitepoint must be an integer in the range of u32")]
  fn levels_need_positive_values() {
    camera_from_toml("model = \"M\"\nwhitepoint = -1\n");
  }

  #[test]
  #[should_panic(expected = "M: default_scale must be an array of 2 values")]
  fn scales_need_two_values() {
    camera_from_toml("model = \"M\"\ndefault_scale = [[1, 1]]\n");
  }

  #[test]
  fn params_in_range() {
    let camera = camera_from_toml("[params]\nnegative = -4\nlarge = 4294967296\n");
    assert_eq!(camera.param_usize("negative"), None);
    assert_eq!(camera.param_i32("negative"), Some(-4));
    assert_eq!(camera.param_i32("large"), None);
    assert_eq!(camera.param_usize("large"), Some(4294967296));
  }
}
