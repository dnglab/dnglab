use std::mem;
use std::usize;

use decoders::{Orientation, RawImage};
use imageops::{OpBuffer,ImageOp,Pipeline};

#[derive(Copy, Clone, Debug)]
pub struct OpTransform {
  orientation: Orientation,
}

impl OpTransform {
  pub fn new(img: &RawImage) -> OpTransform {
    OpTransform{
      orientation: img.orientation,
    }
  }
}

impl ImageOp for OpTransform {
  fn name(&self) -> &str {"transform"}
  fn run(&self, _pipeline: &Pipeline, buf: &OpBuffer) -> OpBuffer {
    rotate_buffer(buf, &self.orientation)
  }
}

fn rotate_buffer(buf: &OpBuffer, orientation: &Orientation) -> OpBuffer {
  // Don't rotate things we don't know how to rotate or don't need to
  if *orientation == Orientation::Normal || *orientation == Orientation::Unknown {
    return buf.clone();
  }

  // Since we are using isize when calculating values for the rotation its
  // indices must be addressable by an isize as well
  if buf.data.len() >= usize::MAX / 2 {
    panic!("Buffer is too wide or high to rotate");
  }

  // We extract buffers parameters early since all math is done with isize.
  // This avoids verbose casts later on
  let mut width = buf.width as isize;
  let mut height = buf.height as isize;
  let colors = buf.colors as isize;

  let (transpose, flip_x, flip_y) = orientation.to_flips();

  let mut base_offset: isize = 0;
  let mut x_step: isize = colors;
  let mut y_step: isize = width * colors;

  if flip_x {
    x_step = -x_step;
    base_offset += (width - 1) * colors;
  }

  if flip_y {
    y_step = -y_step;
    base_offset += width * (height - 1) * colors;
  }

  let mut out = if transpose {
    mem::swap(&mut width, &mut height);
    mem::swap(&mut x_step, &mut y_step);
    OpBuffer::new(buf.height, buf.width, colors as usize)
  } else {
    OpBuffer::new(buf.width, buf.height, colors as usize)
  };

  out.mutate_lines(&(|line: &mut [f32], row| {
    // Calculate the current line's offset in original buffer. When transposing
    // this is the current column's offset in the original buffer
    let line_offset = base_offset + y_step * row as isize;
    for col in 0..width {
      // The current pixel's offset in original buffer
      let offset = line_offset + x_step * col;
      for c in 0..colors {
        line[(col * colors + c) as usize] = buf.data[(offset + c) as usize];
      }
    }
  }));

  out
}

#[cfg(test)]
mod tests {
  use decoders::Orientation;
  use imageops::OpBuffer;
  use super::rotate_buffer;

  // Store a colorful capital F as a constant, since it is used in all tests
  lazy_static! {
      static ref F: OpBuffer = {
        OpBuffer::from_rgb_str_vec(vec![
          "        ",
          " RRRRRR ",
          " GG     ",
          " BBBB   ",
          " GG     ",
          " GG     ",
          "        ",
        ])
      };
  }

  #[test]
  fn rotate_unknown() {
    assert_eq!(rotate_buffer(&F.clone(), &Orientation::Unknown), F.clone());
  }

  #[test]
  fn rotate_normal() {
    assert_eq!(rotate_buffer(&F.clone(), &Orientation::Normal), F.clone());
  }

  #[test]
  fn rotate_flip_x() {
    let output = OpBuffer::from_rgb_str_vec(vec![
      "        ",
      " RRRRRR ",
      "     GG ",
      "   BBBB ",
      "     GG ",
      "     GG ",
      "        ",
    ]);

    assert_eq!(rotate_buffer(&F.clone(), &Orientation::HorizontalFlip), output);
  }

  #[test]
  fn rotate_flip_y() {
    let output = OpBuffer::from_rgb_str_vec(vec![
      "        ",
      " GG     ",
      " GG     ",
      " BBBB   ",
      " GG     ",
      " RRRRRR ",
      "        ",
    ]);
    assert_eq!(rotate_buffer(&F.clone(), &Orientation::VerticalFlip), output);
  }

  #[test]
  fn rotate_rotate90_cw() {
    let output = OpBuffer::from_rgb_str_vec(vec![
      "       ",
      " GGBGR ",
      " GGBGR ",
      "   B R ",
      "   B R ",
      "     R ",
      "     R ",
      "       ",
    ]);
    assert_eq!(rotate_buffer(&F.clone(), &Orientation::Rotate90), output);
  }

  #[test]
  fn rotate_rotate270_cw() {
    let output = OpBuffer::from_rgb_str_vec(vec![
      "       ",
      " R     ",
      " R     ",
      " R B   ",
      " R B   ",
      " RGBGG ",
      " RGBGG ",
      "       ",
    ]);
    assert_eq!(rotate_buffer(&F.clone(), &Orientation::Rotate270), output);
  }

  #[test]
  fn rotate_rotate180() {
    let output = OpBuffer::from_rgb_str_vec(vec![
      "        ",
      "     GG ",
      "     GG ",
      "   BBBB ",
      "     GG ",
      " RRRRRR ",
      "        ",
    ]);
    assert_eq!(rotate_buffer(&F.clone(), &Orientation::Rotate180), output);
  }

  #[test]
  fn rotate_transpose() {
    let output = OpBuffer::from_rgb_str_vec(vec![
      "       ",
      " RGBGG ",
      " RGBGG ",
      " R B   ",
      " R B   ",
      " R     ",
      " R     ",
      "       ",
    ]);
    assert_eq!(rotate_buffer(&F.clone(), &Orientation::Transpose), output);
  }

  #[test]
  fn rotate_transverse() {
    let output = OpBuffer::from_rgb_str_vec(vec![
      "       ",
      "     R ",
      "     R ",
      "   B R ",
      "   B R ",
      " GGBGR ",
      " GGBGR ",
      "       ",
    ]);
    assert_eq!(rotate_buffer(&F.clone(), &Orientation::Transverse), output);
  }
}
