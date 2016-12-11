use decoders::*;
use decoders::tiff::*;
use decoders::basics::*;
use std::f32::NAN;

#[derive(Debug, Clone)]
pub struct DcsDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: TiffIFD<'a>,
}

impl<'a> DcsDecoder<'a> {
  pub fn new(buf: &'a [u8], tiff: TiffIFD<'a>, rawloader: &'a RawLoader) -> DcsDecoder<'a> {
    DcsDecoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for DcsDecoder<'a> {
  fn identify(&self) -> Result<&Camera, String> {
    let make = fetch_tag!(self.tiff, Tag::Make, "DCS: Couldn't find Make").get_str();
    let model = fetch_tag!(self.tiff, Tag::Model, "DCS: Couldn't find Model").get_str();
    self.rawloader.check_supported(make, model)
  }

  fn image(&self) -> Result<Image,String> {
    let camera = try!(self.identify());
    let data = self.tiff.find_ifds_with_tag(Tag::StripOffsets);
    let raw = data.iter().find(|&&ifd| {
      ifd.find_entry(Tag::ImageWidth).unwrap().get_u32(0) > 1000
    }).unwrap();
    let width = fetch_tag!(raw, Tag::ImageWidth, "DCS: Couldn't find width").get_u32(0);
    let height = fetch_tag!(raw, Tag::ImageLength, "DCS: Couldn't find height").get_u32(0);
    let offset = fetch_tag!(raw, Tag::StripOffsets, "DCS: Couldn't find offset").get_u32(0) as usize;
    let src = &self.buffer[offset .. self.buffer.len()];
    let linearization = fetch_tag!(self.tiff, Tag::GrayResponse, "DCS: Couldn't find linearization");
    let table = {
      let mut t: [u16;256] = [0;256];
      for i in 0..256 {
        t[i] = linearization.get_u32(i) as u16;
      }
      LookupTable::new(&t)
    };

    let image = decode_8bit_wtable(src, &table, width as usize, height as usize);
    ok_image(camera, width, height, [NAN,NAN,NAN,NAN], image)
  }
}
