use decoders::RawImage;
use decoders::cfa::CFA;
use imageops::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpDemosaic {
  cfa: String,
}

impl OpDemosaic {
  pub fn new(img: &RawImage) -> OpDemosaic {
    OpDemosaic{
      cfa: img.cropped_cfa().to_string(),
    }
  }
}

impl<'a> ImageOp<'a> for OpDemosaic {
  fn name(&self) -> &str {"demosaic"}
  fn run(&self, pipeline: &mut PipelineGlobals, inid: BufHash, outid: BufHash) {
    let buf = pipeline.cache.get(&inid).unwrap();

    let (scale, nwidth, nheight) = if pipeline.maxwidth == 0 || pipeline.maxheight == 0 {
      (1.0, buf.width, buf.height)
    } else {
      // Do the calculations manually to avoid off-by-one errors from floating point rounding
      let xscale = (buf.width as f32) / (pipeline.maxwidth as f32);
      let yscale = (buf.height as f32) / (pipeline.maxheight as f32);
      if yscale > xscale {
        (yscale, ((buf.width as f32)/yscale) as usize, pipeline.maxheight)
      } else {
        (xscale, pipeline.maxwidth, ((buf.height as f32)/xscale) as usize)
      }
    };

    let cfa = CFA::new(&self.cfa);
    let minscale = match cfa.width {
      2  => 2.0,  // RGGB/RGBE bayer
      6  => 3.0,  // x-trans is 6 wide but has all colors in every 3x3 block
      8  => 2.0,  // Canon pro 70 has a 8x2 patern that has all four colors every 2x2 block
      12 => 12.0, // some crazy sensor I haven't actually encountered, use full block
      _  => 2.0,  // default
    };

    if scale <= 1.0 && buf.colors == 4 {
      // We want full size and the image is already 4 color, pass it through
      pipeline.cache.alias(inid, outid);
    } else if buf.colors == 4 {
      // Scale down a 4 colour image
      let buf = scale_down(&buf, nwidth, nheight);
      pipeline.cache.put(outid, buf, 1);
    } else if scale >= minscale {
      // We're scaling down enough that each pixel has all four colors under it so do the
      // demosaic and scale down in one go
      let buf = scaled(cfa, &buf, nwidth, nheight);
      pipeline.cache.put(outid, buf, 1);
    } else {
      // We're in a close to full scale output that needs full demosaic and possibly
      // minimal scale down
      let fullsize = full(cfa, &buf);
      let buf = if scale > 1.0 {
        scale_down(&fullsize, nwidth, nheight)
      } else {
        fullsize
      };
      pipeline.cache.put(outid, buf, 1);
    }
  }
}

pub fn full(cfa: CFA, buf: &OpBuffer) -> OpBuffer {
  let mut out = OpBuffer::new(buf.width, buf.height, 4);

  let offsets33: [(isize,isize);9] = [
    (-1,-1), (-1, 0), (-1, 1),
    ( 0,-1), ( 0, 0), ( 0, 1),
    ( 1,-1), ( 1, 0), ( 1, 1),
  ];

  // Initialize a lookup table for the colors of each pixel in a 3x3 grid
  let mut lookups = [[[0;9];48];48];
  for (row, line) in lookups.iter_mut().enumerate() {
    for (col, colors) in line.iter_mut().enumerate() {
      let pixcolor = cfa.color_at(row, col);

      for (i, o) in offsets33.iter().enumerate() {
        let (dy, dx) = *o;
        let row = (48+dy) as usize + row;
        let col = (48+dx) as usize + col;
        let ocolor = cfa.color_at(row, col);
        colors[i] = if ocolor != pixcolor || (dx == 0 && dy == 0) { ocolor } else { 4 };
      }
      //println!("pixcolor {} colors is {:?}", pixcolor, colors);
    }
  }

  // Now calculate RGBE for each pixel based on the lookup table
  out.mutate_lines(&(|line: &mut [f32], row| {
    for (col, pix) in line.chunks_mut(4).enumerate() {
      let ref colors = lookups[row%48][col%48];
      let mut sums = [0f32;5];
      let mut counts = [0f32;5];

      for (i, o) in offsets33.iter().enumerate() {
        let (dy, dx) = *o;
        let row = row as isize + dy;
        let col = col as isize + dx;
        if row >= 0 && row < (buf.height as isize) &&
           col >= 0 && col < (buf.width as isize) {
          sums[colors[i]] += buf.data[(row as usize)*buf.width+(col as usize)];
          counts[colors[i]] += 1.0;
        }
      }

      for c in 0..4 {
        if counts[c] > 0.0 {
          pix[c] = sums[c] / counts[c];
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

pub fn scaled(cfa: CFA, buf: &OpBuffer, nwidth: usize, nheight: usize) -> OpBuffer {
  let mut out = OpBuffer::new(nwidth, nheight, 4);

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

          let c = cfa.color_at(y, x);
          sums[c] += (buf.data[y*buf.width+x] as f32) * factor;
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
  assert_eq!(buf.colors, 4); // When we're scaling down we're always at 4 cpp

  let mut out = OpBuffer::new(nwidth, nheight, 4);
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

          for c in 0..4 {
            sums[c] += buf.data[(y*buf.width+x)*4 + c] * factor;
            counts[c] += factor;
          }
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
