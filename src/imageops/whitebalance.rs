use decoders::Image;
use imageops::fcol;

pub fn whitebalance(img: &Image, inb: &[f32]) -> Vec<f32> {
  let mut out: Vec<f32> = vec![0.0; (img.width*img.height) as usize];

  // Set green multiplier as 1.0
  let unity: f32 = img.wb_coeffs[1];
  let mul = img.wb_coeffs.iter().map(|x| if x.is_nan() { 1.0 } else { x / unity }).collect::<Vec<f32>>();

  let mut pos = 0;
  for row in 0..img.height {
    for col in 0..img.width {
      let pixel = inb[pos];
      let color = fcol(img, row, col);
      out[pos] = (pixel * mul[color]).min(1.0);
      pos += 1;
    }
  }

  out
}
