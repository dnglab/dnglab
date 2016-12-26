use std::fmt;
use std::clone;

pub struct CFA {
  patname: String,
  pattern: [[usize;48];48],
}

impl CFA {
  pub fn new(patname: &str) -> CFA {
    let size = match patname.len() {
      0 => 0,
      4 => 2,
      36 => 6,
      144 => 12,
      _ => panic!(format!("Unknown CFA with size {}", patname.len()).to_string()),
    };
    let mut pattern: [[usize;48];48] = [[0;48];48];

    if size > 0 {
      // copy the pattern into the top left
      for (i,c) in patname.bytes().enumerate() {
        pattern[i/size][i%size] = match c {
          b'R' => 0,
          b'G' => 1,
          b'B' => 2,
          b'E' => 3,
          _   => panic!(format!("Unknown CFA color \"{}\"", c).to_string()),
        };
      }

      // extend the pattern into the full matrix
      for row in 0..48 {
        for col in 0..48 {
          pattern[row][col] = pattern[row%size][col%size];
        }
      }
    }

    CFA {
      patname: patname.to_string(),
      pattern: pattern,
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
      patname: format!("shifted-{}-{}-{}", x, y, self.patname).to_string(),
      pattern: pattern,
    }
  }
}

impl fmt::Debug for CFA {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    write!(f, "CFA {{ {} }}", self.patname)
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
      patname: self.patname.clone(),
      pattern: cpattern,
    }
  }
}
