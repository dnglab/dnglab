extern crate rayon;
use self::rayon::prelude::*;

use decoders::{Orientation, RawImage};
use imageops::OpBuffer;

/// Mirror an OpBuffer horizontally
pub fn flip_horizontal(buf: &OpBuffer) -> OpBuffer {
  let mut out = OpBuffer::new(buf.width, buf.height, buf.colors);
  out.data.par_chunks_mut(out.width * out.colors).enumerate().for_each(|(row, line)| {
    let offset = buf.width * row * buf.colors;
    for col in 0 .. buf.width {
      for c in 0 .. buf.colors {
        line[col * buf.colors + c] = buf.data[offset + (buf.width - 1 - col) * buf.colors + c];
      }
    }
  });

  out
}

/// Mirror an OpBuffer vertically
pub fn flip_vertical(buf: &OpBuffer) -> OpBuffer {
  let mut out = OpBuffer::new(buf.width, buf.height, buf.colors);
  out.data.par_chunks_mut(out.width * out.colors).enumerate().for_each(|(row, line)| {
    let offset = (buf.height - 1 - row) * buf.width * buf.colors;
    for col in 0 .. buf.width * buf.colors {
      line[col] = buf.data[offset + col];
    }
  });

  out
}

/// Transpose an OpBuffer
pub fn transpose(buf: &OpBuffer) -> OpBuffer {
  let mut out = OpBuffer::new(buf.height, buf.width, buf.colors);

  out.data.par_chunks_mut(out.width * out.colors).enumerate().for_each(|(row, line)| {
    for col in 0 .. buf.height {
      let target = col * buf.colors;
      let source = (col * buf.width + row) * buf.colors;
      for c in 0 .. buf.colors {
        line[target + c] = buf.data[source + c];
      }
    }
  });

  out
}

fn rotate_buffer(buf: &OpBuffer, orientation: &Orientation) -> OpBuffer {
  match orientation.to_flips() {
    (false, false, false) => buf.clone(),
    (false, false, true) => flip_vertical(buf),
    (false, true, false) => flip_horizontal(buf),
    (false, true, true) => flip_horizontal(&flip_vertical(buf)),
    (true, false, false) => transpose(buf),
    (true, false, true) => flip_vertical(&transpose(buf)),
    (true, true, false) => flip_horizontal(&transpose(buf)),
    (true, true, true) => flip_vertical(&flip_horizontal(&transpose(buf))),
  }
}

/// Rotate an OpBuffer based on the given RawImage's orientation
pub fn rotate(img: &RawImage, buf: &OpBuffer) -> OpBuffer {
  rotate_buffer(buf, &img.orientation)
}

#[cfg(test)]
mod tests {
  use decoders::Orientation;
  use imageops::OpBuffer;
  use super::rotate_buffer;

  /// Helper function to allow human readable creation of `OpBuffer` instances
  fn rgb(data: Vec<&str>) -> OpBuffer {
    let width = data.first().expect("Invalid data for rgb helper function").len();
    let height = data.len();
    let colors = 3;

    let mut pixel_data: Vec<f32> = Vec::with_capacity(width * height * colors);
    for row in data {
      for col in row.chars() {
        let (r, g, b) = match col {
            'R' => (1.0, 0.0, 0.0),
            'G' => (0.0, 1.0, 0.0),
            'B' => (0.0, 0.0, 1.0),
            'O' => (1.0, 1.0, 1.0),
            ' ' => (0.0, 0.0, 0.0),
            c @ _ => panic!(format!(
              "Invalid color '{}' sent to rgb expected any of 'RGBO '", c)),
        };

        pixel_data.push(r);
        pixel_data.push(g);
        pixel_data.push(b);
      }
    }

    OpBuffer {
      width: width,
      height: height,
      colors: colors,
      data: pixel_data,
    }
  }


  // Store a colorful capital F as a constant, since it is used in all tests
  lazy_static! {
      static ref F: OpBuffer = {
        rgb(vec![
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
    let output = rgb(vec![
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
    let output = rgb(vec![
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
    let output = rgb(vec![
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
    let output = rgb(vec![
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
  fn rotate_transpose() {
    let output = rgb(vec![
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
    let output = rgb(vec![
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
