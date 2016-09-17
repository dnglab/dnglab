use decoders::Image;

pub fn level (img: &Image) -> Vec<f32> {
  let mut out: Vec<f32> = vec![0.0; (img.width*img.height) as usize];

  let min: f32 = img.blacklevels[0] as f32;
  let range: f32 = (img.whitelevels[0] as f32) - min;

  for (pos, pixel) in img.data.iter().enumerate() {
    out[pos] = ((*pixel as f32) - min) / range;
  }

  out
}
