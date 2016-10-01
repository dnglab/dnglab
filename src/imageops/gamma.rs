use decoders::Image;

pub fn gamma(_: &Image, buf: &mut Vec<f32>) {
  let g: f32 = 0.45;
  let f: f32 = 0.099;
  let min: f32 = 0.018;
  let mul: f32 = 4.5;
  
  let maxvals = 72100; // 2^16 * 1.099 plus some margin
  let mut powlookup: Vec<f32> = vec![0.0; maxvals+1];
  for i in 0..(maxvals+1) {
    let v = (i as f32) / (maxvals as f32);
    powlookup[i] = v.powf(g);
  }

  for i in 0..buf.len() {
    let val = buf[i];

    buf[i] = if val <= min {
      mul * val
    } else {
      let part = (1.0+f) * val;
      let power = powlookup[(part.max(0.0)*(maxvals as f32)).min(maxvals as f32) as usize];
      power - f
    };
  }
}
