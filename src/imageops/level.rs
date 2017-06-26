use decoders::RawImage;
use imageops::*;
extern crate ordered_float;
use self::ordered_float::OrderedFloat;

#[derive(Copy, Clone, Debug, Serialize, Deserialize, Hash)]
pub struct OpLevel {
  blacklevels: [OrderedFloat<f32>;4],
  whitelevels: [OrderedFloat<f32>;4],
  wb_coeffs: [OrderedFloat<f32>;4],
}

fn from_int4(arr: [u16;4]) -> [OrderedFloat<f32>;4] {
  [OrderedFloat(arr[0] as f32), OrderedFloat(arr[1] as f32),
   OrderedFloat(arr[2] as f32), OrderedFloat(arr[3] as f32)]
}

fn from_float4(arr: [f32;4]) -> [OrderedFloat<f32>;4] {
  [OrderedFloat(arr[0]), OrderedFloat(arr[1]), OrderedFloat(arr[2]), OrderedFloat(arr[3])]
}

fn from_ordered4(arr: [OrderedFloat<f32>;4]) -> [f32;4] {
  [arr[0].into(), arr[1].into(), arr[2].into(), arr[3].into()]
}

impl OpLevel {
  pub fn new(img: &RawImage) -> OpLevel {
    let coeffs = if img.is_monochrome() {
      [1.0, 1.0, 1.0, 1.0]
    } else if !img.wb_coeffs[0].is_normal() ||
              !img.wb_coeffs[1].is_normal() ||
              !img.wb_coeffs[2].is_normal() {
      img.neutralwb()
    } else {
      img.wb_coeffs
    };

    OpLevel{
      blacklevels: from_int4(img.blacklevels),
      whitelevels: from_int4(img.whitelevels),
      wb_coeffs: from_float4(coeffs),
    }
  }
}

impl<'a> ImageOp<'a> for OpLevel {
  fn name(&self) -> &str {"level"}
  fn run(&self, pipeline: &mut PipelineGlobals, inid: u64, outid: u64) {
    let mut buf = (*pipeline.cache.get(inid).unwrap()).clone();

    // Calculate the levels
    let mins = from_ordered4(self.blacklevels);
    let ranges = self.whitelevels.iter().enumerate().map(|(i, &x)| {
      let x: f32 = (*x).into();
      x - mins[i]
    }).collect::<Vec<f32>>();

    // Set green multiplier as 1.0
    let unity: f32 = self.wb_coeffs[1].into();
    let mul = self.wb_coeffs.iter().map(|x| {
      if !x.is_normal() { 
        1.0 
      } else { 
        let x: f32 = (*x).into();
        x / unity 
      }
    }).collect::<Vec<f32>>();

    buf.mutate_lines(&(|line: &mut [f32], _| {
      for pix in line.chunks_mut(4) {
        pix[0] = (((pix[0] - mins[0]) / ranges[0]) * mul[0]).min(1.0);
        pix[1] = (((pix[1] - mins[1]) / ranges[1]) * mul[1]).min(1.0);
        pix[2] = (((pix[2] - mins[2]) / ranges[2]) * mul[2]).min(1.0);
        pix[3] = (((pix[3] - mins[3]) / ranges[3]) * mul[3]).min(1.0);
      }
    }));

    pipeline.cache.put(outid, buf, 1);
  }
}
