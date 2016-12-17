use decoders::*;
use decoders::tiff::*;
use decoders::basics::*;
use std::f32::NAN;

#[derive(Debug, Clone)]
pub struct MefDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: TiffIFD<'a>,
}

impl<'a> MefDecoder<'a> {
  pub fn new(buf: &'a [u8], tiff: TiffIFD<'a>, rawloader: &'a RawLoader) -> MefDecoder<'a> {
    MefDecoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for MefDecoder<'a> {
  fn image(&self) -> Result<Image,String> {
    let camera = try!(self.rawloader.check_supported(&self.tiff));
    let data = self.tiff.find_ifds_with_tag(Tag::CFAPattern);
    let raw = data[0];
    let width = fetch_tag!(raw, Tag::ImageWidth).get_u32(0);
    let height = fetch_tag!(raw, Tag::ImageLength).get_u32(0);
    let offset = fetch_tag!(raw, Tag::StripOffsets).get_u32(0) as usize;
    let src = &self.buffer[offset .. self.buffer.len()];

    let image = decode_12be(src, width as usize, height as usize);
    ok_image(camera, width, height, [NAN,NAN,NAN,NAN] , image)
  }
}
