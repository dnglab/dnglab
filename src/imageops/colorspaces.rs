use decoders::Image;
extern crate itertools;
use self::itertools::Itertools;

pub fn camera_to_lab(img: &Image, inb: &[f32]) -> Vec<f32> {
  let mut out: Vec<f32> = vec![0.0; (img.width*img.height*3) as usize];
  let cmatrix = cam_to_xyz_matrix(img);

  let mut opos = 0;
  for pos in (0..(img.height*img.width*4)).step(4) {
    let cr = inb[pos];
    let cg = inb[pos+1];
    let cb = inb[pos+2];
    let ce = inb[pos+3];

    let x = cr * cmatrix[0][0] + cg * cmatrix[0][1] + cb * cmatrix[0][2] + ce * cmatrix[0][3];
    let y = cr * cmatrix[1][0] + cg * cmatrix[1][1] + cb * cmatrix[1][2] + ce * cmatrix[1][3];
    let z = cr * cmatrix[2][0] + cg * cmatrix[2][1] + cb * cmatrix[2][2] + ce * cmatrix[2][3];

    let (l,a,b) = xyz_to_lab(x,y,z);

    out[opos+0] = l;
    out[opos+1] = a;
    out[opos+2] = b;

    opos += 3;
  }

  out
}

pub fn lab_to_rec709(img: &Image, inb: &[f32]) -> Vec<f32> {
  let mut out: Vec<f32> = vec![0.0; (img.width*img.height*3) as usize];
  let cmatrix = xyz_to_rec709_matrix();

  let mut opos = 0;
  for pos in (0..(img.height*img.width*3)).step(3) {
    let l = inb[pos];
    let a = inb[pos+1];
    let b = inb[pos+2];

    let (x,y,z) = lab_to_xyz(l,a,b);

    let r = x * cmatrix[0][0] + y * cmatrix[0][1] + z * cmatrix[0][2];
    let g = x * cmatrix[1][0] + y * cmatrix[1][1] + z * cmatrix[1][2];
    let b = x * cmatrix[2][0] + y * cmatrix[2][1] + z * cmatrix[2][2];

    out[opos+0] = r;
    out[opos+1] = g;
    out[opos+2] = b;

    opos += 3;
  }

  out
}

fn inverse(inm: [[f32;3];3]) -> [[f32;3];3] {
  let invdet = 1.0 / (
    inm[0][0] * (inm[1][1] * inm[2][2] - inm[2][1] * inm[1][2]) -
    inm[0][1] * (inm[1][0] * inm[2][2] - inm[1][2] * inm[2][0]) +
    inm[0][2] * (inm[1][0] * inm[2][1] - inm[1][1] * inm[2][0])
  );

  let mut out = [[0.0; 3];3];
  out[0][0] =  (inm[1][1]*inm[2][2] - inm[2][1]*inm[1][2]) * invdet;
  out[0][1] = -(inm[0][1]*inm[2][2] - inm[0][2]*inm[2][1]) * invdet;
  out[0][2] =  (inm[0][1]*inm[1][2] - inm[0][2]*inm[1][1]) * invdet;
  out[1][0] = -(inm[1][0]*inm[2][2] - inm[1][2]*inm[2][0]) * invdet;
  out[1][1] =  (inm[0][0]*inm[2][2] - inm[0][2]*inm[2][0]) * invdet;
  out[1][2] = -(inm[0][0]*inm[1][2] - inm[1][0]*inm[0][2]) * invdet;
  out[2][0] =  (inm[1][0]*inm[2][1] - inm[2][0]*inm[1][1]) * invdet;
  out[2][1] = -(inm[0][0]*inm[2][1] - inm[2][0]*inm[0][1]) * invdet;
  out[2][2] =  (inm[0][0]*inm[1][1] - inm[1][0]*inm[0][1]) * invdet;

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


fn cam_to_xyz_matrix(img: &Image) -> [[f32;4];3] {
  let mut xyz_to_cam = [[0.0;3];4];
  for i in 0..12 {
    xyz_to_cam[i/3][i%3] = img.color_matrix[i] as f32;
  }

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

  pseudoinverse(xyz_to_cam)
}

fn xyz_to_rec709_matrix() -> [[f32;3];3] {
  let mut rgb_to_xyz = [
  // sRGB D65
    [ 0.412453, 0.357580, 0.180423 ],
    [ 0.212671, 0.715160, 0.072169 ],
    [ 0.019334, 0.119193, 0.950227 ],
  ];


  // Normalize rgb_to_xyz so that rgb_to_xyz * (1,1,1) is (1,1,1)
  for i in 0..3 {
    let mut num = 0.0;
    for j in 0..3 {
      num += rgb_to_xyz[i][j];
    }
    for j in 0..3 {
      rgb_to_xyz[i][j] = if num == 0.0 {
        0.0
      }  else {
        rgb_to_xyz[i][j] / num
      };
    }
  }

  inverse(rgb_to_xyz)
}

fn xyz_to_lab(x: f32, y: f32, z: f32) -> (f32,f32,f32) {
  // D50 White
  let xw = 0.9642; let yw = 1.000; let zw = 0.8249;

  let l = 116.0 * labf(y/yw) - 16.0;
  let a = 500.0 * (labf(x/xw) - labf(y/yw));
  let b = 200.0 * (labf(y/yw) - labf(z/zw));

  (l/100.0,(a+128.0)/256.0,(b+128.0)/256.0)
}

fn labf(val: f32) -> f32 {
  let cutoff = (6.0/29.0)*(6.0/29.0)*(6.0/29.0);
  let multiplier = (1.0/3.0) * (29.0/6.0) * (29.0/6.0);
  let constant = 4.0 / 29.0;

  if val > cutoff {
    val.powf(1.0 / 3.0)
  } else {
    val * multiplier + constant
  }
}

fn lab_to_xyz(l: f32, a: f32, b: f32) -> (f32,f32,f32) {
  // D50 White
  let xw = 0.9642; let yw = 1.000; let zw = 0.8249;

  let cl = l * 100.0;
  let ca = (a * 256.0) - 128.0;
  let cb = (b * 256.0) - 128.0;

  let x = xw * labinvf((1.0/116.0) * (cl+16.0) + (1.0/500.0) * ca);
  let y = yw * labinvf((1.0/116.0) * (cl+16.0));
  let z = zw * labinvf((1.0/116.0) * (cl+16.0) - (1.0/200.0) * cb);

  (x,y,z)
}

fn labinvf(val: f32) -> f32 {
  let cutoff = 6.0 / 29.0;
  let multiplier = 3.0 * (6.0/29.0) * (6.0/29.0);
  let constant = multiplier * (-4.0 / 29.0);

  if val > cutoff {
    val.powf(3.0)
  } else {
    val * multiplier + constant
  }
}
