use decoders::Image;
use imageops::fcol;

pub fn whitebalance(img: &Image, buf: &mut Vec<f32>) {
  // Set green multiplier as 1.0
  let unity: f32 = img.wb_coeffs[1];
  let mul = img.wb_coeffs.iter().map(|x| if x.is_nan() { 1.0 } else { x / unity }).collect::<Vec<f32>>();

  for row in 0..img.height {
    for col in 0..img.width {
      let pos = row*img.width + col;
      let color = fcol(img, row, col);
      buf[pos] = (buf[pos] * mul[color]).min(1.0);
    }
  }
}
