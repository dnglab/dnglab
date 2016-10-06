use decoders::Image;
use imageops::OpBuffer;

pub fn float(img: &Image) -> OpBuffer {
  let mut out = OpBuffer::new(img.width, img.height, 1);

  for (pixin,pixout) in img.data.chunks(1).zip(out.data.chunks_mut(1)) {
    pixout[0] = (pixin[0] as f32) / 65535.0;
  }

  out
}
