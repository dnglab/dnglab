// SPDX-License-Identifier: LGPL-2.1
// Copyright by image-tiff authors (see https://github.com/image-rs/image-tiff)
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

mod bytecast;
mod cfa_image;
pub mod colortype;
mod encoder;
mod error;
mod preview_image;
pub mod tags;
mod tiff_value;
mod writer;

pub use cfa_image::CfaImageEncoder;
pub use encoder::DngEncoder;
pub use error::DngResult;
pub use preview_image::PreviewImageEncoder;
pub use tiff_value::*;

pub const DNG_VERSION_V1_1: [u8; 4] = [1, 1, 0, 0];
pub const DNG_VERSION_V1_2: [u8; 4] = [1, 2, 0, 0];
pub const DNG_VERSION_V1_3: [u8; 4] = [1, 3, 0, 0];
pub const DNG_VERSION_V1_4: [u8; 4] = [1, 4, 0, 0];
pub const DNG_VERSION_V1_5: [u8; 4] = [1, 5, 0, 0];

#[cfg(test)]
mod tests {
    use super::tags::Tag;
    use super::*;
    use std::io::Cursor;

    #[test]
    fn create_basic_dng() {
        let mut inmem_file = Cursor::new(Vec::new());
        {
            let mut dng = DngEncoder::new(&mut inmem_file).unwrap();
            let mut root_ifd = dng.new_preview_image::<colortype::RGB8>(256, 171).unwrap();
            root_ifd
                .encoder()
                .write_tag(Tag::DNGVersion, &DNG_VERSION_V1_4[..])
                .unwrap();
            let ifd0_offset = root_ifd.finish().unwrap();
            dng.update_ifd0_offset(ifd0_offset).unwrap();
        }
    }
}
