use decoders::ljpeg::*;
use decoders::basics::*;
use itertools::Itertools;

pub fn decode_ljpeg_2components(ljpeg: &LjpegDecompressor, out: &mut [u16], x: usize, stripwidth:usize, width: usize, height: usize) -> Result<(),String> {
  if ljpeg.sof.width*2 < width || ljpeg.sof.height < height {
    return Err(format!("ljpeg: trying to decode {}x{} into {}x{}",
                       ljpeg.sof.width*2, ljpeg.sof.height,
                       width, height).to_string())
  }
  let ref htable1 = ljpeg.dhts[ljpeg.sof.components[0].dc_tbl_num];
  let ref htable2 = ljpeg.dhts[ljpeg.sof.components[1].dc_tbl_num];
  let mut pump = BitPumpJPEG::new(ljpeg.buffer);

  let base_prediction = 1 << (ljpeg.sof.precision - ljpeg.point_transform -1);
  out[x]   = (base_prediction + try!(htable1.huff_decode(&mut pump))) as u16;
  out[x+1] = (base_prediction + try!(htable2.huff_decode(&mut pump))) as u16;
  let skip_x = ljpeg.sof.width - width/2;

  for row in 0..height {
    let startcol = if row == 0 {x+2} else {x};
    for col in (startcol..(width+x)).step(2) {
      let (p1,p2) = if col == x {
        // At start of line predictor starts with start of previous line
        (out[(row-1)*stripwidth+x],out[(row-1)*stripwidth+1+x])
      } else {
        // All other cases use the two previous pixels in the same line
        (out[row*stripwidth+col-2], out[row*stripwidth+col-1])
      };

      let diff1 = try!(htable1.huff_decode(&mut pump));
      let diff2 = try!(htable2.huff_decode(&mut pump));
      out[row*stripwidth+col] = ((p1 as i32) + diff1) as u16;
      out[row*stripwidth+col+1] = ((p2 as i32) + diff2) as u16;
    }
    // Skip extra encoded differences if the ljpeg frame is wider than the output
    for _ in 0..skip_x {
      try!(htable1.huff_decode(&mut pump));
      try!(htable2.huff_decode(&mut pump));
    }
  }

  Ok(())
}

pub fn decode_hasselblad(ljpeg: &LjpegDecompressor, out: &mut [u16], width: usize) -> Result<(),String> {
  // Pixels are packed two at a time, not like LJPEG:
  // [p1_length_as_huffman][p2_length_as_huffman][p0_diff_with_length][p1_diff_with_length]|NEXT PIXELS
  let mut pump = BitPumpMSB32::new(ljpeg.buffer);
  let ref htable = ljpeg.dhts[ljpeg.sof.components[0].dc_tbl_num];

  for line in out.chunks_mut(width) {
    let mut p1: i32 = 0x8000;
    let mut p2: i32 = 0x8000;
    for o in line.chunks_mut(2) {
      let len1 = try!(htable.huff_len(&mut pump)) as u32;
      let len2 = try!(htable.huff_len(&mut pump)) as u32;
      p1 += htable.huff_diff(&mut pump, len1);
      p2 += htable.huff_diff(&mut pump, len2);
      //println!("Len is {} {} p1 {} p2 {}", len1, len2, p1, p2);
      o[0] = p1 as u16;
      o[1] = p2 as u16;
    }
  }

  Ok(())
}
