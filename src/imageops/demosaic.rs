// Demosaic methods adapted from dcraw 9.27
use decoders::Image;
use imageops::fcol;

pub fn ppg(img: &Image, inb: &[f32]) -> Vec<f32> {
  let mut out: Vec<f32> = vec![0.0; (img.width*img.height*3) as usize];

  let mut ipos = 0;
  let mut opos = 0;
  for row in 0..img.width {
    for col in 0..img.height {
      let color = fcol(img, row, col);
      
      out[opos+0] = inb[ipos];
      out[opos+1] = inb[ipos];
      out[opos+2] = inb[ipos];

      ipos += 1;
      opos += 3;
    }
  }

  out
}
