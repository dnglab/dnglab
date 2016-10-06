use decoders::Image;
use imageops::fcol;
use imageops::OpBuffer;
use std::cmp;

pub fn demosaic_and_scale(img: &Image, minwidth: usize, minheight: usize) -> OpBuffer {
  let scale = cmp::min(img.width / minwidth, img.height / minheight);

  if scale <= 1 {
    full(img)
  } else {
    scaled(img, cmp::min(scale, 16)) // Never go less than 1/16th demosaic
  }
}

pub fn full(img: &Image) -> OpBuffer {
  let mut out = OpBuffer::new(img.width, img.height, 4);

  // First we set the colors we already have
  let mut ipos = 0;
  let mut opos = 0;
  for row in 0..img.height {
    for col in 0..img.width {
      let color = fcol(img, row, col);
      out.data[opos+color] = img.data[ipos] as f32;
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
            sums[c] += img.data[y*img.width+x] as f32;
            counts[c] += 1;
          }
        }
      }

      for c in 0..4 {
        if c != color && counts[c] > 0 {
          out.data[(row*img.width+col)*4+c] = sums[c] / (counts[c] as f32);
        }
      }
    }
  }

  out
}

pub fn scaled(img: &Image, scale: usize) -> OpBuffer {
  println!("Scaled demosaic'ing at 1/{} scale", scale);

  let mut out = OpBuffer::new(img.width/scale, img.height/scale, 4);

  // Go around the image averaging every block of pixels
  for row in 0..out.height {
    for col in 0..out.width {
      let mut sums: [f32; 4] = [0.0;4];
      let mut counts: [u32; 4] = [0; 4];

      for y in row*scale..(row+1)*scale {
        for x in col*scale..(col+1)*scale {
          let c = fcol(img, y, x);
          sums[c] += img.data[y*img.width+x] as f32;
          counts[c] += 1;
        }
      }

      for c in 0..4 {
        if counts[c] > 0 {
          out.data[(row*out.width+col)*4+c] = sums[c] / (counts[c] as f32);
        }
      }
    }
  }

  out
}
