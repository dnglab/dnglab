use decoders::RawImage;
use imageops::{OpBuffer,ImageOp,Pipeline};

#[derive(Copy, Clone, Debug)]
pub struct OpLevel {
}

impl OpLevel {
  pub fn new(_img: &RawImage) -> OpLevel {
    OpLevel{}
  }
}

impl ImageOp for OpLevel {
  fn name(&self) -> &str {"level"}
  fn run(&self, pipeline: &Pipeline, buf: &OpBuffer) -> OpBuffer {
    level_and_balance(pipeline.image, buf)
  }
}

pub fn level_and_balance(img: &RawImage, buf: &OpBuffer) -> OpBuffer {
  let mut buf = buf.clone();

  // Calculate the blacklevels
  let mins = img.blacklevels.iter().map(|&x| x as f32).collect::<Vec<f32>>();
  let ranges = img.whitelevels.iter().enumerate().map(|(i, &x)| (x as f32) - mins[i]).collect::<Vec<f32>>();

  let coeffs = if img.is_monochrome() {
    [1.0, 1.0, 1.0, 1.0]
  } else if !img.wb_coeffs[0].is_normal() ||
            !img.wb_coeffs[1].is_normal() ||
            !img.wb_coeffs[2].is_normal() {
    img.neutralwb()
  } else {
    img.wb_coeffs
  };

  // Set green multiplier as 1.0
  let unity: f32 = coeffs[1];
  let mul = coeffs.iter().map(|x| if !x.is_normal() { 1.0 } else { x / unity }).collect::<Vec<f32>>();

  buf.mutate_lines(&(|line: &mut [f32], _| {
    for pix in line.chunks_mut(4) {
      pix[0] = (((pix[0] - mins[0]) / ranges[0]) * mul[0]).min(1.0);
      pix[1] = (((pix[1] - mins[1]) / ranges[1]) * mul[1]).min(1.0);
      pix[2] = (((pix[2] - mins[2]) / ranges[2]) * mul[2]).min(1.0);
      pix[3] = (((pix[3] - mins[3]) / ranges[3]) * mul[3]).min(1.0);
    }
  }));

  buf
}
