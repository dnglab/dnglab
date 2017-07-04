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
use self::serde::{Serialize,Deserialize};

extern crate bincode;
extern crate sha2;
use self::sha2::Digest;

use std;
use std::io::Write;
use std::fmt;
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

type HashType = self::sha2::Sha256;
type BufHash = [u8;32];
#[derive(Copy, Clone)]
pub struct BufHasher {
  hash: HashType,
}
impl BufHasher {
  pub fn new() -> BufHasher {
    BufHasher {
      hash: HashType::default(),
    }
  }
  pub fn result(&self) -> BufHash {
    let mut result = BufHash::default();
    for (i, byte) in self.hash.result().into_iter().enumerate() {
      result[i] = byte;
    }
    result
  }
}
impl Debug for BufHasher {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    write!(f, "BufHasher {{ {:?} }}", self.result())
  }
}

impl Write for BufHasher {
  fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
    self.hash.input(buf);
    Ok(buf.len())
  }
  fn flush(&mut self) -> std::io::Result<()> {Ok(())}
}

pub trait ImageOp<'a>: Debug+Serialize+Deserialize<'a> {
  fn name(&self) -> &str;
  fn run(&self, pipeline: &mut PipelineGlobals, inid: BufHash, outid: BufHash);
  fn to_settings(&self) -> String {
    serde_yaml::to_string(self).unwrap()
  }
  fn hash(&self, hasher: &mut BufHasher) {
    // Hash the name first as a zero sized struct doesn't actually do any hashing
    hasher.write(self.name().as_bytes()).unwrap();
    self::bincode::serialize_into(hasher, self, self::bincode::Infinite).unwrap();
  }
}

#[derive(Debug)]
pub struct PipelineGlobals<'a> {
  cache: MultiCache<BufHash, OpBuffer>,
  maxwidth: usize,
  maxheight: usize,
  linear: bool,
  image: &'a RawImage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

macro_rules! for_vals {
  ([$($val:expr),*] |$x:pat, $i:ident| $body:expr) => {
    let mut pos = 0;
    $({
      let $x = $val;
      pos += 1;
      let $i = pos-1;
      $body
    })*
  }
}

macro_rules! all_ops {
  ($ops:expr, |$x:pat, $i:ident| $body:expr) => {
    for_vals!([
      $ops.gofloat,
      $ops.demosaic,
      $ops.level,
      $ops.tolab,
      $ops.basecurve,
      $ops.fromlab,
      $ops.gamma,
      $ops.transform
    ] |$x, $i| {
      $body
    });
  }
}

#[derive(Debug)]
pub struct Pipeline<'a> {
  globals: PipelineGlobals<'a>,
  ops: PipelineOps,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PipelineSerialization {
  version: u32,
  filehash: String,
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

  pub fn to_serial(&self) -> String {
    let serial = (PipelineSerialization {
      version: 0,
      filehash: "0".to_string(),
    }, &self.ops);

    serde_yaml::to_string(&serial).unwrap()
  }

  pub fn new_from_serial(img: &RawImage, maxwidth: usize, maxheight: usize, linear: bool, serial: String) -> Pipeline {
    let serial: (PipelineSerialization, PipelineOps) = serde_yaml::from_str(&serial).unwrap();

    Pipeline {
      globals: PipelineGlobals {
        cache: MultiCache::new(1),
        maxwidth,
        maxheight,
        linear,
        image: img,
      },
      ops: serial.1,
    }
  }

  pub fn run(&mut self) -> Arc<OpBuffer> {
    // Generate all the hashes for the operations
    let mut hasher = BufHasher::new();
    let mut ophashes = Vec::new();
    let mut startpos = 0;
    all_ops!(self.ops, |ref op, i| {
      op.hash(&mut hasher);
      let result = hasher.result();
      ophashes.push(result);

      // Set the latest op for which we already have the calculated buffer
      if self.globals.cache.contains_key(&result) {
        startpos = i+1;
      }
    });

    // Do the operations, starting with a dummy buffer id as gofloat doesn't use it
    let mut bufin = BufHash::default();
    all_ops!(self.ops, |ref op, i| {
      let hash = ophashes[i];
      if i >= startpos { // We're at the point where we need to start calculating ops
        let globals = &mut self.globals;
        do_timing(op.name(), ||op.run(globals, bufin, hash));
      }
      bufin = hash;
    });
    self.globals.cache.get(bufin).unwrap()
  }
}

fn simple_decode_full(img: &RawImage, maxwidth: usize, maxheight: usize, linear: bool) -> OpBuffer {
  let buf = {
    let mut pipeline = Pipeline::new(img, maxwidth, maxheight, linear);
    // FIXME: turn these into tests
    //
    // --- Check if serialization roundtrips
    // let serial = pipeline.to_serial();
    // println!("Settings are: {}", serial);
    // pipeline = Pipeline::new_from_serial(img, maxwidth, maxheight, linear, serial);
    //
    // --- Check that the pipeline caches buffers and does not recalculate
    // pipeline.run();
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
