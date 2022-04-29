use toml::Value;

use crate::imgop::xyz::FlatColorMatrix;
use crate::imgop::xyz::Illuminant;
use crate::CFA;

use std::collections::HashMap;

/// Contains sanitized information about the raw image's properties
#[derive(Debug, Clone, Default)]
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
  pub whitelevels: [u16; 4],
  pub blacklevels: [u16; 4],
  pub blackareah: Option<(usize, usize)>,
  pub blackareav: Option<(usize, usize)>,
  pub xyz_to_cam: [[f32; 3]; 4],
  pub color_matrix: HashMap<Illuminant, FlatColorMatrix>,
  pub cfa: CFA,
  // Active area relative to sensor size
  pub active_area: Option<[usize; 4]>,
  // Recommended area relative to sensor size
  pub crop_area: Option<[usize; 4]>,
  // Hint/Replacement for EXIF BITDEPTH info
  pub bps: usize,
  // The BPS of the output after decoding
  pub real_bps: usize,
  pub highres_width: usize,
  pub default_scale: [[u32; 2]; 2],
  pub best_quality_scale: [u32; 2],
  pub hints: Vec<String>,
  pub params: HashMap<String, Value>,
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
          let white = val.as_integer().unwrap_or_else(|| panic!("{} must be an integer", n)) as u16;
          self.whitelevels = [white, white, white, white];
        }
        n @ "blackpoint" => {
          let black = val.as_integer().unwrap_or_else(|| panic!("{} must be an integer", n)) as u16;
          self.blacklevels = [black, black, black, black];
        }
        n @ "blackareah" => {
          let vals = val.as_array().unwrap_or_else(|| panic!("{} must be an array", n));
          self.blackareah = Some((vals[0].as_integer().unwrap() as usize, vals[1].as_integer().unwrap() as usize));
        }
        n @ "blackareav" => {
          let vals = val.as_array().unwrap_or_else(|| panic!("{} must be an array", n));
          self.blackareav = Some((vals[0].as_integer().unwrap() as usize, vals[1].as_integer().unwrap() as usize));
        }
        "color_matrix" => {
          if let Some(color_matrix) = val.as_table() {
            for (illu_str, matrix) in color_matrix.into_iter() {
              let illu = Illuminant::new_from_str(illu_str).unwrap();
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
        n @ "active_area" => {
          let crop_vals = val.as_array().unwrap_or_else(|| panic!("{} must be an array", n));
          let mut crop = [0, 0, 0, 0];
          for (i, val) in crop_vals.iter().enumerate() {
            crop[i] = val.as_integer().unwrap() as usize;
          }
          self.active_area = Some(crop);
        }
        n @ "crop_area" => {
          let crop_vals = val.as_array().unwrap_or_else(|| panic!("{} must be an array", n));
          let mut crop = [0, 0, 0, 0];
          for (i, val) in crop_vals.iter().enumerate() {
            crop[i] = val.as_integer().unwrap() as usize;
          }
          self.crop_area = Some(crop);
        }
        n @ "color_pattern" => {
          self.cfa = CFA::new(val.as_str().unwrap_or_else(|| panic!("{} must be a string", n)));
        }
        n @ "bps" => {
          self.bps = val.as_integer().unwrap_or_else(|| panic!("{} must be an integer", n)) as usize;
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
          self.default_scale = scale;
        }
        n @ "best_quality_scale" => {
          let scale_vals = val.as_array().unwrap_or_else(|| panic!("{} must be an array", n));
          self.best_quality_scale = [
            scale_vals[0].as_integer().expect("must be integer") as u32,
            scale_vals[1].as_integer().expect("must be integer") as u32,
          ];
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
      whitelevels: [u16::MAX; 4],
      blacklevels: [0; 4],
      blackareah: None,
      blackareav: None,
      xyz_to_cam: [[0.0; 3]; 4],
      color_matrix: HashMap::new(),
      cfa: CFA::new(""),
      active_area: None,
      crop_area: None,
      bps: 0,
      real_bps: 16,
      highres_width: usize::max_value(),
      default_scale: [[1, 1], [1, 1]],
      best_quality_scale: [1, 1],
      hints: Vec::new(),
      params: HashMap::new(),
      //orientation: Orientation::Unknown,
    }
  }
}
