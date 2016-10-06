// Demosaic methods adapted from dcraw 9.27
use decoders::Image;
use imageops::fcol;
use imageops::OpBuffer;
use std::cmp;

pub fn ppg(img: &Image, inb: &OpBuffer) -> OpBuffer {
  let mut out: Vec<f32> = vec![0.0; (inb.width*inb.height*4) as usize];

  // First we set the colors we already have
  let mut ipos = 0;
  let mut opos = 0;
  for row in 0..img.height {
    for col in 0..img.width {
      let color = fcol(img, row, col);
      out[opos+color] = inb.data[ipos];
      ipos += 1;
      opos += 4;
    }
  }

  // Now we go around the image setting the unset colors to the average of the
  // surrounding pixels
  for row in 0..img.height {
    for col in 0..img.width {
      let mut sums: [f32; 4] = [0.0;4];
      let mut counts: [u32; 4] = [0; 4];
      let color = fcol(img, row, col);

      for y in (cmp::max(0,(row as isize)-1) as usize) .. cmp::min(img.height, row+2) {
        for x in (cmp::max(0,(col as isize)-1) as usize) .. cmp::min(img.width, col+2) {
          let c = fcol(img, y, x);
          if c != color {
            sums[c] += inb.data[y*img.width+x];
            counts[c] += 1;
          }
        }
      }

      for c in 0..4 {
        if c != color && counts[c] > 0 {
          out[(row*img.width+col)*4+c] = sums[c] / (counts[c] as f32);
        }
      }
    }
  }

  OpBuffer {
    width: inb.width,
    height: inb.height,
    colors: 4,
    data: out,
  }
}
