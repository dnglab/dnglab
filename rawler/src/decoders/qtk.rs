// SPDX-License-Identifier: LGPL-2.1
// Copyright 2024 Daniel Vogelbacher <daniel@chaospixel.com>
// Originally written in C in dcraw.c by Dave Coffin

use rayon::iter::IndexedParallelIterator;
use rayon::iter::ParallelIterator;
use std::mem::swap;
use std::ops::Not;

use crate::alloc_image_ok;
use crate::analyze::FormatDump;
use crate::bits::BEu16;
use crate::bits::Endian;
use crate::bits::LookupTable;
use crate::exif::Exif;
use crate::pixarray::PixU16;
use crate::pumps::BitPump;
use crate::pumps::BitPumpMSB;
use crate::pumps::ByteStream;
use crate::OptBuffer;
use crate::Orientation;
use crate::RawFile;
use crate::RawImage;
use crate::RawLoader;
use crate::Result;

use super::ok_cfa_image;
use super::Camera;
use super::Decoder;
use super::FormatHint;
use super::RawDecodeParams;
use super::RawMetadata;

#[derive(Debug, Clone)]
pub struct QtkDecoder<'a> {
  #[allow(unused)]
  rawloader: &'a RawLoader,
  camera: Camera,
}

pub fn is_qtk(file: &mut RawFile) -> bool {
  match file.subview(0, 4) {
    Ok(buf) => buf[0..4] == b"qktk"[..] || buf[0..4] == b"qktn"[..],
    Err(_) => false,
  }
}

impl<'a> QtkDecoder<'a> {
  pub fn new(file: &mut RawFile, rawloader: &'a RawLoader) -> Result<QtkDecoder<'a>> {
    match file.subview(0, 4)?.as_slice() {
      b"qktk" => {
        let make = "Apple";
        let model = "QuickTake 100";
        let camera = rawloader.check_supported_with_everything(make, model, "")?;
        Ok(QtkDecoder { rawloader, camera })
      }
      b"qktn" => {
        if file.subview(0, 6)?[5] != 0 {
          let make = "Apple";
          let model = "QuickTake 200";
          let camera = rawloader.check_supported_with_everything(make, model, "")?;
          Ok(QtkDecoder { rawloader, camera })
        } else {
          let make = "Apple";
          let model = "QuickTake 150";
          let camera = rawloader.check_supported_with_everything(make, model, "")?;
          Ok(QtkDecoder { rawloader, camera })
        }
      }
      sig @ _ => Err(crate::RawlerError::DecoderFailed(format!(
        "Unable to use QTK decoder on file with signature: '{:?}'",
        sig
      ))),
    }
  }
}

impl<'a> Decoder for QtkDecoder<'a> {
  fn raw_image(&self, file: &mut RawFile, _params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    const META_OFFSET: u64 = 544;
    let meta = file.subview(META_OFFSET, 16)?;
    let mut stream = ByteStream::new(&meta, Endian::Big);
    let mut height = stream.get_u16() as usize;
    let mut width = stream.get_u16() as usize;
    let _zero = stream.get_u32();
    let hint = stream.get_u16();
    let offset = if hint == 30 { 738 } else { 736 };
    let mut orientation = Orientation::Normal;

    if height > width {
      swap(&mut width, &mut height);
      let info = file.subview(offset - 6, 6)?;
      orientation = if BEu16(&info, 0).not() & 3 > 0 {
        Orientation::Rotate90
      } else {
        Orientation::Rotate270
      };
      log::debug!("QTK file has flipped width/height, new orientation: {:?}", orientation);
    }

    log::debug!("QTK file w: {}, h: {}, hint: {}", width, height, hint);

    let src: OptBuffer = file.subview_until_eof(offset as u64)?.into();

    let image = match file.subview(0, 4)?.as_slice() {
      b"qktk" => Self::decompress_quicktake_100(self, &src, width, height, dummy)?,
      b"qktn" => Self::decompress_quicktake_150(self, &src, width, height, dummy)?,
      _ => unreachable!(),
    };

    let cpp = 1;
    ok_cfa_image(self.camera.clone(), cpp, self.get_wb()?, image, dummy).map(|mut image| {
      image.orientation = orientation;
      image
    })
  }

  fn format_dump(&self) -> FormatDump {
    todo!()
  }

  fn raw_metadata(&self, _file: &mut RawFile, _params: RawDecodeParams) -> Result<RawMetadata> {
    let meta = RawMetadata::new(&self.camera, Exif::default());
    Ok(meta)
  }

  fn format_hint(&self) -> FormatHint {
    FormatHint::QTK
  }
}

impl<'a> QtkDecoder<'a> {
  fn get_wb(&self) -> Result<[f32; 4]> {
    Ok([f32::NAN, f32::NAN, f32::NAN, f32::NAN])
  }

  pub fn decompress_quicktake_150(&self, src: &OptBuffer, width: usize, height: usize, dummy: bool) -> Result<PixU16> {
    // Model 150 always compress with cbpp=3
    let cbpp = 3;
    crate::decompressors::radc::decompress(src, width, height, cbpp, dummy)
  }

