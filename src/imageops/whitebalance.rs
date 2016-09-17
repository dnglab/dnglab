use decoders::Image;
use imageops::fcol;

pub fn whitebalance(img: &Image, inb: &[f32]) -> Vec<f32> {
  let mut out: Vec<f32> = vec![0.0; (img.width*img.height) as usize];

  let max: f32 = img.wb_coeffs.iter().fold(0.0, |acc, &x| acc.max(x));
  let mul = img.wb_coeffs.iter().map(|x| x / max).collect::<Vec<f32>>();

  let mut pos = 0;
  for row in 0..img.height {
    for col in 0..img.width {
      let pixel = inb[pos];
      let color = fcol(img, row, col);
      out[pos] = pixel * mul[color];
      pos += 1;
    }
  }

  out
}
