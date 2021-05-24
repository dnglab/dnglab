use std::fmt;

use crate::decoders::tiff::*;

/// Representation of the color filter array pattern in raw cameras
///
/// # Example
/// ```
/// use rawloader::CFA;
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
#[derive(Clone)]
pub struct CFA {
  /// CFA pattern as a String
  pub name: String,
  /// Width of the repeating pattern
  pub width: usize,
  /// Height of the repeating pattern
  pub height: usize,

  pattern: [[usize;48];48],
}

impl CFA {
  #[doc(hidden)] pub fn new_from_tag(pat: &TiffEntry) -> CFA {
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
      0 => (0,0),
      4 => (2,2),
      36 => (6,6),
      16 => (2,8),
      144 => (12,12),
      _ => panic!("Unknown CFA size \"{}\"", patname),
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
          _    => {
              let unknown_char = patname[i..].chars().next().unwrap();
              panic!("Unknown CFA color \"{}\" in pattern \"{}\"", unknown_char, patname)
          },
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

  /// Get the color index at the given position. Designed to be fast so it can be called
  /// from inner loops without performance issues.
  pub fn color_at(&self, row: usize, col: usize) -> usize {
    self.pattern[(row+48) % 48][(col+48) % 48]
  }

  /// Shift the pattern left and/or down. This is useful when cropping the image to get
  /// the equivalent pattern of the crop when it's not a multiple of the pattern size.
  ///
  /// # Example
  /// ```
  /// use rawloader::CFA;
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
    let mut pattern: [[usize;48];48] = [[0;48];48];
    for row in 0..48 {
      for col in 0..48 {
        pattern[row][col] = self.color_at(row+y,col+x);
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
      name: name,
      pattern: pattern,
      width: self.width,
      height: self.height,
    }
  }

  /// Test if this is actually a valid CFA pattern
  ///
  /// # Example
  /// ```
  /// use rawloader::CFA;
  /// let cfa = CFA::new("RGGB");
  /// assert!(cfa.is_valid());
  ///
  /// let cfa = CFA::new("");
  /// assert!(!cfa.is_valid());
  /// ```
  pub fn is_valid(&self) -> bool {
    self.width != 0 && self.height != 0
  }

  /// Convert the CFA back into a pattern string
  ///
  /// # Example
  /// ```
  /// use rawloader::CFA;
  /// let cfa = CFA::new("RGGB");
  /// assert_eq!(cfa.to_string(), "RGGB");
  ///
  /// let shifted = cfa.shift(1,1);
  /// assert_eq!(shifted.to_string(), "BGGR");
  /// ```
  pub fn to_string(&self) -> String {
    self.name.clone()
  }
}

impl fmt::Debug for CFA {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    write!(f, "CFA {{ {} }}", self.name)
  }
}
