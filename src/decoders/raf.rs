use decoders::*;
use decoders::tiff::*;
use decoders::basics::*;
use std::f32::NAN;

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
  fn image(&self) -> Result<Image,String> {
    let camera = try!(self.rawloader.check_supported(&self.tiff));
    let raw = fetch_ifd!(&self.tiff, Tag::RafOffsets);
    let (width,height) = if raw.has_entry(Tag::RafImageWidth) {
      (fetch_tag!(raw, Tag::RafImageWidth).get_u32(0),
       fetch_tag!(raw, Tag::RafImageLength).get_u32(0))
    } else {
      let sizes = fetch_tag!(raw, Tag::ImageWidth);
      (sizes.get_u32(1), sizes.get_u32(0))
    };
    let offset = fetch_tag!(raw, Tag::RafOffsets).get_u32(0) as usize + raw.start_offset();
    let bps = match raw.find_entry(Tag::RafBitsPerSample) {
      Some(val) => val.get_u32(0),
      None      => 16,
    };
    let src = &self.buffer[offset..];

    let image = match bps {
      12 => decode_12le(src, width as usize, height as usize),
      14 => decode_14le_unpacked(src, width as usize, height as usize),
      16 => decode_16le(src, width as usize, height as usize),
      _ => {return Err(format!("RAF: Don't know how to decode bps {}", bps).to_string());},
    };

    if camera.find_hint("fuji_rotation") {
      let (width, height, image) = RafDecoder::rotate_image(&image, camera, width as usize, height as usize);
      Ok(Image {
        make: camera.make.clone(),
        model: camera.model.clone(),
        canonical_make: camera.canonical_make.clone(),
        canonical_model: camera.canonical_model.clone(),
        width: width as usize,
        height: height as usize,
        wb_coeffs: try!(self.get_wb()),
        data: image.into_boxed_slice(),
        blacklevels: camera.blacklevels,
        whitelevels: camera.whitelevels,
        color_matrix: camera.color_matrix,
        cfa: camera.cfa.clone(),
        crops: [0,0,0,0],
      })
    } else {
      ok_image(camera, width, height, try!(self.get_wb()), image)
    }
  }
}

impl<'a> RafDecoder<'a> {
  fn get_wb(&self) -> Result<[f32;4], String> {
    match self.tiff.find_entry(Tag::RafWBGRB) {
      Some(levels) => Ok([levels.get_f32(1), levels.get_f32(0), levels.get_f32(2), NAN]),
      None => {
        let levels = fetch_tag!(self.tiff, Tag::RafOldWB);
        Ok([levels.get_f32(1), levels.get_f32(0), levels.get_f32(3), NAN])
      },
    }
  }

  fn rotate_image(src: &[u16], camera: &Camera, width: usize, height: usize) -> (usize, usize, Vec<u16>) {
    let x = camera.crops[3] as usize;
    let y = camera.crops[0] as usize;
    let cropwidth = width - (camera.crops[1] as usize) - x;
    let cropheight = height - (camera.crops[2] as usize) - y;

    let rotatedwidth = cropwidth + cropheight/2;
    let rotatedheight = rotatedwidth-1;

    // Doing the rotation by iterating the output instead of the input results in stranger
    // code that is ~30% faster including threading. In absolute terms it's not a large
    // improvement though so going back to the simpler code may make sense in case of bugs
    let out = decode_threaded(rotatedwidth, rotatedheight, &(|out: &mut [u16], row| {
      let startcol = if row < cropwidth { cropwidth - 1 - row } else { row + 1 - cropwidth };
      let endcol = if (row + cropwidth) < rotatedwidth {
        row + cropwidth
      } else {
        (rotatedwidth - 1) - (row + cropwidth - rotatedwidth) - 1
      };

      for (i,col) in (startcol..endcol+1).enumerate() {
        let (in_row, in_col) = if row < cropwidth {
          (y+i, (x+cropwidth-1) - row + i/2)
        } else {
          (y + (row-cropwidth+1)*2 + i, x + i/2)
        };
        out[col] = src[in_row*width + in_col];
      }
    }));

    (rotatedwidth, rotatedheight, out)
  }
}
