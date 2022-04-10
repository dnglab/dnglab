use std::f32::NAN;

use crate::bits::*;
use crate::exif::Exif;
use crate::packed::*;
use crate::OptBuffer;
use crate::RawFile;
use crate::RawImage;
use crate::RawLoader;
use crate::Result;

use super::ok_image;
use super::Camera;
use super::Decoder;
use super::RawDecodeParams;
use super::RawMetadata;

pub fn is_ari(file: &mut RawFile) -> bool {
  match file.subview(0, 4) {
    Ok(buf) => buf[0..4] == b"ARRI"[..],
    Err(_) => false,
  }
}

#[derive(Debug, Clone)]
pub struct AriDecoder<'a> {
  #[allow(unused)]
  rawloader: &'a RawLoader,
  camera: Camera,
}

impl<'a> AriDecoder<'a> {
  pub fn new(file: &mut RawFile, rawloader: &'a RawLoader) -> Result<AriDecoder<'a>> {
    let buffer = file.subview(668, 30).unwrap();
    let model = String::from_utf8_lossy(&buffer).split_terminator('\0').next().unwrap_or("").to_string();
    let camera = rawloader.check_supported_with_everything("ARRI", &model, "")?;
    Ok(AriDecoder { rawloader, camera })
  }
}

impl<'a> Decoder for AriDecoder<'a> {
  fn raw_image(&self, file: &mut RawFile, _params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let buffer = file.subview(0, 100).unwrap();
    let offset = LEu32(&buffer, 8) as usize;
    let width = LEu32(&buffer, 20) as usize;
    let height = LEu32(&buffer, 24) as usize;
    let src: OptBuffer = file.subview_until_eof(offset as u64).unwrap().into(); // TODO add size and check all samples

    //let image = decode_12be_msb32(&src, width, height, dummy);
    let image = decode_12be_msb32(&src, width, height, dummy);

    let cpp = 1;
    ok_image(self.camera.clone(), width, height, cpp, self.get_wb(file)?, image.into_inner())
  }

  fn format_dump(&self) -> crate::analyze::FormatDump {
    todo!()
  }

  fn raw_metadata(&self, _file: &mut RawFile, _params: RawDecodeParams) -> Result<RawMetadata> {
    let exif = Exif::default();
    let mdata = RawMetadata::new(&self.camera, exif);
    Ok(mdata)
  }
}

impl<'a> AriDecoder<'a> {
  fn get_wb(&self, file: &mut RawFile) -> Result<[f32; 4]> {
    let buffer = file.subview(0, 100 + 12).unwrap();
    Ok([LEf32(&buffer, 100), LEf32(&buffer, 104), LEf32(&buffer, 108), NAN])
  }
}
