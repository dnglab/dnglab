use decoders::ljpeg::*;
use decoders::basics::*;
use itertools::Itertools;

pub fn decode_ljpeg_2components(ljpeg: &LjpegDecompressor) -> Result<Vec<u16>,String> {
  let mut out: Vec<u16> = vec![0; ljpeg.width*ljpeg.height];

  let ref htable1 = ljpeg.dhts[ljpeg.sof.components[0].dc_tbl_num];
  let ref htable2 = ljpeg.dhts[ljpeg.sof.components[1].dc_tbl_num];
  let mut pump = BitPumpJPEG::new(ljpeg.buffer);

  let base_prediction = 1 << (ljpeg.sof.precision - ljpeg.point_transform -1);
  out[0] = (base_prediction + try!(htable1.huff_decode(&mut pump))) as u16;
  out[1] = (base_prediction + try!(htable2.huff_decode(&mut pump))) as u16;

  for row in 0..ljpeg.height {
    let startcol = if row == 0 {2} else {0};
    for col in (startcol..ljpeg.width).step(2) {
      let (p1,p2) = if col == 0 {
        // At start of line predictor starts with start of previous line
        (out[(row-1)*ljpeg.width],out[(row-1)*ljpeg.width+1])
      } else {
        // All other cases use the two previous pixels in the same line
        (out[row*ljpeg.width+col-2], out[row*ljpeg.width+col-1])
      };

      let diff1 = try!(htable1.huff_decode(&mut pump));
      out[row*ljpeg.width+col] = ((p1 as i32) + diff1) as u16;
      if col + 1 < ljpeg.width {
        let diff2 = try!(htable2.huff_decode(&mut pump));
        out[row*ljpeg.width+col+1] = ((p2 as i32) + diff2) as u16;
      }
    }
  }

  Ok(out)
}
