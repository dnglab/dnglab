use decoders::Image;
extern crate itertools;
use self::itertools::Itertools;

pub fn camera_to_rec709(img: &Image, inb: &[f32]) -> Vec<f32> {
  let mut out: Vec<f32> = vec![0.0; (img.width*img.height*3) as usize];

  let cmatrix = cam_to_rec709_matrix(img);

  for pos in (0..(img.height*img.width*3)).step(3) {
    let cr = inb[pos];
    let cg = inb[pos+1];
    let cb = inb[pos+2];

    out[pos+0] = cr * cmatrix[0][0] + cg * cmatrix[0][1] + cb * cmatrix[0][2];
    out[pos+1] = cr * cmatrix[1][0] + cg * cmatrix[1][1] + cb * cmatrix[1][2];
    out[pos+2] = cr * cmatrix[2][0] + cg * cmatrix[2][1] + cb * cmatrix[2][2];
  }

  out
}

fn pseudoinverse(inm: [[f32;3];4]) -> [[f32;4];3] {
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


fn cam_to_rec709_matrix(img: &Image) -> [[f32;4];3] {
  let mut xyz_to_cam = [[0.0;3];4];
  for i in 0..12 {
    xyz_to_cam[i/3][i%3] = img.color_matrix[i] as f32;
  }

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
        rgb_to_cam[i][j] += xyz_to_cam[i][k] * rgb_to_xyz[k][j];
      }
    }
  }

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
  }

  pseudoinverse(rgb_to_cam)
}
