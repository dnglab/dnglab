use std::fmt;

use itertools::Itertools;

use crate::formats::tiff::Value;

pub const CFA_COLOR_R: usize = 0;
pub const CFA_COLOR_G: usize = 1;
pub const CFA_COLOR_B: usize = 2;

use num_enum::TryFromPrimitive;

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, TryFromPrimitive)]
#[repr(usize)]
#[allow(non_camel_case_types)]
pub enum CFAColor {
  // see also DngDecoder
  RED = 0,
  GREEN = 1,
  BLUE = 2,
  CYAN = 3,
  MAGENTA = 4,
  YELLOW = 5,
  WHITE = 6,
  FUJI_GREEN = 7,
  END, // keep it last!
  UNKNOWN = 255,
}

impl Default for CFAColor {
  fn default() -> Self {
    Self::UNKNOWN
  }
}

/// Representation of the color filter array pattern in raw cameras
///
/// # Example
/// ```
/// use rawler::CFA;
/// let cfa = CFA::new("RGGB");
/// assert_eq!(cfa.color_at(0,0), 0);
/// assert_eq!(cfa.color_at(0,1), 1);
/// assert_eq!(cfa.color_at(1,0), 1);
/// assert_eq!(cfa.color_at(1,1), 2);
/// ```
///
/// You will almost always get your CFA struct from a RawImage decode, already fully
/// initialized and ready to be used in processing. The color_at() implementation is
/// designed to be fast so it can be called inside the inner loop of demosaic or other
/// color-aware algorithms that work on pre-demosaic data
#[derive(Clone, Eq, PartialEq, PartialOrd, Ord)]
pub struct CFA {
  /// CFA pattern as a String
  pub name: String,
  /// Width of the repeating pattern
  pub width: usize,
  /// Height of the repeating pattern
  pub height: usize,

  pattern: [[usize; 48]; 48],
}

impl Default for CFA {
  fn default() -> Self {
    Self::new("")
  }
}

impl CFA {
  #[doc(hidden)]
  pub fn new_from_tag(pat: &Value) -> CFA {
    let mut patname = String::new();
    for i in 0..pat.count() {
      patname.push(match pat.force_u32(i as usize) {
        0 => 'R',
        1 => 'G',
        2 => 'B',
        _ => 'U',
      });
    }
    CFA::new(&patname)
  }

  /// Create a new CFA from a string describing it. For simplicity the pattern is specified
  /// as each pixel being one of R/G/B/E representing the 0/1/2/3 colors in a 4 color image.
  /// The pattern is specified as the colors in each row concatenated so RGGB means that
  /// the first row is RG and the second row GB. Row size is determined by pattern size
  /// (e.g., the xtrans pattern is 6x6 and thus 36 characters long). In theory this could
  /// lead to confusion between different pattern sizes but in practice there are only
  /// a few oddball cameras no one cares about that do anything but 2x2 and 6x6 (and those
  /// work fine with this as well).
  pub fn new(patname: &str) -> CFA {
    let (width, height) = match patname.len() {
      0 => (0, 0),
      4 => (2, 2),
      36 => (6, 6),
      16 => (2, 8),
      144 => (12, 12),
      _ => panic!("Unknown CFA size \"{}\"", patname),
    };
    let mut pattern: [[usize; 48]; 48] = [[0; 48]; 48];

    if width > 0 {
      // copy the pattern into the top left
      for (i, c) in patname.bytes().enumerate() {
        pattern[i / width][i % width] = match c {
          b'R' => 0,
          b'G' => 1,
          b'B' => 2,
          b'E' => 3,
          b'M' => 1,
          b'Y' => 3,
          _ => {
            let unknown_char = patname[i..].chars().next().unwrap();
            panic!("Unknown CFA color \"{}\" in pattern \"{}\"", unknown_char, patname)
          }
        };
      }

      // extend the pattern into the full matrix
      for row in 0..48 {
        for col in 0..48 {
          pattern[row][col] = pattern[row % height][col % width];
        }
      }
    }

    CFA {
      name: patname.to_string(),
      pattern,
      width,
      height,
    }
  }

  /// Remap the color values
  /// This is useful if you need to remap RGB to R G1 G2 B.
  pub fn map_colors<F>(&self, op: F) -> Self
  where
    F: Fn(usize, usize, usize) -> usize, // row, col, color -> new-color
  {
    let mut copy = self.clone();
    for row in 0..48 {
      for col in 0..48 {
        copy.pattern[row][col] = op(row % self.height, col % self.width, copy.pattern[row % self.height][col % self.width]);
      }
    }
    copy
  }

