use decoders::RawImage;
use imageops::OpBuffer;

pub fn convert(img: &RawImage) -> OpBuffer {
  // Calculate the resulting width/height and top-left corner after crops
  let width = img.width - img.crops[1] - img.crops[3];
  let height = img.height - img.crops[0] - img.crops[2];
  let x = img.crops[3];
  let y = img.crops[0];

  if img.cpp == 1 && !img.cfa.is_valid() {
    // We're in a monochrome image so turn it into RGB
    let mut out = OpBuffer::new(width, height, 4);
    out.mutate_lines(&(|line: &mut [f32], row| {
      for (o, i) in line.chunks_mut(4).zip(img.data[img.width*(row+x)+y..].chunks(1)) {
        o[0] = i[0] as f32;
        o[1] = i[0] as f32;
        o[2] = i[0] as f32;
        o[3] = 0.0;
      }
    }));
    out
  } else if img.cpp == 3 {
    // We're in an RGB image, turn it into four channel
    let mut out = OpBuffer::new(width, height, 4);
    out.mutate_lines(&(|line: &mut [f32], row| {
      for (o, i) in line.chunks_mut(4).zip(img.data[img.width*(row+x)+y..].chunks(3)) {
        o[0] = i[0] as f32;
        o[1] = i[1] as f32;
        o[2] = i[2] as f32;
        o[3] = 0.0;
      }
    }));
    out
  } else {
    let mut out = OpBuffer::new(width, height, img.cpp);
    out.mutate_lines(&(|line: &mut [f32], row| {
      for (o, i) in line.chunks_mut(1).zip(img.data[img.width*(row+x)+y..].chunks(1)) {
        o[0] = i[0] as f32;
      }
    }));
    out
  }
}
