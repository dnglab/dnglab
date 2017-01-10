use decoders::*;
use decoders::tiff::*;
use decoders::ljpeg::*;
use std::f32::NAN;

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
  fn image(&self) -> Result<Image,String> {
    let camera = try!(self.rawloader.check_supported(&self.tiff));
    let raw = fetch_ifd!(&self.tiff, Tag::Cr2Id);
    let offset = fetch_tag!(raw, Tag::StripOffsets).get_usize(0);
    let src = &self.buffer[offset..];

    let (width, height, image) = {
      let decompressor = try!(LjpegDecompressor::new(src, true));
      let width = decompressor.width();
      let height = decompressor.height();
      let mut ljpegout = vec![0 as u16; width*height];
      try!(decompressor.decode(&mut ljpegout, 0, width, width, height));

      // Take each of the vertical fields and put them into the right location
      // FIXME: Doing this at the decode would reduce about 5% in runtime but I haven't
      //        been able to do it without hairy code
      let mut out = vec![0 as u16; width*height];
      let canoncol = fetch_tag!(raw, Tag::Cr2StripeWidths);
      let mut fieldwidths = Vec::new();
      for _ in 0..canoncol.get_usize(0) {
        fieldwidths.push(canoncol.get_usize(1));
      }
      fieldwidths.push(canoncol.get_usize(2));

      let mut fieldstart = 0;
      let mut fieldpos = 0;
      for fieldwidth in fieldwidths {
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

      (width, height, out)
    };
    ok_image(camera, width, height, try!(self.get_wb()), image)
  }
}

impl<'a> Cr2Decoder<'a> {
  fn get_wb(&self, cam: &Camera) -> Result<[f32;4], String> {
    let levels = fetch_tag!(self.tiff, Tag::Cr2ColorData);
    let offset = if cam.wb_offset != 0 {cam.wb_offset} else {63};
    Ok([levels.get_force_u16(offset) as f32, levels.get_force_u16(offset+1) as f32,
        levels.get_force_u16(offset+3) as f32, NAN])
  }
}
