use decoders::Image;

pub fn whitebalance(img: &Image, buf: &mut Vec<f32>) {
  // Set green multiplier as 1.0
  let unity: f32 = img.wb_coeffs[1];
  let mul = img.wb_coeffs.iter().map(|x| if x.is_nan() { 1.0 } else { x / unity }).collect::<Vec<f32>>();

  for pix in buf.chunks_mut(4) {
    pix[0] = (pix[0] * mul[0]).min(1.0);
    pix[1] = (pix[1] * mul[1]).min(1.0);
    pix[2] = (pix[2] * mul[2]).min(1.0);
    pix[3] = (pix[3] * mul[3]).min(1.0);
  }
}
