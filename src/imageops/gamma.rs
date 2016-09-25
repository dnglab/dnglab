use decoders::Image;

pub fn gamma(img: &Image, inb: &[f32]) -> Vec<f32> {
  let mut out: Vec<f32> = vec![0.0; (img.width*img.height*3) as usize];

  let g: f32 = 0.45;
  let f: f32 = 0.099;
  let min: f32 = 0.018;
  let mul: f32 = 4.5;
  

  let mut pos = 0;
  for val in inb {
    out[pos] = if *val <= min {
      mul * val
    } else {
      ((1.0+f) * val).powf(g) - f
    };

    pos += 1;
  }

  out
}
