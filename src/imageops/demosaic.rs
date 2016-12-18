use decoders::Image;
use imageops::fcol;
use imageops::OpBuffer;
use std::cmp;

pub fn demosaic_and_scale(img: &Image, minwidth: usize, minheight: usize) -> OpBuffer {
  // Calculate the resulting width/height and top-left corner after crops
  let width = (img.width as i64 - img.crops[1] - img.crops[3]) as usize;
  let height = (img.height as i64 - img.crops[0] - img.crops[2]) as usize;
  let x = img.crops[3] as usize;
  let y = img.crops[0] as usize;

  let scale = if minwidth == 0 || minheight == 0 {
    0
  } else {
    cmp::min(width / minwidth, height / minheight)
  };

  if scale <= 1 {
    full(img, x, y, width, height)
  } else {
    scaled(img, cmp::min(scale, 16), x, y, width, height) // Never go less than 1/16th demosaic
  }
}

pub fn full(img: &Image, xs: usize, ys: usize, width: usize, height: usize) -> OpBuffer {
  let mut out = OpBuffer::new(width, height, 4);

  // First we set the colors we already have
  out.mutate_lines(&(|line: &mut [f32], row| {
    for (col, (pixout, pixin)) in line.chunks_mut(4).zip(img.data[img.width*(row+ys)+xs..].chunks(1)).enumerate() {
      let color = fcol(img, row, col);
      pixout[color] = pixin[0] as f32;
    }
  }));

  // Now we go around the image setting the unset colors to the average of the
  // surrounding pixels
  out.mutate_lines(&(|line: &mut [f32], row| {
    for col in 0..width {
      let mut sums: [f32; 4] = [0.0;4];
      let mut counts: [u32; 4] = [0; 4];
      let color = fcol(img, row, col);

      for y in (cmp::max(0,(row as isize)-1) as usize) .. cmp::min(height, row+2) {
        for x in (cmp::max(0,(col as isize)-1) as usize) .. cmp::min(width, col+2) {
          let c = fcol(img, y+ys, x+xs);
          if c != color {
            sums[c] += img.data[(y+ys)*img.width+(x+xs)] as f32;
            counts[c] += 1;
          }
        }
      }

      for c in 0..4 {
        if c != color && counts[c] > 0 {
          line[col*4+c] = sums[c] / (counts[c] as f32);
        }
      }
    }
  }));

  out
}

pub fn scaled(img: &Image, scale: usize, xs: usize, ys: usize, width: usize, height: usize) -> OpBuffer {
  let nwidth = width/scale;
  let nheight = height/scale;
  let mut out = OpBuffer::new(nwidth, nheight, 4);

  // Go around the image averaging every block of pixels
  out.mutate_lines(&(|line: &mut [f32], row| {
    for col in 0..nwidth {
      let mut sums: [f32; 4] = [0.0;4];
      let mut counts: [u32; 4] = [0; 4];

      for y in row*scale..(row+1)*scale {
        for x in col*scale..(col+1)*scale {
          let c = fcol(img, y+ys, x+xs);
          sums[c] += img.data[(y+ys)*img.width+(x+xs)] as f32;
          counts[c] += 1;
        }
      }

      for c in 0..4 {
        if counts[c] > 0 {
          line[col*4+c] = sums[c] / (counts[c] as f32);
        }
      }
    }
  }));

  out
}
