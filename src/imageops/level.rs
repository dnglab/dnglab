use decoders::Image;
pub fn level_and_balance(img: &Image, buf: &mut [f32]) {
  // Calculate the blacklevels
  let mins = img.blacklevels.iter().map(|&x| (x as f32) / 65535.0).collect::<Vec<f32>>();
  let ranges = img.whitelevels.iter().enumerate().map(|(i, &x)| ((x as f32) / 65535.0) - mins[i]).collect::<Vec<f32>>();

  // Set green multiplier as 1.0
  let unity: f32 = img.wb_coeffs[1];
  let mul = img.wb_coeffs.iter().map(|x| if x.is_nan() { 1.0 } else { x / unity }).collect::<Vec<f32>>();

  for pix in buf.chunks_mut(4) {
    pix[0] = (((pix[0] - mins[0]) / ranges[0]) * mul[0]).min(1.0);
    pix[1] = (((pix[1] - mins[1]) / ranges[1]) * mul[1]).min(1.0);
    pix[2] = (((pix[2] - mins[2]) / ranges[2]) * mul[2]).min(1.0);
    pix[3] = (((pix[3] - mins[3]) / ranges[3]) * mul[3]).min(1.0);
  }
}
