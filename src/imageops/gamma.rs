use decoders::Image;

pub fn gamma(_: &Image, buf: &mut [f32]) {
  let g: f32 = 0.45;
  let f: f32 = 0.099;
  let min: f32 = 0.018;
  let mul: f32 = 4.5;
  
  let maxvals = 1 << 16; // 2^16 is enough precision for any output format
  let mut glookup: Vec<f32> = vec![0.0; maxvals+1];
  for i in 0..(maxvals+1) {
    let v = (i as f32) / (maxvals as f32);
    glookup[i] = if v <= min {
      mul * v
    } else {
      ((1.0+f) * v).powf(g) - f
    }
  }

  for pix in buf.chunks_mut(1) {
    pix[0] = glookup[(pix[0].max(0.0)*(maxvals as f32)).min(maxvals as f32) as usize];
  }
}
