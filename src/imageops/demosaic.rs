use decoders::RawImage;
use decoders::cfa::CFA;
use imageops::*;
use std::cmp;

#[derive(Clone, Debug, Serialize, Deserialize, Hash)]
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
  fn to_settings(&self) -> String {standard_to_settings(self)}
  fn hash(&self, hasher: &mut MetroHash) {standard_hash(self, hasher)}
  fn run(&self, pipeline: &mut PipelineGlobals, inid: u64, outid: u64) {
    let buf = pipeline.cache.get(inid).unwrap();

    let (scale, nwidth, nheight) = if pipeline.maxwidth == 0 || pipeline.maxheight == 0 {
      (0.0, buf.width, buf.height)
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

    // If we want full size and the image is already 4 color pass it through
    if (scale < minscale || buf.colors != 1) && buf.colors == 4 {
      pipeline.cache.alias(inid, outid);
    // If we're not scaling enough do full demosaic and then scale down
    } else if scale < minscale  {
      let fullsize = full(cfa, &buf);
      let buf = if scale > 1.0 {
        scale_down(&fullsize, nwidth, nheight)
      } else {
        fullsize
      };
      pipeline.cache.put(outid, buf, 1);
    // Do a demosaic by scaling down
    } else {
      let buf = scaled(cfa, &buf, nwidth, nheight);
      pipeline.cache.put(outid, buf, 1);
    }
  }
}

pub fn full(cfa: CFA, buf: &OpBuffer) -> OpBuffer {
  let mut out = OpBuffer::new(buf.width, buf.height, 4);

  // First we set the colors we already have
  out.mutate_lines(&(|line: &mut [f32], row| {
    for (col, (pixout, pixin)) in line.chunks_mut(4).zip(buf.data[buf.width*row..].chunks(1)).enumerate() {
      let color = cfa.color_at(row, col);
      pixout[color] = pixin[0] as f32;
    }
  }));

  // Now we go around the image setting the unset colors to the average of the
  // surrounding pixels
  out.mutate_lines(&(|line: &mut [f32], row| {
    for col in 0..buf.width {
      let mut sums: [f32; 4] = [0.0;4];
      let mut counts: [u32; 4] = [0; 4];
      let color = cfa.color_at(row, col);

      for y in (cmp::max(0,(row as isize)-1) as usize) .. cmp::min(buf.height, row+2) {
        for x in (cmp::max(0,(col as isize)-1) as usize) .. cmp::min(buf.width, col+2) {
          let c = cfa.color_at(y, x);
          if c != color {
            sums[c] += buf.data[y*buf.width+x] as f32;
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
