use decoders::*;
use decoders::tiff::*;
use decoders::basics::*;
use decoders::ljpeg::*;
use std::f32::NAN;
use itertools::Itertools;

#[derive(Debug, Clone)]
pub struct Cr2Decoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: TiffIFD<'a>,
}

impl<'a> Cr2Decoder<'a> {
  pub fn new(buf: &'a [u8], tiff: TiffIFD<'a>, rawloader: &'a RawLoader) -> Cr2Decoder<'a> {
    Cr2Decoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for Cr2Decoder<'a> {
  fn image(&self) -> Result<RawImage,String> {
    let camera = try!(self.rawloader.check_supported(&self.tiff));
    let (raw, offset) = {
      if let Some(raw) = self.tiff.find_first_ifd(Tag::Cr2Id) {
        (raw, fetch_tag!(raw, Tag::StripOffsets).get_usize(0))
      } else if let Some(raw) = self.tiff.find_first_ifd(Tag::CFAPattern) {
        (raw, fetch_tag!(raw, Tag::StripOffsets).get_usize(0))
      } else if let Some(off) = self.tiff.find_entry(Tag::Cr2OldOffset) {
        (&self.tiff, off.get_usize(0))
      } else {
        return Err("CR2: Couldn't find raw info".to_string())
      }
    };
    let src = &self.buffer[offset..];

    let (width, height, cpp, image) = {
      let decompressor = try!(LjpegDecompressor::new(src));
      let ljpegwidth = decompressor.width();
      let mut width = ljpegwidth;
      let mut height = decompressor.height();
      let cpp = if decompressor.super_h() == 2 {3} else {1};
      let mut ljpegout = vec![0 as u16; width*height];
      try!(decompressor.decode(&mut ljpegout, 0, width, width, height));

      // Linearize the output (applies only to D2000 as far as I can tell)
      if camera.find_hint("linearization") {
        let table = {
          let linearization = fetch_tag!(self.tiff, Tag::GrayResponse);
          let mut t = [0 as u16;4096];
          for i in 0..t.len() {
            t[i] = linearization.get_u32(i) as u16;
          }
          LookupTable::new(&t)
        };

        let mut random = ljpegout[0] as u32;
        for o in ljpegout.chunks_exact_mut(1) {
          o[0] = table.dither(o[0], &mut random);
        }
      }

      // Convert the YUV in sRAWs to RGB
      if cpp == 3 {
        try!(self.convert_to_rgb(&camera, &mut ljpegout));
        if raw.has_entry(Tag::ImageWidth) {
          width = fetch_tag!(raw, Tag::ImageWidth).get_usize(0) * cpp;
          height = fetch_tag!(raw, Tag::ImageLength).get_usize(0) ;
        } else if width/cpp < height {
          let temp = width/cpp;
          width = height*cpp;
          height = temp;
        }
      } else if camera.find_hint("double_line") {
        width /= 2;
        height *= 2;
      }

      // Take each of the vertical fields and put them into the right location
      // FIXME: Doing this at the decode would reduce about 5% in runtime but I haven't
      //        been able to do it without hairy code
      if let Some(canoncol) = raw.find_entry(Tag::Cr2StripeWidths) {
        if canoncol.get_usize(0) == 0 {
          (width, height, cpp, ljpegout)
        } else {
          let mut out = vec![0 as u16; width*height];
          let mut fieldwidths = Vec::new();
          for _ in 0..canoncol.get_usize(0) {
            fieldwidths.push(canoncol.get_usize(1));
          }
          fieldwidths.push(canoncol.get_usize(2));

          if decompressor.super_v() == 2 {
            // We've decoded 2 lines at a time so we also need to copy two strips at a time
            let nfields = fieldwidths.len();
            let fieldwidth = fieldwidths[0];
            let mut fieldstart = 0;
            let mut inpos = 0;
            for _ in 0..nfields {
              for row in (0..height).step(2) {
                for col in (0..fieldwidth).step(3) {
                  let outpos = row*width+fieldstart+col;
                  out[outpos..outpos+3].copy_from_slice(&ljpegout[inpos..inpos+3]);
                  let outpos = (row+1)*width+fieldstart+col;
                  let inpos2 = inpos+ljpegwidth;
                  out[outpos..outpos+3].copy_from_slice(&ljpegout[inpos2..inpos2+3]);
                  inpos += 3;
                  if inpos % ljpegwidth == 0 {
                    // we've used a full input line and we're reading 2 by 2 so skip one
                    inpos += ljpegwidth;
                  }
                }
              }
              fieldstart += fieldwidth;
            }
          } else {
            let sh = decompressor.super_h();
            let mut fieldstart = 0;
            let mut fieldpos = 0;
            for fieldwidth in fieldwidths {
              let fieldwidth = fieldwidth/sh*cpp;
              for row in 0..height {
                let outpos = row*width+fieldstart;
                let inpos = fieldpos+row*fieldwidth;
                let outb = &mut out[outpos..outpos+fieldwidth];
                let inb = &ljpegout[inpos..inpos+fieldwidth];
                outb.copy_from_slice(inb);
              }
              fieldstart += fieldwidth;
              fieldpos += fieldwidth*height;
            }
          }

          (width, height, cpp, out)
        }
      } else {
        (width, height, cpp, ljpegout)
      }
    };

    let wb = self.get_wb(&camera)?;
    let mut img = RawImage::new(camera, width, height, wb, image);
    if cpp == 3 {
      img.cpp = 3;
      img.width /= 3;
      img.crops = [0,0,0,0];
      img.blacklevels = [0,0,0,0];
      img.whitelevels = [65535,65535,65535,65535];
    }
    Ok(img)
  }
}

impl<'a> Cr2Decoder<'a> {
  fn get_wb(&self, cam: &Camera) -> Result<[f32;4], String> {
    if let Some(levels) = self.tiff.find_entry(Tag::Cr2ColorData) {
      let offset = if cam.wb_offset != 0 {cam.wb_offset} else {63};
      Ok([levels.get_force_u16(offset) as f32, levels.get_force_u16(offset+1) as f32,
          levels.get_force_u16(offset+3) as f32, NAN])
    } else if let Some(levels) = self.tiff.find_entry(Tag::Cr2PowerShotWB) {
      Ok([levels.get_force_u32(3) as f32, levels.get_force_u32(2) as f32,
          levels.get_force_u32(4) as f32, NAN])
    } else if let Some(levels) = self.tiff.find_entry(Tag::Cr2OldWB) {
      Ok([levels.get_f32(0), levels.get_f32(1), levels.get_f32(2), NAN])
    } else {
      // At least the D2000 has no WB
      Ok([NAN,NAN,NAN,NAN])
    }
  }

  fn convert_to_rgb(&self, cam: &Camera, image: &mut [u16]) -> Result<(),String>{
    let coeffs = try!(self.get_wb(cam));
    let c1 = (1024.0*1024.0/coeffs[0]) as i32;
    let c2 = coeffs[1] as i32;
    let c3 = (1024.0*1024.0/coeffs[2]) as i32;

    let yoffset = if cam.find_hint("40d_yuv") { 512 } else { 0 };

    for pix in image.chunks_exact_mut(3) {
      let y = pix[0] as i32 - yoffset;
      let cb = pix[1] as i32 - 16383;
      let cr = pix[2] as i32 - 16383;

      let r = c1 * (y + cr);
      let g = c2 * (y + ((-778*cb - (cr<<11)) >> 12));
      let b = c3 * (y + cb);

      pix[0] = clampbits(r >> 8, 16) as u16;
      pix[1] = clampbits(g >> 8, 16) as u16;
      pix[2] = clampbits(b >> 8, 16) as u16;
    }
    Ok(())
  }
}
