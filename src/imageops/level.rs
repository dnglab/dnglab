use decoders::Image;
use imageops::fcol;

pub fn level (img: &Image) -> Vec<f32> {
  let mut out: Vec<f32> = vec![0.0; (img.width*img.height) as usize];

  let mins = img.blacklevels.iter().map(|&x| x as f32).collect::<Vec<f32>>();
  let ranges = img.whitelevels.iter().enumerate().map(|(i, &x)| (x as f32) - mins[i]).collect::<Vec<f32>>();

  for (row,line) in img.data.chunks(img.width).enumerate() {
    for (col,pix) in line.iter().enumerate() {
      let color = fcol(img, row, col);
      let pixel = *pix as f32;
      out[row*img.width+col] = (pixel - mins[color]) / ranges[color];
    }
  }

  out
}
