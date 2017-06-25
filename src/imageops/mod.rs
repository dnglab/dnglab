extern crate rayon;
use self::rayon::prelude::*;

pub mod gofloat;
pub mod demosaic;
pub mod level;
pub mod colorspaces;
pub mod curves;
pub mod gamma;
pub mod transform;

use decoders::RawImage;

extern crate time;
extern crate toml;
extern crate serde;
extern crate serde_yaml;
use self::serde::Serialize;

use std::hash::{Hash, Hasher};
extern crate metrohash;
use self::metrohash::MetroHash;

use std::fmt::Debug;

extern crate multicache;
use self::multicache::MultiCache;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub struct OpBuffer {
  pub width: usize,
  pub height: usize,
  pub colors: usize,
  pub data: Vec<f32>,
}

impl OpBuffer {
  pub fn new(width: usize, height: usize, colors: usize) -> OpBuffer {
    OpBuffer {
      width: width,
      height: height,
      colors: colors,
      data: vec![0.0; width*height*(colors as usize)],
    }
  }

  pub fn mutate_lines<F>(&mut self, closure: &F)
    where F : Fn(&mut [f32], usize)+Sync {

    self.data.par_chunks_mut(self.width*self.colors).enumerate().for_each(|(row, line)| {
      closure(line, row);
    });
  }

  pub fn process_into_new<F>(&self, colors: usize, closure: &F) -> OpBuffer
    where F : Fn(&mut [f32], &[f32])+Sync {

    let mut out = OpBuffer::new(self.width, self.height, colors);
    out.data.par_chunks_mut(out.width*out.colors).enumerate().for_each(|(row, line)| {
      closure(line, &self.data[self.width*self.colors*row..]);
    });
    out
  }

  /// Helper function to allow human readable creation of `OpBuffer` instances
  pub fn from_rgb_str_vec(data: Vec<&str>) -> OpBuffer {
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
}

fn do_timing<O, F: FnMut() -> O>(name: &str, mut closure: F) -> O {
  let from_time = time::precise_time_ns();
  let ret = closure();
  let to_time = time::precise_time_ns();
  println!("{} ms for '{}'", (to_time - from_time)/1000000, name);

  ret
}

pub trait ImageOp<'a>: Debug {
  fn name(&self) -> &str;
  fn run(&self, pipeline: &mut PipelineGlobals, inid: u64, outid: u64);
  fn to_settings(&self) -> String;
  fn hash(&self, hasher: &mut MetroHash);
}

fn standard_to_settings<T: Serialize>(obj: &T) -> String {
  serde_yaml::to_string(obj).unwrap()
}

fn standard_hash<T: Hash>(obj: &T, hasher: &mut MetroHash) {
  obj.hash(hasher)
}

#[derive(Debug)]
pub struct PipelineGlobals<'a> {
  cache: MultiCache<u64, OpBuffer>,
  maxwidth: usize,
  maxheight: usize,
  linear: bool,
  image: &'a RawImage,
}

#[derive(Debug)]
pub struct PipelineOps {
  gofloat: gofloat::OpGoFloat,
  demosaic: demosaic::OpDemosaic,
  level: level::OpLevel,
  tolab: colorspaces::OpToLab,
  basecurve: curves::OpBaseCurve,
  fromlab: colorspaces::OpFromLab,
  gamma: gamma::OpGamma,
  transform: transform::OpTransform,
}

impl PipelineOps {
  pub fn all(&self) -> Vec<&ImageOp> {
    vec![
      &self.gofloat,
      &self.demosaic,
      &self.level,
      &self.tolab,
      &self.basecurve,
      &self.fromlab,
      &self.gamma,
      &self.transform,
    ]
  }
}

#[derive(Debug)]
pub struct Pipeline<'a> {
  globals: PipelineGlobals<'a>,
  ops: PipelineOps,
}

impl<'a> Pipeline<'a> {
  pub fn new(img: &RawImage, maxwidth: usize, maxheight: usize, linear: bool) -> Pipeline {
    // Check if the image's orientation results in a rotation that
    // swaps the maximum width with the maximum height
    let (transpose, ..) = img.orientation.to_flips();
    let (maxwidth, maxheight) = if transpose {
      (maxheight, maxwidth)
    } else {
      (maxwidth, maxheight)
    };

    Pipeline {
      globals: PipelineGlobals {
        cache: MultiCache::new(1),
        maxwidth,
        maxheight,
        linear,
        image: img,
      },
      ops: PipelineOps {
        gofloat: gofloat::OpGoFloat::new(img),
        demosaic: demosaic::OpDemosaic::new(img),
        level: level::OpLevel::new(img),
        tolab: colorspaces::OpToLab::new(img),
        basecurve: curves::OpBaseCurve::new(img),
        fromlab: colorspaces::OpFromLab::new(img),
        gamma: gamma::OpGamma::new(img),
        transform: transform::OpTransform::new(img),
      },
    }
  }

  pub fn run(&mut self) -> Arc<OpBuffer> {
    // Generate all the hashes for the operations
    let mut hasher = MetroHash::new();
    let mut ophashes = Vec::new();
    for op in self.ops.all() {
      // Hash the name first as a zero sized struct doesn't actually do any hashing
      op.name().hash(&mut hasher);
      op.hash(&mut hasher);
      ophashes.push((hasher.finish(), op));
    }

    // Do the operations, starting with a dummy buffer id as gofloat doesn't use it
    let mut bufin: u64 = 0;
    for (hash, op) in ophashes {
      let globals = &mut self.globals;
      do_timing(op.name(), ||op.run(globals, bufin, hash));
      bufin = hash;
    }
    self.globals.cache.get(bufin).unwrap()
  }
}

fn simple_decode_full(img: &RawImage, maxwidth: usize, maxheight: usize, linear: bool) -> OpBuffer {
  let buf = {
    let mut pipeline = Pipeline::new(img, maxwidth, maxheight, linear);
    pipeline.run()
  };

  // Since we've kept the pipeline to ourselves unwraping always works
  Arc::try_unwrap(buf).unwrap()
}


pub fn simple_decode(img: &RawImage, maxwidth: usize, maxheight: usize) -> OpBuffer {
  simple_decode_full(img, maxwidth, maxheight, false)
}

pub fn simple_decode_linear(img: &RawImage, maxwidth: usize, maxheight: usize) -> OpBuffer {
  simple_decode_full(img, maxwidth, maxheight, true)
}
