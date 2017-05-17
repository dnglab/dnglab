extern crate rayon;
use self::rayon::prelude::*;

use decoders::RawImage;
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

/// Rotate an OpBuffer based on the given RawImage's orientation
pub fn rotate(img: &RawImage, buf: &OpBuffer) -> OpBuffer {
  match img.orientation.to_flips() {
    (false, false, false) => buf.clone(),
    (false, false, true) => flip_vertical(buf),
    (false, true, false) => flip_horizontal(buf),
    (false, true, true) => flip_horizontal(&flip_vertical(buf)),
    (true, false, false) => transpose(buf),
    (true, false, true) => flip_vertical(&transpose(buf)),
    (true, true, false) => flip_horizontal(&transpose(buf)),
    (true, true, true) => flip_vertical(&transpose(&flip_horizontal(buf))),
  }
}
