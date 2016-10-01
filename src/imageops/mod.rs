pub mod demosaic;
pub mod whitebalance;
pub mod level;
pub mod gamma;
pub mod curves;
pub mod colorspaces;

use decoders::Image;

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

pub fn simple_decode (img: &Image) -> Vec<f32> {
  // Start with a 1 channel f32 (pre-demosaic)
  let mut channel1 = level::level(img);
  whitebalance::whitebalance(img, &mut channel1);
  // Demosaic into 4 channel f32 (RGB or RGBE)
  let channel4 = demosaic::ppg(img, &channel1);
  // From now on we are in 3 channel f32 (RGB or Lab)
  let mut channel3 = colorspaces::camera_to_lab(img, &channel4);
  curves::base(img, &mut channel3);
  colorspaces::lab_to_rec709(img, &mut channel3);
  gamma::gamma(img, &mut channel3);

  channel3
}
