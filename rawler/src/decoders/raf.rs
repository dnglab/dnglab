use std::f32::NAN;

use crate::alloc_image_plain;
use crate::decoders::*;
use crate::formats::tiff::*;
use crate::packed::*;

#[derive(Debug, Clone)]
pub struct RafDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: TiffIFD<'a>,
}

impl<'a> RafDecoder<'a> {
  pub fn new(buf: &'a [u8], tiff: TiffIFD<'a>, rawloader: &'a RawLoader) -> RafDecoder<'a> {
    RafDecoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for RafDecoder<'a> {
  fn raw_image(&self, _params: RawDecodeParams, dummy: bool) -> Result<RawImage,String> {
    let cam = self.rawloader.check_supported(&self.tiff)?;
    let raw = fetch_ifd!(&self.tiff, TiffRootTag::RafOffsets);
    let (width,height) = if raw.has_entry(TiffRootTag::RafImageWidth) {
      (fetch_tag!(raw, TiffRootTag::RafImageWidth).get_usize(0),
       fetch_tag!(raw, TiffRootTag::RafImageLength).get_usize(0))
    } else {
      let sizes = fetch_tag!(raw, TiffRootTag::ImageWidth);
      (sizes.get_usize(1), sizes.get_usize(0))
    };
    let offset = fetch_tag!(raw, TiffRootTag::RafOffsets).get_usize(0) + raw.start_offset();
    let bps = match raw.find_entry(TiffRootTag::RafBitsPerSample) {
      Some(val) => val.get_u32(0) as usize,
      None      => 16,
    };
    let src = &self.buffer[offset..];

    let image = if cam.find_hint("double_width") {
      // Some fuji SuperCCD cameras include a second raw image next to the first one
      // that is identical but darker to the first. The two combined can produce
      // a higher dynamic range image. Right now we're ignoring it.
      decode_16le_skiplines(src, width, height, dummy)
    } else if cam.find_hint("jpeg32") {
      decode_12be_msb32(src, width, height, dummy)
    } else {
      if src.len() < bps*width*height/8 {
        return Err("RAF: Don't know how to decode compressed yet".to_string())
      }
      match bps {
        12 => decode_12le(src, width, height, dummy),
        14 => decode_14le_unpacked(src, width, height, dummy),
        16 => {
          if self.tiff.little_endian() {
            decode_16le(src, width, height, dummy)
          } else {
            decode_16be(src, width, height, dummy)
          }
        },
        _ => {return Err(format!("RAF: Don't know how to decode bps {}", bps).to_string());},
      }
    };

    if cam.find_hint("fuji_rotation") || cam.find_hint("fuji_rotation_alt") {
      let (width, height, image) = RafDecoder::rotate_image(&image, &cam, width, height, dummy);
      Ok(RawImage {
        make: cam.make.clone(),
        model: cam.model.clone(),
        clean_make: cam.clean_make.clone(),
        clean_model: cam.clean_model.clone(),
        width: width,
        height: height,
        cpp: 1,
        wb_coeffs: self.get_wb()?,
        data: RawImageData::Integer(image),
        blacklevels: cam.blacklevels,
        whitelevels: cam.whitelevels,
        xyz_to_cam: cam.xyz_to_cam,
        cfa: cam.cfa.clone(),
        crops: [0,0,0,0],
        blackareas: Vec::new(),
        orientation: cam.orientation,
        color_matrix: cam.color_matrix,
      })
    } else {
      ok_image(cam, width, height, self.get_wb()?, image)
    }
  }
}

impl<'a> RafDecoder<'a> {
  fn get_wb(&self) -> Result<[f32;4], String> {
    match self.tiff.find_entry(TiffRootTag::RafWBGRB) {
      Some(levels) => Ok([levels.get_f32(1), levels.get_f32(0), levels.get_f32(2), NAN]),
      None => {
        let levels = fetch_tag!(self.tiff, TiffRootTag::RafOldWB);
        Ok([levels.get_f32(1), levels.get_f32(0), levels.get_f32(3), NAN])
      },
    }
  }

  fn rotate_image(src: &[u16], camera: &Camera, width: usize, height: usize, dummy: bool) -> (usize, usize, Vec<u16>) {
    let x = camera.crops[3];
    let y = camera.crops[0];
    let cropwidth = width - camera.crops[1] - x;
    let cropheight = height - camera.crops[2] - y;

    if camera.find_hint("fuji_rotation_alt") {
      let rotatedwidth = cropheight + cropwidth/2;
      let rotatedheight = rotatedwidth-1;

      let mut out: Vec<u16> = alloc_image_plain!(rotatedwidth, rotatedheight, dummy);
      if !dummy {
        for row in 0..cropheight {
          let inb = &src[(row+y)*width+x..];
          for col in 0..cropwidth {
            let out_row = rotatedwidth - (cropheight + 1 - row + (col >> 1));
            let out_col = ((col+1) >> 1) + row;
            out[out_row*rotatedwidth+out_col] = inb[col];
          }
        }
      }

      (rotatedwidth, rotatedheight, out)
    } else {
      let rotatedwidth = cropwidth + cropheight/2;
      let rotatedheight = rotatedwidth-1;

      let mut out: Vec<u16> = alloc_image_plain!(rotatedwidth, rotatedheight, dummy);
      if !dummy {
        for row in 0..cropheight {
          let inb = &src[(row+y)*width+x..];
          for col in 0..cropwidth {
            let out_row = cropwidth - 1 - col + (row>>1);
            let out_col = ((row+1) >> 1) + col;
            out[out_row*rotatedwidth+out_col] = inb[col];
          }
        }
      }

      (rotatedwidth, rotatedheight, out)
    }
  }
}
