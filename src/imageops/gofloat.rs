use decoders::RawImage;
use imageops::*;

#[derive(Copy, Clone, Debug, Serialize, Deserialize, Hash)]
pub struct OpGoFloat {
  width: usize,
  height: usize,
  x: usize,
  y: usize,
  cpp: usize,
  is_cfa: bool,
}

impl OpGoFloat {
  pub fn new(img: &RawImage) -> OpGoFloat {
    // Calculate the resulting width/height and top-left corner after crops
    OpGoFloat{
      width: img.width - img.crops[1] - img.crops[3],
      height: img.height - img.crops[0] - img.crops[2],
      x: img.crops[3],
      y: img.crops[0],
      cpp: img.cpp,
      is_cfa: img.cfa.is_valid(),
    }
  }
}

impl<'a> ImageOp<'a> for OpGoFloat {
  fn name(&self) -> &str {"gofloat"}
  fn run(&self, pipeline: &mut PipelineGlobals, _inid: u64, outid: u64) {
    let img = pipeline.image;
    let x = self.x;
    let y = self.y;

    let buf = if self.cpp == 1 && !self.is_cfa {
      // We're in a monochrome image so turn it into RGB
      let mut out = OpBuffer::new(self.width, self.height, 4);
      out.mutate_lines(&(|line: &mut [f32], row| {
        for (o, i) in line.chunks_mut(4).zip(img.data[img.width*(row+y)+x..].chunks(1)) {
          o[0] = i[0] as f32;
          o[1] = i[0] as f32;
          o[2] = i[0] as f32;
          o[3] = 0.0;
        }
      }));
      out
    } else if self.cpp == 3 {
      // We're in an RGB image, turn it into four channel
      let mut out = OpBuffer::new(self.width, self.height, 4);
      out.mutate_lines(&(|line: &mut [f32], row| {
        for (o, i) in line.chunks_mut(4).zip(img.data[img.width*(row+y)+x..].chunks(3)) {
          o[0] = i[0] as f32;
          o[1] = i[1] as f32;
          o[2] = i[2] as f32;
          o[3] = 0.0;
        }
      }));
      out
    } else {
      let mut out = OpBuffer::new(self.width, self.height, img.cpp);
      out.mutate_lines(&(|line: &mut [f32], row| {
        for (o, i) in line.chunks_mut(1).zip(img.data[img.width*(row+y)+x..].chunks(1)) {
          o[0] = i[0] as f32;
        }
      }));
      out
    };

    pipeline.cache.put(outid, buf, 1);
  }
}
