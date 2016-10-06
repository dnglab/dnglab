pub mod demosaic;
pub mod level;
pub mod colorspaces;
pub mod curves;
pub mod gamma;

use decoders::Image;

extern crate time;

#[derive(Debug, Clone)]
pub struct OpBuffer {
  pub width: usize,
  pub height: usize,
  pub colors: u8,
  pub data: Vec<f32>,
}

impl OpBuffer {
  pub fn new(width: usize, height: usize, colors: u8) -> OpBuffer {
    OpBuffer {
      width: width,
      height: height,
      colors: colors,
      data: vec![0.0; width*height*(colors as usize)],
    }
  }
}

#[inline] pub fn fcol (img: &Image, row: usize, col: usize) -> usize {
//  let filter: [usize; 256] = [
//    2,1,1,3,2,3,2,0,3,2,3,0,1,2,1,0,
//    0,3,0,2,0,1,3,1,0,1,1,2,0,3,3,2,
//    2,3,3,2,3,1,1,3,3,1,2,1,2,0,0,3,
//    0,1,0,1,0,2,0,2,2,0,3,0,1,3,2,1,
//    3,1,1,2,0,1,0,2,1,3,1,3,0,1,3,0,
//    2,0,0,3,3,2,3,1,2,0,2,0,3,2,2,1,
//    2,3,3,1,2,1,2,1,2,1,1,2,3,0,0,1,
//    1,0,0,2,3,0,0,3,0,3,0,3,2,1,2,3,
//    2,3,3,1,1,2,1,0,3,2,3,0,2,3,1,3,
//    1,0,2,0,3,0,3,2,0,1,1,2,0,1,0,2,
//    0,1,1,3,3,2,2,1,1,3,3,0,2,1,3,2,
//    2,3,2,0,0,1,3,0,2,0,1,2,3,0,1,0,
//    1,3,1,2,3,2,3,2,0,2,0,1,1,0,3,0,
//    0,2,0,3,1,0,0,1,1,3,3,2,3,2,2,1,
//    2,1,3,2,3,1,2,1,0,3,0,2,0,2,0,2,
//    0,3,1,0,0,2,0,3,2,1,3,1,1,3,1,3
//  ];

//  match img.dcraw_filters {
//    1 => filter[(row&15)*img.width+(col&15)] as usize,
//    //9 => img.xtrans_filters[((row+600) % 6)*width + ((col+600) % 6)],
//    _ => 
(img.dcraw_filters >> (((row << 1 & 14) + (col & 1) ) << 1) & 3) as usize
//  }
}

fn do_timing<O, F: FnMut() -> O>(name: &str, mut closure: F) -> O {
  let from_time = time::precise_time_ns();
  let ret = closure();
  let to_time = time::precise_time_ns();
  println!("{} ms for '{}'", (to_time - from_time)/1000000, name);

  ret
}

pub fn simple_decode (img: &Image) -> OpBuffer {
  // Demosaic into 4 channel f32 (RGB or RGBE)
  let mut channel4 = do_timing("demosaic", ||demosaic::demosaic_and_scale(img, 200, 200));

  do_timing("level_and_balance", || { level::level_and_balance(img, &mut channel4) });
  // From now on we are in 3 channel f32 (RGB or Lab)
  let mut channel3 = do_timing("camera_to_lab", ||colorspaces::camera_to_lab(img, &channel4));
  do_timing("base_curve", ||curves::base(img, &mut channel3));
  do_timing("lab_to_rec709", ||colorspaces::lab_to_rec709(img, &mut channel3));
  do_timing("gamma", ||gamma::gamma(img, &mut channel3));

  channel3
}
