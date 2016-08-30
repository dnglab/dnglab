use decoders::*;
use decoders::tiff::*;
use decoders::basics::*;

#[derive(Debug, Clone)]
pub struct ArwDecoder<'a> {
  rawloader: &'a RawLoader,
  tiff: TiffIFD<'a>,
}

impl<'a> ArwDecoder<'a> {
  pub fn new(tiff: TiffIFD<'a>, rawloader: &'a RawLoader) -> ArwDecoder<'a> {
    ArwDecoder {
      tiff: tiff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for ArwDecoder<'a> {
  fn identify(&self) -> Result<&Camera, String> {
    let make = try!(self.tiff.find_entry(Tag::MAKE).ok_or("ARW: Couldn't find Make".to_string())).get_str();
    let model = try!(self.tiff.find_entry(Tag::MODEL).ok_or("ARW: Couldn't find Model".to_string())).get_str();
    self.rawloader.check_supported(make, model)
  }

  fn image(&self) -> Result<Image,String> {
    Err("ARW: Decoding not implemented yet!".to_string())
  }
}
