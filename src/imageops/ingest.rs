use decoders::Image;

pub fn float(img: &Image) -> Vec<f32> {
  let mut out: Vec<f32> = vec![0.0; (img.width*img.height) as usize];

  for (pixin,pixout) in img.data.chunks(1).zip(out.chunks_mut(1)) {
    pixout[0] = (pixin[0] as f32) / 65535.0;
  }

  out
}
