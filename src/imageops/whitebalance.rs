use decoders::Image;
use imageops::fcol;

pub fn whitebalance(img: &Image, buf: &mut Vec<f32>) {
  // Set green multiplier as 1.0
  let unity: f32 = img.wb_coeffs[1];
  let mul = img.wb_coeffs.iter().map(|x| if x.is_nan() { 1.0 } else { x / unity }).collect::<Vec<f32>>();

  for (row,line) in buf.chunks_mut(img.width).enumerate() {
    for (col,pix) in line.chunks_mut(1).enumerate() {
      let color = fcol(img, row, col);
      pix[0] = (pix[0] * mul[color]).min(1.0);
    }
  }
}