  pub fn decompress_quicktake_100(&self, src: &[u8], width: usize, height: usize, dummy: bool) -> Result<PixU16> {
    assert!(width > height);
    let mut out = alloc_image_ok!(width, height, dummy);
    let mut pump = BitPumpMSB::new(src);

    const GSTEP: [i16; 16] = [-89, -60, -44, -32, -22, -15, -8, -2, 2, 8, 15, 22, 32, 44, 60, 89];

    const RSTEP: [[i16; 4]; 6] = [
      [-3, -1, 1, 3],
      [-5, -1, 1, 5],
      [-8, -2, 2, 8],
      [-13, -3, 3, 13],
      [-19, -4, 4, 19],
      [-28, -6, 6, 28],
    ];

    const CURVE: [u16; 256] = [
      0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42,
      43, 44, 45, 46, 47, 48, 49, 50, 51, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 74, 75, 76, 77, 78, 79, 80, 81, 82,
      83, 84, 86, 88, 90, 92, 94, 97, 99, 101, 103, 105, 107, 110, 112, 114, 116, 118, 120, 123, 125, 127, 129, 131, 134, 136, 138, 140, 142, 144, 147, 149,
      151, 153, 155, 158, 160, 162, 164, 166, 168, 171, 173, 175, 177, 179, 181, 184, 186, 188, 190, 192, 195, 197, 199, 201, 203, 205, 208, 210, 212, 214,
      216, 218, 221, 223, 226, 230, 235, 239, 244, 248, 252, 257, 261, 265, 270, 274, 278, 283, 287, 291, 296, 300, 305, 309, 313, 318, 322, 326, 331, 335,
      339, 344, 348, 352, 357, 361, 365, 370, 374, 379, 383, 387, 392, 396, 400, 405, 409, 413, 418, 422, 426, 431, 435, 440, 444, 448, 453, 457, 461, 466,
      470, 474, 479, 483, 487, 492, 496, 500, 508, 519, 531, 542, 553, 564, 575, 587, 598, 609, 620, 631, 643, 654, 665, 676, 687, 698, 710, 721, 732, 743,
      754, 766, 777, 788, 799, 810, 822, 833, 844, 855, 866, 878, 889, 900, 911, 922, 933, 945, 956, 967, 978, 989, 1001, 1012, 1023,
    ];

    let mut pix = [[0x80_i16; 644]; 484];

    for row in 2..(height + 2) {
      let cstart = 2 + (row & 1);
      let mut val = 0;
      for col in (cstart..(width + 2)).step_by(2) {
        val = (((pix[row - 1][col - 1] + 2 * pix[row - 1][col + 1] + pix[row][col - 2]) >> 2) + GSTEP[pump.get_bits(4) as usize]).clamp(0, 255);
        pix[row][col] = val;
        if col < 4 {
          pix[row][col - 2] = val;
          pix[row + 1][(!row) & 1] = val;
        }
        if row == 2 {
          pix[row - 1][col + 1] = val;
          pix[row - 1][col + 3] = val;
        }
      }
      pix[row][width + 2 + (row & 1)] = val; // last column
    }

    for rb in 0..2 {
      for row in ((2 + rb)..(height + 2)).step_by(2) {
        for col in ((3 - (row & 1))..(width + 2)).step_by(2) {
          let sharp = if row < 4 || col < 4 {
            2
          } else {
            let val = (pix[row - 2][col] - pix[row][col - 2]).abs() as i32
              + (pix[row - 2][col] - pix[row - 2][col - 2]).abs() as i32
              + (pix[row][col - 2] - pix[row - 2][col - 2]).abs() as i32;
            match val {
              0..4 => 0,
              4..8 => 1,
              8..16 => 2,
              16..32 => 3,
              32..48 => 4,
              _ => 5,
            }
          };

          let val = (((pix[row - 2][col] + pix[row][col - 2]) >> 1) + RSTEP[sharp][pump.get_bits(2) as usize]).clamp(0, 255);

          pix[row][col] = val;
          if row < 4 {
            pix[row - 2][col + 2] = val;
          };
          if col < 4 {
            pix[row + 2][col - 2] = val;
          };
        }
      }
    }

    for row in 2..(height + 2) {
      for col in ((3 - (row & 1))..(width + 2)).step_by(2) {
        let val = ((pix[row][col - 1] + (pix[row][col] << 2) + pix[row][col + 1]) >> 1) - 0x100;
        pix[row][col] = val.clamp(0, 255);
      }
    }

    let tbl = LookupTable::new_with_bits(&CURVE, 10);
    out.par_pixel_rows_mut().enumerate().for_each(|(row, line)| {
      let mut random = (pix[row + 2][2] as u32) << 16 | (pix[row + 2][3]) as u32;
      for (x, p) in line.iter_mut().zip(pix[row + 2][2..width + 2].iter()) {
        *x = tbl.dither(*p as u16, &mut random);
        //*x = CURVE[*p as usize]; // no dither
      }
    });

    Ok(out)
  }
}
