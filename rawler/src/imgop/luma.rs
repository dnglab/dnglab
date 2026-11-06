// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Luma of linear camera values

/// Coefficients of the color planes for the luma of neutral values with a white
/// of 1.0, `None` for other numbers of color planes than 1, 3 or 4
///
/// `scale` maps the values of each plane to neutral colors with a white of 1.0:
/// the white balance that is not applied yet and the exposure gain. Only the
/// first coefficients for the planes are used, one that can't be used counts
/// as 1.0.
pub fn luma_coefficients(channels: usize, scale: &[f32; 4]) -> Option<[f32; 4]> {
  let shares = match channels {
    1 => [1.0, 0.0, 0.0, 0.0],
    3 => [0.25, 0.5, 0.25, 0.0],
    4 => [0.25; 4],
    _ => return None,
  };
  let scale = scale.map(|c| if c.is_finite() && c > 0.0 { c } else { 1.0 });
  Some(std::array::from_fn(|plane| shares[plane] * scale[plane]))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn shares_of_the_color_planes() {
    let neutral = [1.0; 4];
    assert_eq!(luma_coefficients(1, &neutral), Some([1.0, 0.0, 0.0, 0.0]));
    assert_eq!(luma_coefficients(3, &neutral), Some([0.25, 0.5, 0.25, 0.0]));
    assert_eq!(luma_coefficients(4, &neutral), Some([0.25; 4]));
    // The luma of white is 1.0
    for channels in [1, 3, 4] {
      assert_eq!(luma_coefficients(channels, &neutral).map(|c| c.iter().sum::<f32>()), Some(1.0));
    }
  }

  #[test]
  fn scale_of_the_color_planes() {
    assert_eq!(luma_coefficients(1, &[2.0, 3.0, 4.0, 5.0]), Some([2.0, 0.0, 0.0, 0.0]));
    assert_eq!(luma_coefficients(3, &[2.0, 3.0, 4.0, 5.0]), Some([0.5, 1.5, 1.0, 0.0]));
    assert_eq!(luma_coefficients(4, &[2.0, 4.0, 8.0, 12.0]), Some([0.5, 1.0, 2.0, 3.0]));
  }

  #[test]
  fn coefficients_that_cannot_be_used_count_as_one() {
    for bad in [f32::NAN, f32::INFINITY, 0.0, -1.0] {
      assert_eq!(luma_coefficients(4, &[bad, 2.0, bad, 2.0]), Some([0.25, 0.5, 0.25, 0.5]), "{}", bad);
    }
  }

  #[test]
  fn other_numbers_of_color_planes() {
    for channels in [0, 2, 5] {
      assert_eq!(luma_coefficients(channels, &[1.0; 4]), None, "{}", channels);
    }
  }
}
