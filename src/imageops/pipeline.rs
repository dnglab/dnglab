use imageops::*;

extern crate multicache;
use self::multicache::MultiCache;
use std::sync::Arc;
use std::io::Write;

extern crate time;
extern crate serde;
extern crate serde_yaml;
use self::serde::{Serialize,Deserialize};

fn do_timing<O, F: FnMut() -> O>(name: &str, mut closure: F) -> O {
  let from_time = time::precise_time_ns();
  let ret = closure();
  let to_time = time::precise_time_ns();
  println!("{} ms for '{}'", (to_time - from_time)/1000000, name);

  ret
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
    hasher.from_serialize(self);
  }
}

#[derive(Debug)]
pub struct PipelineGlobals<'a> {
  pub cache: MultiCache<BufHash, OpBuffer>,
  pub maxwidth: usize,
  pub maxheight: usize,
  pub linear: bool,
  pub image: &'a RawImage,
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
    self.globals.cache.get(&bufin).unwrap()
  }
}
