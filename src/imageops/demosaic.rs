use decoders::Image;
use imageops::OpBuffer;
use std::cmp;

pub fn demosaic_and_scale(img: &Image, minwidth: usize, minheight: usize) -> OpBuffer {
  // Calculate the resulting width/height and top-left corner after crops
  let width = (img.width as i64 - img.crops[1] - img.crops[3]) as usize;
  let height = (img.height as i64 - img.crops[0] - img.crops[2]) as usize;
  let x = img.crops[3] as usize;
  let y = img.crops[0] as usize;

  let (scale, nwidth, nheight) = if minwidth == 0 || minheight == 0 {
    (0.0, width, height)
  } else {
    // Do the calculations manually to avoid off-by-one errors from floating point rounding
    let xscale = (width as f32) / (minwidth as f32);
    let yscale = (height as f32) / (minheight as f32);
    if yscale > xscale {
      (yscale, ((width as f32)/yscale) as usize, minheight)
    } else {
      (xscale, minwidth, ((height as f32)/xscale) as usize)
    }
  };

  if scale < 2.0 {
    full(img, x, y, width, height)
  } else {
    scaled(img, x, y, width, height, nwidth, nheight)
  }
}

pub fn full(img: &Image, xs: usize, ys: usize, width: usize, height: usize) -> OpBuffer {
  let mut out = OpBuffer::new(width, height, 4);
  let crop_cfa = img.cfa.shift(xs, ys);

  // First we set the colors we already have
  out.mutate_lines(&(|line: &mut [f32], row| {
    for (col, (pixout, pixin)) in line.chunks_mut(4).zip(img.data[img.width*(row+ys)+xs..].chunks(1)).enumerate() {
      let color = crop_cfa.color_at(row, col);
      pixout[color] = pixin[0] as f32;
    }
  }));

  // Now we go around the image setting the unset colors to the average of the
  // surrounding pixels
  out.mutate_lines(&(|line: &mut [f32], row| {
    for col in 0..width {
      let mut sums: [f32; 4] = [0.0;4];
      let mut counts: [u32; 4] = [0; 4];
      let color = crop_cfa.color_at(row, col);

      for y in (cmp::max(0,(row as isize)-1) as usize) .. cmp::min(height, row+2) {
        for x in (cmp::max(0,(col as isize)-1) as usize) .. cmp::min(width, col+2) {
          let c = crop_cfa.color_at(y, x);
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

pub fn scaled(img: &Image, xs: usize, ys: usize, width: usize, height: usize, nwidth: usize, nheight: usize) -> OpBuffer {
  let mut out = OpBuffer::new(nwidth, nheight, 4);
  let crop_cfa = img.cfa.shift(xs, ys);

  let rowskip = (width as f32) / (nwidth as f32);
  let colskip = (height as f32) / (nheight as f32);

  // Go around the image averaging blocks of pixels
  out.mutate_lines(&(|line: &mut [f32], row| {
    for col in 0..nwidth {
      let mut sums: [f32; 4] = [0.0;4];
      let mut counts: [f32; 4] = [0.0;4];

      let fromrow = ((row as f32)*rowskip).floor() as usize;
      let torow = cmp::min(height, (((row+1) as f32)*rowskip).ceil() as usize);
      for y in fromrow..torow {
        let fromcol = ((col as f32)*colskip).floor() as usize;
        let tocol = cmp::min(width, (((col+1) as f32)*colskip).ceil() as usize);
        for x in fromcol..tocol {
          let c = crop_cfa.color_at(y, x);
          sums[c] += img.data[(y+ys)*img.width+(x+xs)] as f32;
          counts[c] += 1.0;
        }
      }

      for c in 0..4 {
        if counts[c] > 0.0 {
          line[col*4+c] = sums[c] / (counts[c] as f32);
        }
      }
    }
  }));

  out
}
