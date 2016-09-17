use decoders::Image;
use imageops::fcol;

pub fn level (img: &Image) -> Vec<f32> {
  let mut out: Vec<f32> = vec![0.0; (img.width*img.height) as usize];

  let mins = img.blacklevels.iter().map(|&x| x as f32).collect::<Vec<f32>>();
  let ranges = img.whitelevels.iter().enumerate().map(|(i, &x)| (x as f32) - mins[i]).collect::<Vec<f32>>();

  let mut pos = 0;
  for row in 0..img.height {
    for col in 0..img.width {
      let color = fcol(img, row, col);
      let pixel = img.data[pos] as f32;
      out[pos] = (pixel - mins[color]) / ranges[color];
      pos += 1;
    }
  }

  out
}
