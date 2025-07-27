// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

pub const IDENTITY_MATRIX_3: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// Multiply two matrices a and b
pub fn multiply<const X: usize, const A: usize, const B: usize>(a: &[[f32; X]; A], b: &[[f32; B]; X]) -> [[f32; B]; A] {
  let mut r = [[0.0; B]; A];
  for i in 0..A {
    for j in 0..B {
      for x in 0..X {
        r[i][j] += a[i][x] * b[x][j];
      }
    }
  }
  r
}

pub fn multiply_row1(a: &[[f32; 3]; 3], b: &[f32; 3]) -> [f32; 3] {
  [
    a[0][0] * b[0] + a[0][1] * b[1] + a[0][2] * b[2],
    a[1][0] * b[0] + a[1][1] * b[1] + a[1][2] * b[2],
    a[2][0] * b[0] + a[2][1] * b[1] + a[2][2] * b[2],
  ]
}

/// Normalize a matrix so that the sum of each row equals to 1.0
pub fn normalize<const N: usize, const M: usize>(rgb2cam: [[f32; N]; M]) -> [[f32; N]; M] {
  let mut result = [[0.0; N]; M];
  for m in 0..M {
    let sum: f32 = rgb2cam[m].iter().sum();
    if sum.abs() != 0.0 {
      for n in 0..N {
        result[m][n] = rgb2cam[m][n] / sum;
      }
    }
  }
  result
}

/// Calculate pseudo-inverse of a given matrix
pub fn pseudo_inverse<const N: usize>(matrix: [[f32; 3]; N]) -> [[f32; N]; 3] {
  let mut tmp: [[f32; 3]; N] = [Default::default(); N];
  let mut result: [[f32; N]; 3] = [[Default::default(); N]; 3];

  let mut work: [[f32; 6]; 3] = [Default::default(); 3];
  for i in 0..3 {
    for j in 0..6 {
      work[i][j] = if j == i + 3 { 1.0 } else { 0.0 };
    }
    for j in 0..3 {
      for k in 0..N {
        work[i][j] += matrix[k][i] * matrix[k][j];
      }
    }
  }
  for i in 0..3 {
    let mut num = work[i][i];
    for j in 0..6 {
      work[i][j] /= num;
    }
    for k in 0..3 {
      if k == i {
        continue;
      }
      num = work[k][i];
      for j in 0..6 {
        work[k][j] -= work[i][j] * num;
      }
    }
  }
  for i in 0..N {
    for j in 0..3 {
      tmp[i][j] = 0.0;
      for k in 0..3 {
        tmp[i][j] += work[j][k + 3] * matrix[i][k];
      }
    }
  }
  for i in 0..3 {
    for j in 0..N {
      result[i][j] = tmp[j][i];
    }
  }
  result
}

/// Transpose a given input matrix
pub fn transpose<const N: usize, const M: usize>(matrix: &[[f32; M]; N]) -> [[f32; N]; M] {
  let mut transposed = [[f32::NAN; N]; M];
  for n in 0..N {
    for m in 0..M {
      transposed[m][n] = matrix[n][m];
    }
  }
  transposed
}

/// Transform a 2D matrix representation to 1D
pub fn transform_2d<const N: usize, const M: usize>(matrix: &[[f32; M]; N]) -> Vec<f32> {
  matrix.iter().flat_map(|n| n.iter().cloned()).collect()
}

/// Transform a 1D matrix representation to 2D
pub fn transform_1d<const N: usize, const M: usize>(matrix: &[f32]) -> Option<[[f32; M]; N]> {
  if matrix.len() != (N * M) {
    return None;
  };
  let mut transformed = [[f32::NAN; M]; N];
  for (i, v) in matrix.iter().cloned().enumerate() {
    *transformed.get_mut(i / M).and_then(|inner| inner.get_mut(i % M))? = v;
  }
  Some(transformed)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn transform_1d_to_2d() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let m1d = vec![1.3, 5.3, 6.1, 4.2, 8.3, 8.2];
    assert!(transform_1d::<2, 3>(&m1d).is_some());
    assert!(transform_1d::<3, 2>(&m1d).is_some());
    assert!(transform_1d::<1, 1>(&m1d).is_none());
    assert!(transform_1d::<6, 7>(&m1d).is_none());
    Ok(())
  }
}
