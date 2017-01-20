use decoders::RawImage;
use imageops::OpBuffer;

pub fn level_and_balance(img: &RawImage, buf: &mut OpBuffer) {
  // Calculate the blacklevels
  let mins = img.blacklevels.iter().map(|&x| x as f32).collect::<Vec<f32>>();
  let ranges = img.whitelevels.iter().enumerate().map(|(i, &x)| (x as f32) - mins[i]).collect::<Vec<f32>>();

  let coeffs = if img.wb_coeffs[0].is_nan() {
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
}
