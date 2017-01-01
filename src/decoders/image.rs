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
}
