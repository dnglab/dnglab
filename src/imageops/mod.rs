extern crate rayon;
use self::rayon::prelude::*;

pub mod gofloat;
pub mod demosaic;
pub mod level;
pub mod colorspaces;
pub mod curves;
pub mod gamma;
pub mod transform;

use decoders::{Orientation, RawImage};

extern crate time;

#[derive(Debug, Clone, PartialEq)]
pub struct OpBuffer {
  pub width: usize,
  pub height: usize,
  pub colors: usize,
  pub data: Vec<f32>,
}

impl OpBuffer {
  pub fn new(width: usize, height: usize, colors: usize) -> OpBuffer {
    OpBuffer {
      width: width,
      height: height,
      colors: colors,
      data: vec![0.0; width*height*(colors as usize)],
    }
  }

  pub fn mutate_lines<F>(&mut self, closure: &F)
    where F : Fn(&mut [f32], usize)+Sync {

    self.data.par_chunks_mut(self.width*self.colors).enumerate().for_each(|(row, line)| {
      closure(line, row);
    });
  }

  pub fn process_into_new<F>(&self, colors: usize, closure: &F) -> OpBuffer
    where F : Fn(&mut [f32], &[f32])+Sync {

    let mut out = OpBuffer::new(self.width, self.height, colors);
    out.data.par_chunks_mut(out.width*out.colors).enumerate().for_each(|(row, line)| {
      closure(line, &self.data[self.width*self.colors*row..]);
    });
    out
  }

  /// Helper function to allow human readable creation of `OpBuffer` instances
  pub fn from_rgb_str_vec(data: Vec<&str>) -> OpBuffer {
    let width = data.first().expect("Invalid data for rgb helper function").len();
    let height = data.len();
    let colors = 3;

    let mut pixel_data: Vec<f32> = Vec::with_capacity(width * height * colors);
    for row in data {
      for col in row.chars() {
        let (r, g, b) = match col {
            'R' => (1.0, 0.0, 0.0),
            'G' => (0.0, 1.0, 0.0),
            'B' => (0.0, 0.0, 1.0),
            'O' => (1.0, 1.0, 1.0),
            ' ' => (0.0, 0.0, 0.0),
            c @ _ => panic!(format!(
              "Invalid color '{}' sent to rgb expected any of 'RGBO '", c)),
        };

        pixel_data.push(r);
        pixel_data.push(g);
        pixel_data.push(b);
      }
    }

    OpBuffer {
      width: width,
      height: height,
      colors: colors,
      data: pixel_data,
    }
  }
}

fn do_timing<O, F: FnMut() -> O>(name: &str, mut closure: F) -> O {
  let from_time = time::precise_time_ns();
  let ret = closure();
  let to_time = time::precise_time_ns();
  println!("{} ms for '{}'", (to_time - from_time)/1000000, name);

  ret
}

fn decoder(img: &RawImage, maxwidth: usize, maxheight: usize, linear: bool) -> OpBuffer {
  // First we check if the image's orientation results in a rotation that
  // swaps the maximum width with the maximum height
  let (transpose, ..) = img.orientation.to_flips();
  let (maxwidth, maxheight) = if transpose {
    (maxheight, maxwidth)
  } else {
    (maxwidth, maxheight)
  };

  let input = do_timing("gofloat", ||gofloat::convert(img));

  // Demosaic into 4 channel f32 (RGB or RGBE)
  let mut channel4 = do_timing("demosaic", ||demosaic::demosaic_and_scale(img, &input, maxwidth, maxheight));

  // Fix orientation if necessary and possible
  if img.orientation != Orientation::Normal && img.orientation != Orientation::Unknown {
    channel4 = do_timing("rotate", || { transform::rotate(img, &channel4) });
  }

  do_timing("level_and_balance", || { level::level_and_balance(img, &mut channel4) });
  // From now on we are in 3 channel f32 (RGB or Lab)
  let mut channel3 = do_timing("camera_to_lab", ||colorspaces::camera_to_lab(img, &channel4));
  do_timing("base_curve", ||curves::base(img, &mut channel3));
  do_timing("lab_to_rec709", ||colorspaces::lab_to_rec709(img, &mut channel3));
  if !linear {
    do_timing("gamma", ||gamma::gamma(img, &mut channel3));
  }

  channel3
}

pub fn simple_decode(img: &RawImage, maxwidth: usize, maxheight: usize) -> OpBuffer {
  decoder(img, maxwidth, maxheight, false)
}

pub fn simple_decode_linear(img: &RawImage, maxwidth: usize, maxheight: usize) -> OpBuffer {
  decoder(img, maxwidth, maxheight, true)
}
