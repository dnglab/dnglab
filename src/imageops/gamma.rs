use decoders::RawImage;
use imageops::*;

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct OpGamma {
}

impl<'a> OpGamma {
  pub fn new(_img: &RawImage) -> OpGamma {
    OpGamma{}
  }
}

impl<'a> ImageOp<'a> for OpGamma {
  fn name(&self) -> &str {"gamma"}
  fn run(&self, pipeline: &mut PipelineGlobals, inid: BufHash, outid: BufHash) {
    if pipeline.linear {
      pipeline.cache.alias(inid, outid);
    } else {
      let mut buf = (*pipeline.cache.get(inid).unwrap()).clone();

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
          ((1.0+f) * v.powf(g)) - f
        }
      }

      buf.mutate_lines(&(|line: &mut [f32], _| {
        for pix in line.chunks_mut(1) {
          pix[0] = glookup[(pix[0].max(0.0)*(maxvals as f32)).min(maxvals as f32) as usize];
        }
      }));

      pipeline.cache.put(outid, buf, 1);
    }
  }
}
