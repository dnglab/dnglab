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
    let width = fetch_tag!(raw, Tag::RafImageWidth).get_u32(0);
    let height = fetch_tag!(raw, Tag::RafImageLength).get_u32(0);
    let offset = fetch_tag!(raw, Tag::RafOffsets).get_u32(0) as usize + raw.start_offset();
    let src = &self.buffer[offset..];

    let image = decode_12le(src, width as usize, height as usize);
    ok_image(camera, width, height, try!(self.get_wb()), image)
  }
}

impl<'a> RafDecoder<'a> {
  fn get_wb(&self) -> Result<[f32;4], String> {
    let levels = fetch_tag!(self.tiff, Tag::RafWBGRB);
    Ok([levels.get_f32(1), levels.get_f32(0), levels.get_f32(2), NAN])
  }
}
