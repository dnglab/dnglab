// Demosaic methods adapted from dcraw 9.27
use decoders::Image;
use imageops::fcol;
use std::cmp;

pub fn ppg(img: &Image, inb: &[f32]) -> Vec<f32> {
  let mut out: Vec<f32> = vec![0.0; (img.width*img.height*3) as usize];

  // First we set the colors we already have
  let mut ipos = 0;
  let mut opos = 0;
  for row in 0..img.width {
    for col in 0..img.height {
      let color = fcol(img, row, col);
      out[opos+color] = inb[ipos];
      ipos += 1;
      opos += 3;
    }
  }

  // Now we go around the image setting the unset colors to the average of the
  // surrounding pixels
  for row in 0..img.height {
    for col in 0..img.width {
      let mut sums: [f32; 4] = [0.0;4];
      let mut counts: [f32; 4] = [0.0;4];

      for y in (cmp::max(0,(row as isize)-1) as usize) .. cmp::min(img.height, row+1) {
        for x in (cmp::max(0,(col as isize)-1) as usize) .. cmp::min(img.width, col+1) {
          let color = fcol(img, x, y);
          //println!("Fetching color {} at pos {}x{}", color, x, y);
          sums[color] += inb[y*img.width+x];
          counts[color] += 1.0;
        }
      }

      let color = fcol(img, row, col);
      for c in 0..3 {
        if c != color {
          out[(row*img.width+col)*3+c] = sums[c] / counts[c];
        }
      }
    }
  }

  out
}
