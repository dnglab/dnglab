use crate::RawImage;
use crate::RawLoader;
use crate::Result;
use crate::bits::*;
use crate::exif::Exif;
use crate::formats::tiff::Rational;
use crate::packed::*;
use crate::rawsource::RawSource;

use super::Camera;
use super::Decoder;
use super::FormatHint;
use super::RawDecodeParams;
use super::RawMetadata;
use super::ok_cfa_image;

pub fn is_ari(file: &RawSource) -> bool {
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
  pub fn new(file: &RawSource, rawloader: &'a RawLoader) -> Result<AriDecoder<'a>> {
    let buffer = file.subview(668, 30)?;
    let model = String::from_utf8_lossy(buffer).split_terminator('\0').next().unwrap_or("").to_string();
    let camera = rawloader.check_supported_with_everything("ARRI", &model, "")?;
    Ok(AriDecoder { rawloader, camera })
  }
}

impl<'a> Decoder for AriDecoder<'a> {
  fn raw_image(&self, file: &RawSource, _params: &RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let buffer = file.subview(0, 100)?;
    let offset = LEu32(buffer, ArriRawTag::DataOffset as usize) as usize;
    let width = LEu32(buffer, ArriRawTag::Width as usize) as usize;
    let height = LEu32(buffer, ArriRawTag::Height as usize) as usize;
    let src = file.subview_until_eof_padded(offset as u64)?;

    let image = if self.camera.find_hint("little-endian") {
      decode_12le(&src, width, height, dummy)
    } else {
      decode_12be_msb32(&src, width, height, dummy)
    };

    let cpp = 1;
    ok_cfa_image(self.camera.clone(), cpp, self.get_wb(file)?, image, dummy)
  }

  fn format_dump(&self) -> crate::analyze::FormatDump {
    todo!()
  }

  fn raw_metadata(&self, file: &RawSource, _params: &RawDecodeParams) -> Result<RawMetadata> {
    let mut exif = Exif::default();
    let buffer = file.subview(0, 0x0a98)?; // max header
    exif.recommended_exposure_index = Some(LEu32(buffer, ArriRawTag::ExposureIndexASA as usize));
    exif.sensitivity_type = Some(2);
    let lens_model = char_slice_to_string(&buffer[ArriRawTag::LensModel as usize..ArriRawTag::LensModel as usize + 32]);
    exif.lens_model = lens_model.map(|s| s.trim().into());
    log::debug!("Lens model: {:?}", exif.lens_model);
    let exposure_time = LEu32(buffer, ArriRawTag::ExposureTime as usize);
    exif.exposure_time = Some(Rational::new(exposure_time, 1000000));
    let focal_len = LEu32(buffer, ArriRawTag::LensFocalLen as usize);
    exif.focal_length = Some(Rational::new(focal_len, 1000));

    let mdata = RawMetadata::new(&self.camera, exif);
    Ok(mdata)
  }

  fn format_hint(&self) -> FormatHint {
    FormatHint::ARI
  }
}

impl<'a> AriDecoder<'a> {
  fn get_wb(&self, file: &RawSource) -> Result<[f32; 4]> {
    let buffer = file.subview(0, 100 + 12)?;
    Ok([LEf32(buffer, 100), LEf32(buffer, 104), LEf32(buffer, 108), f32::NAN])
  }
}

enum ArriRawTag {
  DataOffset = 0x0008,
  Width = 0x0014,
  Height = 0x0018,
  ExposureIndexASA = 0x0074,
  ExposureTime = 0x018C,
  LensFocalLen = 0x037C,
  LensModel = 0x0398,
}

fn char_slice_to_string(buf: &[u8]) -> Option<String> {
  Some(buf.iter().take_while(|&&c| c != 0).map(|&c| char::from(c)).collect())
}
