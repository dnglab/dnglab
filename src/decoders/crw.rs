use decoders::*;
use decoders::ciff::*;

#[derive(Debug, Clone)]
pub struct CrwDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  ciff: CiffIFD<'a>,
}

impl<'a> CrwDecoder<'a> {
  pub fn new(buf: &'a [u8], ciff: CiffIFD<'a>, rawloader: &'a RawLoader) -> CrwDecoder<'a> {
    CrwDecoder {
      buffer: buf,
      ciff: ciff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for CrwDecoder<'a> {
  fn image(&self) -> Result<Image,String> {
    let makemodel = fetch_tag!(self.ciff, CiffTag::MakeModel).get_strings();
    if makemodel.len() < 2 {
      return Err("CRW: MakeModel tag needs to have 2 strings".to_string())
    }
    let camera = try!(self.rawloader.check_supported_with_everything(&makemodel[0], &makemodel[1], ""));

    Err("CRW is not done yet".to_string())
  }
}
