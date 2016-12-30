use decoders::tiff::*;
use std::fmt;
use std::clone;

pub struct CFA {
  name: String,
  pattern: [[usize;48];48],
  pub width: usize,
  pub height: usize,
}

impl CFA {
  pub fn new_from_tag(pat: &TiffEntry) -> CFA {
    let mut patname = String::new();
    for i in 0..pat.count() {
      patname.push(match pat.get_u32(i as usize) {
        0 => 'R',
        1 => 'G',
        2 => 'B',
        _ => 'U',
      });
    }
    CFA::new(&patname)
  }

  pub fn new(patname: &str) -> CFA {
    let (width, height) = match patname.len() {
      0 => (0,0),
      4 => (2,2),
      36 => (6,6),
      16 => (2,8),
      144 => (12,12),
      _ => panic!(format!("Unknown CFA size \"{}\"", patname).to_string()),
    };
    let mut pattern: [[usize;48];48] = [[0;48];48];

    if width > 0 {
      // copy the pattern into the top left
      for (i,c) in patname.bytes().enumerate() {
        pattern[i/width][i%width] = match c {
          b'R' => 0,
          b'G' => 1,
          b'B' => 2,
          b'E' => 3,
          b'M' => 1,
          b'Y' => 3,
          _   => panic!(format!("Unknown CFA color \"{}\"", c).to_string()),
        };
      }

      // extend the pattern into the full matrix
      for row in 0..48 {
        for col in 0..48 {
          pattern[row][col] = pattern[row%height][col%width];
        }
      }
    }

    CFA {
      name: patname.to_string(),
      pattern: pattern,
      width: width,
      height: height,
    }
  }

  pub fn color_at(&self, row: usize, col: usize) -> usize {
    self.pattern[(row+48) % 48][(col+48) % 48]
  }

  pub fn shift(&self, x: usize, y: usize) -> CFA {
    let mut pattern: [[usize;48];48] = [[0;48];48];
    for row in 0..48 {
      for col in 0..48 {
        pattern[row][col] = self.color_at(row+y,col+x);
      }
    }
    CFA {
      name: format!("shifted-{}-{}-{}", x, y, self.name).to_string(),
      pattern: pattern,
      width: self.width,
      height: self.height,
    }
  }
}

impl fmt::Debug for CFA {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    write!(f, "CFA {{ {} }}", self.name)
  }
}

impl clone::Clone for CFA {
  fn clone(&self) -> CFA {
    let mut cpattern: [[usize;48];48] = [[0;48];48];
    for row in 0..48 {
      for col in 0..48 {
        cpattern[row][col] = self.pattern[row][col];
      }
    }
    CFA {
      name: self.name.clone(),
      pattern: cpattern,
      width: self.width,
      height: self.height,
    }
  }
}
