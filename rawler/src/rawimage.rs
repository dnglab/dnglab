use std::collections::HashMap;

use log::debug;

use crate::{
  decoders::*,
  imgop::{
    raw::{ColorMatrix, DevelopParams},
    sensor::bayer::BayerPattern,
    xyz::{FlatColorMatrix, Illuminant},
    Dim2, Point, Rect,
  },
  CFA,
};

/// All the data needed to process this raw image, including the image data itself as well
/// as all the needed metadata
#[derive(Debug, Clone)]
pub struct RawImage {
  /// Camera definition
  pub camera: Camera,
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
  /// Bits per pixel
  pub bps: usize,
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
  /// how much to crop the image to get all the usable (non-black) area
  pub active_area: Option<Rect>,
  /// how much to crop the image to get all the recommended area
  pub crop_area: Option<Rect>,

  /// Areas of the sensor that is masked to prevent it from receiving light. Used to calculate
  /// black levels and noise.
  pub blackareas: Vec<Rect>,

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
  pub fn calc_black_levels(cam: &Camera, blackareas: &[Rect], width: usize, _height: usize, image: &[u16]) -> Option<[u16; 4]> {
    let mut avg = [0 as f32; 4];
    let mut count = [0 as f32; 4];

    for area in blackareas {
      for row in area.p.y..area.p.y + area.d.h {
        for col in area.p.x..area.p.x + area.d.w {
          let color = cam.cfa.color_at(row, col);
          avg[color] += image[row * width + col] as f32;
          count[color] += 1.0;
        }
      }
    }

    if cam.blackareav.is_some() || cam.blackareah.is_some() {
      let blacklevels = [
        (avg[0] / count[0]) as u16,
        (avg[1] / count[1]) as u16,
        (avg[2] / count[2]) as u16,
        (avg[3] / count[3]) as u16,
      ];
      debug!("Calculated blacklevels: {:?}", blacklevels);
      Some(blacklevels)
    } else {
      None
    }
  }

  #[doc(hidden)]
  pub fn new(cam: Camera, width: usize, height: usize, cpp: usize, wb_coeffs: [f32; 4], image: Vec<u16>, dummy: bool) -> RawImage {
    /*
    let (width, height, image) = if let Some(crops) = &cam.crops {
      let crops = Rect::new_with_borders(Dim2::new(width, height), crops);
      if !dummy {
        let mut croppedimage = Vec::with_capacity(crops.d.h * crops.d.w * cpp);
        for row in crops.p.y..(crops.p.y + crops.d.h) {
          let start = row * width * cpp + crops.p.x;
          croppedimage.extend_from_slice(&image[start..start + crops.d.w * cpp]);
        }
        (crops.d.w, crops.d.h, croppedimage)
      } else {
        (crops.d.w, crops.d.h, image)
      }
    } else {
      (width, height, image)
    };
    */

    let mut blackareas: Vec<Rect> = Vec::new();

    let active_area = cam.active_area.map(|area| Rect::new_with_borders(Dim2::new(width, height), &area));

    if let Some(active) = active_area {
      // Build black areas
      if let Some(ah) = cam.blackareah {
        blackareas.push(Rect::new_with_points(Point::new(active.p.x, ah.0), Point::new(active.p.x + active.d.w, ah.1)));
      }
      if let Some(av) = cam.blackareav {
        blackareas.push(Rect::new_with_points(Point::new(av.0, active.p.y), Point::new(av.1, active.p.y + active.d.h)));
      }
    }

    let blacks = if !dummy {
      Self::calc_black_levels(&cam, &blackareas, width, height, &image).unwrap_or(cam.blacklevels)
    } else {
      cam.blacklevels
    };

    let crop_area = cam.crop_area.map(|area| Rect::new_with_borders(Dim2::new(width, height), &area));

    RawImage {
      camera: cam.clone(),
      make: cam.make.clone(),
      model: cam.model.clone(),
      clean_make: cam.clean_make.clone(),
      clean_model: cam.clean_model.clone(),
      width,
      height,
      cpp,
      bps: cam.real_bps,
      wb_coeffs,
      data: RawImageData::Integer(image),
      blacklevels: blacks,
      whitelevels: cam.whitelevels,
      xyz_to_cam: cam.xyz_to_cam,
      cfa: cam.cfa.clone(),
      active_area,
      crop_area,
      blackareas,
      orientation: Orientation::Normal, //cam.orientation, // TODO fixme
      color_matrix: cam.color_matrix,
    }
  }

  pub fn dim(&self) -> Dim2 {
    Dim2::new(self.width, self.height)
  }

  pub fn pixels_u16(&self) -> &[u16] {
    if let RawImageData::Integer(data) = &self.data {
      data
    } else {
      panic!("Data ist not u16");
    }
  }

  pub fn pixels_u16_mut(&mut self) -> &mut [u16] {
    if let RawImageData::Integer(data) = &mut self.data {
      data
    } else {
      panic!("Data ist not u16");
    }
  }

  pub fn develop_params(&self) -> Result<DevelopParams, String> {
    let mut xyz2cam: [[f32; 3]; 4] = [[0.0; 3]; 4];
    //let color_matrix = self.color_matrix.get(&Illuminant::D65).unwrap(); // TODO fixme
    let color_matrix = self.color_matrix.values().next().unwrap(); // TODO fixme
    assert_eq!(color_matrix.len() % 3, 0); // this is not so nice...
    let components = color_matrix.len() / 3;
    for i in 0..components {
      for j in 0..3 {
        xyz2cam[i][j] = color_matrix[i * 3 + j];
      }
    }

    let pattern = match self.cfa.to_string().as_str() {
      "RGGB" => BayerPattern::RGGB,
      "BGGR" => BayerPattern::BGGR,
      "GRBG" => BayerPattern::GRBG,
      "GBRG" => BayerPattern::GBRG,
      _ => panic!("Unsupported bayer pattern"),
    };

    /*
    let active_area = Rect::new(
      Point::new(self.crops[3], self.crops[0]),
      Dim2::new(self.width - self.crops[3] - self.crops[1], self.height - self.crops[0] - self.crops[2]),
    );
    */
    debug!("RAW developing active area: {:?}", self.active_area);

    let params = DevelopParams {
      width: self.width,
      height: self.height,
      color_matrices: vec![ColorMatrix {
        illuminant: Illuminant::D65,
        matrix: xyz2cam,
      }],
      white_level: self.whitelevels.into(),
      black_level: self.blacklevels.into(),
      pattern,
      cfa: self.cfa.clone(),
      wb_coeff: self.wb_coeffs,
      active_area: self.active_area,
      crop_area: self.crop_area,
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
    //self.cfa.shift(self.crops[3], self.crops[0])
    todo!()
    // Need to specify which crop, active or DefaultCrop
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
      return crate::pixarray::PixU16::default();
    }
    out
  }};
}

#[macro_export]
macro_rules! alloc_image_ok {
  ($width:expr, $height:expr, $dummy: expr) => {{
    let out = crate::alloc_image_plain!($width, $height, $dummy);
    if $dummy {
      return Ok(crate::pixarray::PixU16::default());
    }
    out
  }};
}
