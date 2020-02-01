use crate::decoders::*;
use crate::decoders::cfa::*;

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
  pub wb_coeffs: [f32;4],
  /// image whitelevels in RGBE order
  pub whitelevels: [u16;4],
  /// image blacklevels in RGBE order
  pub blacklevels: [u16;4],
  /// matrix to convert XYZ to camera RGBE
  pub xyz_to_cam: [[f32;3];4],
  /// color filter array
  pub cfa: CFA,
  /// how much to crop the image to get all the usable area, order is top, right, bottom, left
  pub crops: [usize;4],

  /// Areas of the sensor that is masked to prevent it from receiving light. Used to calculate
  /// black levels and noise. Each tuple represents a masked rectangle's top, right, bottom, left
  pub blackareas: Vec<(u64,u64,u64,u64)>,

  /// orientation of the image as indicated by the image metadata
  pub orientation: Orientation,
  /// image data itself, has `width`\*`height`\*`cpp` elements
  pub data: RawImageData,
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
  #[doc(hidden)] pub fn new(camera: Camera, width: usize, height: usize, wb_coeffs: [f32;4], image: Vec<u16>, dummy: bool) -> RawImage {
    let blacks = if !dummy && (camera.blackareah.1 != 0 || camera.blackareav.1 != 0) {
      let mut avg = [0 as f32; 4];
      let mut count = [0 as f32; 4];
      for row in camera.blackareah.0 .. camera.blackareah.0+camera.blackareah.1 {
        for col in 0..width {
          let color = camera.cfa.color_at(row,col);
          avg[color] += image[row*width+col] as f32;
          count[color] += 1.0;
        }
      }
      for row in 0..height {
        for col in camera.blackareav.0 .. camera.blackareav.0+camera.blackareav.1 {
          let color = camera.cfa.color_at(row,col);
          avg[color] += image[row*width+col] as f32;
          count[color] += 1.0;
        }
      }
      [(avg[0]/count[0]) as u16,
       (avg[1]/count[1]) as u16,
       (avg[2]/count[2]) as u16,
       (avg[3]/count[3]) as u16]
    } else {
      camera.blacklevels
    };

    // tuple format is top, right, bottom left
    let mut blackareas: Vec<(u64,u64,u64,u64)> = Vec::new();

    if camera.blackareah.1 != 0 {
        blackareas.push((camera.blackareah.0 as u64, width as u64,
                        (camera.blackareah.0 + camera.blackareah.1) as u64, 0));
    }

    if camera.blackareav.1 != 0 {
        blackareas.push((0, (camera.blackareav.0 + camera.blackareav.1) as u64,
                        height as u64, camera.blackareav.0 as u64))
    }

    RawImage {
      make: camera.make.clone(),
      model: camera.model.clone(),
      clean_make: camera.clean_make.clone(),
      clean_model: camera.clean_model.clone(),
      width: width,
      height: height,
      cpp: 1,
      wb_coeffs: wb_coeffs,
      data: RawImageData::Integer(image),
      blacklevels: blacks,
      whitelevels: camera.whitelevels,
      xyz_to_cam: camera.xyz_to_cam,
      cfa: camera.cfa.clone(),
      crops: camera.crops,
      blackareas: blackareas,
      orientation: camera.orientation,
    }
  }

  /// Outputs the inverted matrix that converts pixels in the camera colorspace into
  /// XYZ components.
  pub fn cam_to_xyz(&self) -> [[f32;4];3] {
    self.pseudoinverse(self.xyz_to_cam)
  }

  /// Outputs the inverted matrix that converts pixels in the camera colorspace into
  /// XYZ components normalized to be easily used to convert to Lab or a RGB output space
  pub fn cam_to_xyz_normalized(&self) -> [[f32;4];3] {
    let mut xyz_to_cam = self.xyz_to_cam;
    // Normalize xyz_to_cam so that xyz_to_cam * (1,1,1) is (1,1,1,1)
    for i in 0..4 {
      let mut num = 0.0;
      for j in 0..3 {
        num += xyz_to_cam[i][j];
      }
      for j in 0..3 {
        xyz_to_cam[i][j] = if num == 0.0 {
          0.0
        }  else {
          xyz_to_cam[i][j] / num
        };
      }
    }

    self.pseudoinverse(xyz_to_cam)
  }

  /// Not all cameras encode a whitebalance so in those cases just using a 6500K neutral one
  /// is a good compromise
  pub fn neutralwb(&self) -> [f32;4] {
    let rgb_to_xyz = [
    // sRGB D65
      [ 0.412453, 0.357580, 0.180423 ],
      [ 0.212671, 0.715160, 0.072169 ],
      [ 0.019334, 0.119193, 0.950227 ],
    ];

    // Multiply RGB matrix
    let mut rgb_to_cam = [[0.0;3];4];
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

    [neutralwb[0]/neutralwb[1],
     neutralwb[1]/neutralwb[1],
     neutralwb[2]/neutralwb[1],
     neutralwb[3]/neutralwb[1]]
  }

  fn pseudoinverse(&self, inm: [[f32;3];4]) -> [[f32;4];3] {
    let mut temp: [[f32;6];3] = [[0.0; 6];3];

    for i in 0..3 {
      for j in 0..6 {
        temp[i][j] = if j == i+3 { 1.0 } else { 0.0 };
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
        if k == i { continue }
        num = temp[k][i];
        for j in 0..6 {
          temp[k][j] -= temp[i][j] * num;
        }
      }
    }

    let mut out: [[f32;4];3] = [[0.0; 4];3];

    for i in 0..4 {
      for j in 0..3 {
        out[j][i] = 0.0;
        for k in 0..3 {
          out[j][i] += temp[j][k+3] * inm[i][k];
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