  /// Get the color index at the given position. Designed to be fast so it can be called
  /// from inner loops without performance issues.
  pub fn color_at(&self, row: usize, col: usize) -> usize {
    self.pattern[(row + 48) % 48][(col + 48) % 48]
  }

  /// from inner loops without performance issues.
  pub fn cfa_color_at(&self, row: usize, col: usize) -> CFAColor {
    self.pattern[(row + 48) % 48][(col + 48) % 48].try_into().unwrap()
  }

  /// Get a flat pattern
  pub fn flat_pattern(&self) -> Vec<u8> {
    self
      .pattern
      .iter()
      .take(self.height)
      .flat_map(|v| v.iter().take(self.width))
      .cloned()
      .map(|v| v as u8)
      .collect()
  }

  /// Count of unique colors in pattern
  pub fn unique_colors(&self) -> usize {
    self.pattern.iter().flatten().unique().count()
  }

  /// Check if pattern is a RGGB or variant.
  /// False for 4-color patterns like RGBE.
  pub fn is_rgb(&self) -> bool {
    self.name.chars().filter(|ch| !['R', 'G', 'B'].contains(ch)).count() == 0 && self.name.contains('R') && self.name.contains('G') && self.name.contains('B')
  }

  pub fn is_rgbe(&self) -> bool {
    self.name.chars().filter(|ch| !['R', 'G', 'B', 'E'].contains(ch)).count() == 0
      && self.name.contains('R')
      && self.name.contains('G')
      && self.name.contains('B')
      && self.name.contains('E')
  }

  pub fn is_cygm(&self) -> bool {
    self.name.chars().filter(|ch| !['C', 'Y', 'G', 'M'].contains(ch)).count() == 0
      && self.name.contains('C')
      && self.name.contains('Y')
      && self.name.contains('G')
      && self.name.contains('M')
  }

  /// Shift the pattern left and/or down. This is useful when cropping the image to get
  /// the equivalent pattern of the crop when it's not a multiple of the pattern size.
  ///
  /// # Example
  /// ```
  /// use rawler::CFA;
  /// let cfa = CFA::new("RGGB");
  /// assert_eq!(cfa.color_at(0,0), 0);
  /// assert_eq!(cfa.color_at(0,1), 1);
  /// assert_eq!(cfa.color_at(1,0), 1);
  /// assert_eq!(cfa.color_at(1,1), 2);
  ///
  /// let shifted = cfa.shift(1,1);
  /// assert_eq!(shifted.color_at(0,0), 2);
  /// assert_eq!(shifted.color_at(0,1), 1);
  /// assert_eq!(shifted.color_at(1,0), 1);
  /// assert_eq!(shifted.color_at(1,1), 0);
  /// ```
  pub fn shift(&self, x: usize, y: usize) -> CFA {
    let mut pattern: [[usize; 48]; 48] = [[0; 48]; 48];
    for row in 0..48 {
      for col in 0..48 {
        pattern[row][col] = self.color_at(row + y, col + x);
      }
    }

    let mut name = "".to_string();
    for row in 0..self.height {
      for col in 0..self.width {
        name.push_str(match pattern[row][col] {
          0 => "R",
          1 => "G",
          2 => "B",
          3 => "E",
          x => panic!("Unknown CFA color \"{}\"", x),
        });
      }
    }

    CFA {
      name,
      pattern,
      width: self.width,
      height: self.height,
    }
  }

  /// Test if this is actually a valid CFA pattern
  ///
  /// # Example
  /// ```
  /// use rawler::CFA;
  /// let cfa = CFA::new("RGGB");
  /// assert!(cfa.is_valid());
  ///
  /// let cfa = CFA::new("");
  /// assert!(!cfa.is_valid());
  /// ```
  pub fn is_valid(&self) -> bool {
    self.width != 0 && self.height != 0
  }
}

impl fmt::Display for CFA {
  /// Convert the CFA back into a pattern string
  ///
  /// # Example
  /// ```
  /// use rawler::CFA;
  /// let cfa = CFA::new("RGGB");
  /// assert_eq!(cfa.to_string(), "RGGB");
  ///
  /// let shifted = cfa.shift(1,1);
  /// assert_eq!(shifted.to_string(), "BGGR");
  /// ```
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.write_str(&self.name)
  }
}

impl fmt::Debug for CFA {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    write!(f, "CFA {{ {} }}", self.name)
  }
}
