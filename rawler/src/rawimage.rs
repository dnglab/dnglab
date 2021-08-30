use std::collections::HashMap;

use log::debug;

use crate::{CFA, decoders::*, imgop::{Dim2, Point, Rect, raw::{ColorMatrix, DevelopParams}, sensor::bayer::BayerPattern, xyz::{FlatColorMatrix, Illuminant}}};

/// All the data needed to process this raw image, including the image data itself as well
/// as all the needed metadata
#[derive(Debug, Clone)]
pub struct RawImage {
  /// camera make as encoded in the file
  pub make: String,
  /// camera model as encoded in the file
  pub model: String,
  /// make cleaned up to be consistent and short
  pub clean_make: String,
  /// model cleaned up to be consistent and short
  pub clean_model: String,
  /// width of the full image
  pub width: usize,
  /// height of the full image
  pub height: usize,
  /// number of components per pixel (1 for bayer, 3 for RGB images)
  pub cpp: usize,
  /// whitebalance coefficients encoded in the file in RGBE order
  pub wb_coeffs: [f32; 4],
  /// image whitelevels in RGBE order
  pub whitelevels: [u16; 4],
  /// image blacklevels in RGBE order
  pub blacklevels: [u16; 4],
  /// matrix to convert XYZ to camera RGBE
  pub xyz_to_cam: [[f32; 3]; 4],
  /// color filter array
  pub cfa: CFA,
  /// how much to crop the image to get all the usable area, order is top, right, bottom, left
  pub crops: [usize; 4],

  /// Areas of the sensor that is masked to prevent it from receiving light. Used to calculate
  /// black levels and noise. Each tuple represents a masked rectangle's top, right, bottom, left
  pub blackareas: Vec<(u64, u64, u64, u64)>,

  /// orientation of the image as indicated by the image metadata
  pub orientation: Orientation,
  /// image data itself, has `width`\*`height`\*`cpp` elements
  pub data: RawImageData,

  pub color_matrix: HashMap<Illuminant, FlatColorMatrix>,
}

/// The actual image data, after decoding
#[derive(Debug, Clone)]
pub enum RawImageData {
  /// The most usual u16 output of almost all formats
  Integer(Vec<u16>),
  /// Some formats are directly encoded as f32, most notably some DNGs
  Float(Vec<f32>),
}

impl RawImage {
  #[doc(hidden)]
  pub fn new(cam: Camera, width: usize, height: usize, wb_coeffs: [f32; 4], image: Vec<u16>, dummy: bool) -> RawImage {
    let blacks = if !dummy && (cam.blackareah.1 != 0 || cam.blackareav.1 != 0) {
      let mut avg = [0 as f32; 4];
      let mut count = [0 as f32; 4];
      for row in cam.blackareah.0..cam.blackareah.0 + cam.blackareah.1 {
        for col in 0..width {
          let color = cam.cfa.color_at(row, col);
          avg[color] += image[row * width + col] as f32;
          count[color] += 1.0;
        }
      }
      for row in 0..height {
        for col in cam.blackareav.0..cam.blackareav.0 + cam.blackareav.1 {
          let color = cam.cfa.color_at(row, col);
          avg[color] += image[row * width + col] as f32;
          count[color] += 1.0;
        }
      }
      [
        (avg[0] / count[0]) as u16,
        (avg[1] / count[1]) as u16,
        (avg[2] / count[2]) as u16,
        (avg[3] / count[3]) as u16,
      ]
    } else {
      cam.blacklevels
    };

    // tuple format is top, right, bottom left
    let mut blackareas: Vec<(u64, u64, u64, u64)> = Vec::new();

    if cam.blackareah.1 != 0 {
      blackareas.push((cam.blackareah.0 as u64, width as u64, (cam.blackareah.0 + cam.blackareah.1) as u64, 0));
    }

    if cam.blackareav.1 != 0 {
      blackareas.push((0, (cam.blackareav.0 + cam.blackareav.1) as u64, height as u64, cam.blackareav.0 as u64))
    }

    RawImage {
      make: cam.make.clone(),
      model: cam.model.clone(),
      clean_make: cam.clean_make.clone(),
      clean_model: cam.clean_model.clone(),
      width: width,
      height: height,
      cpp: 1,
      wb_coeffs: wb_coeffs,
      data: RawImageData::Integer(image),
      blacklevels: blacks,
      whitelevels: cam.whitelevels,
      xyz_to_cam: cam.xyz_to_cam,

      cfa: cam.cfa.clone(),
      crops: cam.crops,
      blackareas: blackareas,
      orientation: cam.orientation,
      color_matrix: cam.color_matrix,
    }
  }

  pub fn develop_params(&self) -> Result<DevelopParams, String> {
    let mut xyz2cam: [[f32; 3]; 4] = [[0.0; 3]; 4];
    let color_matrix = self.color_matrix.get(&Illuminant::D65).unwrap(); // TODO fixme
    assert_eq!(color_matrix.len() % 3, 0); // this is not so nice...
    let components = color_matrix.len() / 3;
    for i in 0..components {
      for j in 0..3 {
        xyz2cam[i][j] = color_matrix[i*3+j];
      }
    }

    let active_area = Rect::new(
      Point::new(self.crops[3], self.crops[0]),
      Dim2::new(self.width - self.crops[3] - self.crops[1], self.height - self.crops[0] - self.crops[2]),
    );
    debug!("RAW developing active area: {:?}", active_area);

    let params = DevelopParams {
      width: self.width,
      height: self.height,
      color_matrices: vec![ColorMatrix {
        illuminant: Illuminant::D65,
        matrix: xyz2cam,
      }],
      white_level: self.whitelevels.into(),
      black_level: self.blacklevels.into(),
      pattern: BayerPattern::RGGB,
      wb_coeff: self.wb_coeffs.iter().map(|v| v / 1024.0).take(3).collect(),
      active_area: Some(active_area),
      gamma: 2.4,
    };

    Ok(params)
  }

