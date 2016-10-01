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
  let leveled = level::level(img);
  let wbed = whitebalance::whitebalance(img, &leveled);
  let demosaic = demosaic::ppg(img, &wbed);
  let lab = colorspaces::camera_to_lab(img, &demosaic);
  let curved = curves::base(img, &lab);
  let rgb = colorspaces::lab_to_rec709(img, &curved);
  let gamma = gamma::gamma(img, &rgb);

  gamma
}
