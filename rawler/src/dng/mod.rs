// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

mod embedded;

pub use embedded::original_compress;
pub use embedded::original_digest;

use crate::RawImage;

pub const DNG_VERSION_V1_0: [u8; 4] = [1, 0, 0, 0];
pub const DNG_VERSION_V1_1: [u8; 4] = [1, 1, 0, 0];
pub const DNG_VERSION_V1_2: [u8; 4] = [1, 2, 0, 0];
pub const DNG_VERSION_V1_3: [u8; 4] = [1, 3, 0, 0];
pub const DNG_VERSION_V1_4: [u8; 4] = [1, 4, 0, 0];
pub const DNG_VERSION_V1_5: [u8; 4] = [1, 5, 0, 0];
pub const DNG_VERSION_V1_6: [u8; 4] = [1, 6, 0, 0];

/// Convert internal crop rectangle to DNG active area
///
/// DNG ActiveArea  is:
///  Top, Left, Bottom, Right
/// RawImage.crop is:
/// Top, Right, Bottom, Left
pub fn dng_active_area(image: &RawImage) -> [u16; 4] {
  [
    image.crops[0] as u16, // top
    image.crops[3] as u16, // left
    //(image.height-image.crops[0]-image.crops[2]) as u16, // bottom
    //(image.width-image.crops[1]-image.crops[3]) as u16, // Right
    (image.height - (image.crops[2])) as u16, // bottom coord
    (image.width - (image.crops[1])) as u16,  // Right coord
  ]
}
