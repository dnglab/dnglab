use decoders::*;
use decoders::tiff::*;
use decoders::basics::*;
use std::f32::NAN;

#[derive(Debug, Clone)]
pub struct OrfDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: TiffIFD<'a>,
}

impl<'a> OrfDecoder<'a> {
  pub fn new(buf: &'a [u8], tiff: TiffIFD<'a>, rawloader: &'a RawLoader) -> OrfDecoder<'a> {
    OrfDecoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for OrfDecoder<'a> {
  fn identify(&self) -> Result<&Camera, String> {
    let make = fetch_tag!(self.tiff, Tag::Make, "ORF: Couldn't find Make").get_str();
    let model = fetch_tag!(self.tiff, Tag::Model, "ORF: Couldn't find Model").get_str();
    self.rawloader.check_supported(make, model)
  }

  fn image(&self) -> Result<Image,String> {
    let camera = try!(self.identify());
    let data = self.tiff.find_ifds_with_tag(Tag::StripOffsets);
    let raw = data[0];
    let width = fetch_tag!(raw, Tag::ImageWidth, "ORF: Couldn't find width").get_u16(0) as u32;
    let height = fetch_tag!(raw, Tag::ImageLength, "ORF: Couldn't find height").get_u16(0) as u32;
    let offset = fetch_tag!(raw, Tag::StripOffsets, "ORF: Couldn't find offset").get_u32(0) as usize;
    let src = &self.buffer[offset .. self.buffer.len()];

    let image = decode_12be_interlaced(src, width as usize, height as usize);
    ok_image(camera, width, height, [NAN,NAN,NAN,NAN] , image)
  }
}
