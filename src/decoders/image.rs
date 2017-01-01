use decoders::*;
use decoders::cfa::*;

#[derive(Debug, Clone)]
pub struct Image {
  pub make: String,
  pub model: String,
  pub canonical_make: String,
  pub canonical_model: String,
  pub width: usize,
  pub height: usize,
  pub wb_coeffs: [f32;4],
  pub data: Box<[u16]>,
  pub whitelevels: [u16;4],
  pub blacklevels: [u16;4],
  pub xyz_to_cam: [[f32;3];4],
  pub cfa: CFA,
  pub crops: [usize;4],
}

impl Image {
  pub fn new(camera: &Camera, width: usize, height: usize, wb_coeffs: [f32;4], image: Vec<u16>) -> Image {
    Image {
      make: camera.make.clone(),
      model: camera.model.clone(),
      canonical_make: camera.canonical_make.clone(),
      canonical_model: camera.canonical_model.clone(),
      width: width,
      height: height,
      wb_coeffs: wb_coeffs,
      data: image.into_boxed_slice(),
      blacklevels: camera.blacklevels,
      whitelevels: camera.whitelevels,
      xyz_to_cam: camera.xyz_to_cam,
      cfa: camera.cfa.clone(),
      crops: camera.crops,
    }
  }

  pub fn cam_to_xyz(&self) -> [[f32;4];3] {
    let (cam_to_xyz, _) = self.xyz_matrix_and_neutralwb();
    cam_to_xyz
  }

  pub fn neutralwb(&self) -> [f32;4] {
    let (_, neutralwb) = self.xyz_matrix_and_neutralwb();
    [neutralwb[0]/neutralwb[1],
     neutralwb[1]/neutralwb[1],
     neutralwb[2]/neutralwb[1],
     neutralwb[3]/neutralwb[1]]
  }

  fn xyz_matrix_and_neutralwb(&self) -> ([[f32;4];3],[f32;4]) {
    let d65_white = [0.9547,1.0,1.08883];
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
    // Normalize rgb_to_cam so that rgb_to_cam * (1,1,1) is (1,1,1,1)
    for i in 0..4 {
      let mut num = 0.0;
      for j in 0..3 {
        num += rgb_to_cam[i][j];
      }
      for j in 0..3 {
        rgb_to_cam[i][j] = if num == 0.0 {
          0.0
        }  else {
          rgb_to_cam[i][j] / num
        };
      }
      neutralwb[i] = 1.0 / num;
    }

    let cam_to_rgb = self.pseudoinverse(rgb_to_cam);
    let mut cam_to_xyz = [[0.0;4];3];
    // Multiply RGB matrix and adjust white to get a cam_to_xyz
    for i in 0..3 {
      for j in 0..4 {
        cam_to_xyz[i][j] = 0.0;
        for k in 0..3 {
          cam_to_xyz[i][j] += cam_to_rgb[k][j] * rgb_to_xyz[i][k] / d65_white[i];
        }
      }
    }

    (cam_to_xyz, neutralwb)
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
}
