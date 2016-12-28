use decoders::Image;
use imageops::OpBuffer;
use std::cmp;

pub fn demosaic_and_scale(img: &Image, maxwidth: usize, maxheight: usize) -> OpBuffer {
  // Calculate the resulting width/height and top-left corner after crops
  let width = img.width - img.crops[1] - img.crops[3];
  let height = img.height - img.crops[0] - img.crops[2];
  let x = img.crops[3];
  let y = img.crops[0];

  let (scale, nwidth, nheight) = if maxwidth == 0 || maxheight == 0 {
    (0.0, width, height)
  } else {
    // Do the calculations manually to avoid off-by-one errors from floating point rounding
    let xscale = (width as f32) / (maxwidth as f32);
    let yscale = (height as f32) / (maxheight as f32);
    if yscale > xscale {
      (yscale, ((width as f32)/yscale) as usize, maxheight)
    } else {
      (xscale, maxwidth, ((height as f32)/xscale) as usize)
    }
  };

  let minscale = match img.cfa.width() {
    2  => 2.0,  // RGGB/RGBE bayer
    6  => 3.0,  // x-trans is 6 wide but has all colors in every 3x3 block
    12 => 12.0, // some crazy sensor I haven't actually encountered, use full block
    _  => 2.0,  // default
  };

  if scale < minscale {
    let buf = full(img, x, y, width, height);
    if scale > 1.0 {
      scale_down(&buf, nwidth, nheight)
    } else {
      buf
    }
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

fn calc_skips(idx: usize, idxmax: usize, skip: f32) -> (usize, usize, f32, f32) {
  let from = (idx as f32)*skip;
  let fromback = from.floor();
  let fromfactor = 1.0 - (from-fromback).fract();

  let to = ((idx+1) as f32)*skip;
  let toforward = (idxmax as f32).min(to.ceil());
  let tofactor = (toforward-to).fract();

  (fromback as usize, toforward as usize, fromfactor, tofactor)
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
      let (fromrow, torow, topfactor, bottomfactor) = calc_skips(row, height, rowskip);
      for y in fromrow..torow {
        let (fromcol, tocol, leftfactor, rightfactor) = calc_skips(col, width, colskip);
        for x in fromcol..tocol {
          let factor = {
            (if y == fromrow {topfactor} else if y == torow {bottomfactor} else {1.0}) *
            (if x == fromcol {leftfactor} else if x == tocol {rightfactor} else {1.0})
          };

          let c = crop_cfa.color_at(y, x);
          sums[c] += (img.data[(y+ys)*img.width+(x+xs)] as f32) * factor;
          counts[c] += factor;
        }
      }

      for c in 0..4 {
        if counts[c] > 0.0 {
          line[col*4+c] = sums[c] / counts[c];
        }
      }
    }
  }));

  out
}

pub fn scale_down(buf: &OpBuffer, nwidth: usize, nheight: usize) -> OpBuffer {
  let mut out = OpBuffer::new(nwidth, nheight, buf.colors);
  let rowskip = (buf.width as f32) / (nwidth as f32);
  let colskip = (buf.height as f32) / (nheight as f32);

  // Go around the image averaging blocks of pixels
  out.mutate_lines(&(|line: &mut [f32], row| {
    for col in 0..nwidth {
      let mut sums: [f32; 4] = [0.0;4];
      let mut counts: [f32; 4] = [0.0;4];
      let (fromrow, torow, topfactor, bottomfactor) = calc_skips(row, buf.height, rowskip);
      for y in fromrow..torow {
        let (fromcol, tocol, leftfactor, rightfactor) = calc_skips(col, buf.width, colskip);
        for x in fromcol..tocol {
          let factor = {
            (if y == fromrow {topfactor} else if y == torow {bottomfactor} else {1.0}) *
            (if x == fromcol {leftfactor} else if x == tocol {rightfactor} else {1.0})
          };

          for c in 0..buf.colors {
            sums[c] += buf.data[(y*buf.width+x)*buf.colors + c] * factor;
            counts[c] += factor;
          }
        }
      }

      for c in 0..buf.colors {
        if counts[c] > 0.0 {
          line[col*buf.colors+c] = sums[c] / counts[c];
        }
      }
    }
  }));

  out
}
