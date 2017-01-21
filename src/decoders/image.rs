use decoders::*;
use decoders::cfa::*;
use imageops;

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
  /// image data itself, has width*height*cpp elements
  pub data: Vec<u16>,
}

/// A RawImage processed into a full RGB image with levels and gamma
///
/// The data is a Vec<f32> width width*height*3 elements, where each element is a value
/// between 0 and 1 with the intensity of the color channel
#[derive(Debug, Clone)]
pub struct RGBImage {
  pub width: usize,
  pub height: usize,
  pub data: Vec<f32>,
}

impl RawImage {
  #[doc(hidden)] pub fn new(camera: &Camera, width: usize, height: usize, wb_coeffs: [f32;4], image: Vec<u16>) -> RawImage {
    let blacks = if camera.blackareah.1 != 0 || camera.blackareav.1 != 0 {
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

    RawImage {
      make: camera.make.clone(),
      model: camera.model.clone(),
      clean_make: camera.clean_make.clone(),
      clean_model: camera.clean_model.clone(),
      width: width,
      height: height,
      cpp: 1,
      wb_coeffs: wb_coeffs,
      data: image,
      blacklevels: blacks,
      whitelevels: camera.whitelevels,
      xyz_to_cam: camera.xyz_to_cam,
      cfa: camera.cfa.clone(),
      crops: camera.crops,
    }
  }

  /// Outputs the inverted matrix that converts pixels in the camera colorspace into
  /// XYZ components. Those can then be easily used to convert to Lab or a RGB output space
  pub fn cam_to_xyz(&self) -> [[f32;4];3] {
    let (cam_to_xyz, _) = self.xyz_matrix_and_neutralwb();
    cam_to_xyz
  }

  /// Not all cameras encode a whitebalance so in those cases just using a 6500K neutral one
  /// is a good compromise
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

  /// Convert the image to a RGB image by doing a demosaic and applying levels whitebalance
  /// The maxwidth and maxheight values specify maximum dimensions for the final image. If
  /// the original image is smaller this will not scale up but otherwise you will get an
  /// image that is either maxwidth wide or maxheight tall and maintains the image ratio.
  /// Pass in maxwidth and maxheight as 0 if you want the maximum possible image size.
  pub fn to_rgb(&self, maxwidth: usize, maxheight: usize) -> Result<RGBImage,String> {
    let buffer = imageops::simple_decode(self, maxwidth, maxheight);

    Ok(RGBImage{
      width: buffer.width,
      height: buffer.height,
      data: buffer.data,
    })
  }
}
