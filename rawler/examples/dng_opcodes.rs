//! Print the DNG opcodes of a Google Pixel 6 Pro DNG of rawdb.dnglab.org, apply
//! its OpcodeList2 and develop it to a JPEG image
//!
//! DNG opcodes are processing steps stored in the file: OpcodeList1 is applied
//! to the raw values, OpcodeList2 to the linear values after the mapping of the
//! black and white level, and OpcodeList3 after demosaicing. The OpcodeList2 of
//! this file has a GainMap for each Bayer phase, which corrects the lens shading.
//!
//! ```sh
//! cargo run -p rawler --release --example dng_opcodes [-- <output.jpg>]
//! ```

use rawler::{
  decoders::RawDecodeParams,
  dng::opcodes::{Opcode, OpcodeStage},
  imgop::develop::RawDevelop,
  rawdb,
  rawsource::RawSource,
};
use std::{fs::File, io::BufWriter, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
  let output = std::env::args_os().nth(1).map_or_else(|| PathBuf::from("dng_opcodes.jpg"), PathBuf::from);
  let path = rawdb::sample("Google", "Pixel 6 Pro", "raw_modes/Pixel_6_pro.dng")?;
  println!("Sample: {}", path.display());

  let source = RawSource::new(&path)?;
  let decoder = rawler::get_decoder(&source)?;
  let params = RawDecodeParams::default();
  let mut rawimage = decoder.raw_image(&source, &params, false)?;
  let metadata = decoder.raw_metadata(&source, &params)?;

  for stage in [OpcodeStage::List1, OpcodeStage::List2, OpcodeStage::List3] {
    let Some(list) = rawimage.opcode_list(stage) else {
      println!("{}: none", stage);
      continue;
    };
    println!("{}:", stage);
    for opcode in list.opcodes() {
      println!("  {}", describe(opcode));
    }
  }

  // OpcodeList2 is applied to linear values, so OpcodeList1 is applied and the
  // raw values are mapped to 0.0 .. 1.0 before
  rawimage.apply_opcode_list(OpcodeStage::List1)?;
  rawimage.apply_scaling()?;
  let before = rawimage.data.as_f32().into_owned();
  rawimage.apply_opcode_list(OpcodeStage::List2)?;
  let after = rawimage.data.as_f32();

  // Average gain of blocks of 1/8 of the image size
  let (stride, height) = (rawimage.width * rawimage.cpp, rawimage.height);
  let (block_width, block_height) = (stride / 8, height / 8);
  let gain = |x: usize, y: usize| {
    let (mut sum_before, mut sum_after) = (0.0, 0.0);
    for row in y..y + block_height {
      let block = row * stride + x..row * stride + x + block_width;
      sum_before += before[block.clone()].iter().map(|v| f64::from(*v)).sum::<f64>();
      sum_after += after[block].iter().map(|v| f64::from(*v)).sum::<f64>();
    }
    sum_after / sum_before
  };
  let center = gain((stride - block_width) / 2, (height - block_height) / 2);
  println!("Average gain of OpcodeList2: center {:.3}, top left corner {:.3}", center, gain(0, 0));

  // OpcodeList1 and OpcodeList2 are consumed and the values are already mapped,
  // so the development continues with the demosaicing and OpcodeList3
  RawDevelop::default().develop_jpeg(rawimage, &metadata, BufWriter::new(File::create(&output)?), 90)?;
  println!("Developed image: {}", output.display());
  Ok(())
}

/// Name and version of `opcode`, and the parameters of a GainMap
fn describe(opcode: &Opcode) -> String {
  let optional = if opcode.is_optional() { ", optional" } else { "" };
  let mut text = format!("{} (DNG {}{})", opcode.name(), opcode.version(), optional);
  if let Opcode::GainMap(map) = opcode {
    let area = &map.area;
    let (min, max) = map
      .gain_map
      .iter()
      .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), gain| (min.min(*gain), max.max(*gain)));
    text += &format!(
      ": rows {}..{} step {}, columns {}..{} step {}, plane {}, {}x{}x{} points, gains {:.3} .. {:.3}",
      area.top, area.bottom, area.row_pitch, area.left, area.right, area.col_pitch, area.plane, map.map_points_h, map.map_points_v, map.map_planes, min, max
    );
  }
  text
}