  /// Outputs the inverted matrix that converts pixels in the camera colorspace into
  /// XYZ components.
  pub fn cam_to_xyz(&self) -> [[f32; 4]; 3] {
    self.pseudoinverse(self.xyz_to_cam)
  }

  /// Outputs the inverted matrix that converts pixels in the camera colorspace into
  /// XYZ components normalized to be easily used to convert to Lab or a RGB output space
  pub fn cam_to_xyz_normalized(&self) -> [[f32; 4]; 3] {
    let mut xyz_to_cam = self.xyz_to_cam;
    // Normalize xyz_to_cam so that xyz_to_cam * (1,1,1) is (1,1,1,1)
    for i in 0..4 {
      let mut num = 0.0;
      for j in 0..3 {
        num += xyz_to_cam[i][j];
      }
      for j in 0..3 {
        xyz_to_cam[i][j] = if num == 0.0 { 0.0 } else { xyz_to_cam[i][j] / num };
      }
    }

    self.pseudoinverse(xyz_to_cam)
  }

  /// Not all cameras encode a whitebalance so in those cases just using a 6500K neutral one
  /// is a good compromise
  pub fn neutralwb(&self) -> [f32; 4] {
    let rgb_to_xyz = [
      // sRGB D65
      [0.412453, 0.357580, 0.180423],
      [0.212671, 0.715160, 0.072169],
      [0.019334, 0.119193, 0.950227],
    ];

    // Multiply RGB matrix
    let mut rgb_to_cam = [[0.0; 3]; 4];
    for i in 0..4 {
      for j in 0..3 {
        rgb_to_cam[i][j] = 0.0;
        for k in 0..3 {
          rgb_to_cam[i][j] += self.xyz_to_cam[i][k] * rgb_to_xyz[k][j];
        }
      }
    }

    let mut neutralwb = [0 as f32; 4];
    for i in 0..4 {
      let mut num = 0.0;
      for j in 0..3 {
        num += rgb_to_cam[i][j];
      }
      neutralwb[i] = 1.0 / num;
    }

    [
      neutralwb[0] / neutralwb[1],
      neutralwb[1] / neutralwb[1],
      neutralwb[2] / neutralwb[1],
      neutralwb[3] / neutralwb[1],
    ]
  }

  fn pseudoinverse(&self, inm: [[f32; 3]; 4]) -> [[f32; 4]; 3] {
    let mut temp: [[f32; 6]; 3] = [[0.0; 6]; 3];

    for i in 0..3 {
      for j in 0..6 {
        temp[i][j] = if j == i + 3 { 1.0 } else { 0.0 };
      }
      for j in 0..3 {
        for k in 0..4 {
          temp[i][j] += inm[k][i] * inm[k][j];
        }
      }
    }

    for i in 0..3 {
      let mut num = temp[i][i];
      for j in 0..6 {
        temp[i][j] /= num;
      }
      for k in 0..3 {
        if k == i {
          continue;
        }
        num = temp[k][i];
        for j in 0..6 {
          temp[k][j] -= temp[i][j] * num;
        }
      }
    }

    let mut out: [[f32; 4]; 3] = [[0.0; 4]; 3];

    for i in 0..4 {
      for j in 0..3 {
        out[j][i] = 0.0;
        for k in 0..3 {
          out[j][i] += temp[j][k + 3] * inm[i][k];
        }
      }
    }

    out
  }

  /// Returns the CFA pattern after the crop has been applied (and thus the pattern
  /// potentially shifted)
  pub fn cropped_cfa(&self) -> CFA {
    self.cfa.shift(self.crops[3], self.crops[0])
  }

  /// Checks if the image is monochrome
  pub fn is_monochrome(&self) -> bool {
    self.cpp == 1 && !self.cfa.is_valid()
  }
}

#[macro_export]
macro_rules! alloc_image_plain {
  ($width:expr, $height:expr, $dummy: expr) => {{
    if $width * $height > 500000000 || $width > 50000 || $height > 50000 {
      panic!("rawloader: surely there's no such thing as a >500MP or >50000 px wide/tall image!");
    }
    if $dummy {
      vec![0]
    } else {
      vec![0; $width * $height]
    }
  }};
}

#[macro_export]
macro_rules! alloc_image {
  ($width:expr, $height:expr, $dummy: expr) => {{
    let out = crate::alloc_image_plain!($width, $height, $dummy);
    if $dummy {
      return out;
    }
    out
  }};
}

#[macro_export]
macro_rules! alloc_image_ok {
  ($width:expr, $height:expr, $dummy: expr) => {{
    let out = crate::alloc_image_plain!($width, $height, $dummy);
    if $dummy {
      return Ok(out);
    }
    out
  }};
}
