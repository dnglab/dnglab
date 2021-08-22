// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

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
