use super::{Camera, Decoder, FormatHint, RawDecodeParams, RawMetadata, ok_cfa_image};
use crate::Result;
use crate::analyze::FormatDump;
use crate::exif::Exif;
use crate::packed::{decode_10le_lsb16, decode_12be_msb16, decode_12le_16bitaligned};
use crate::rawsource::RawSource;
use crate::{RawImage, RawLoader, RawlerError};

#[derive(Debug, Clone)]
pub struct NakedDecoder<'a> {
  #[allow(dead_code)]
  rawloader: &'a RawLoader,
  camera: Camera,
}

impl<'a> NakedDecoder<'a> {
  pub fn new(camera: Camera, rawloader: &'a RawLoader) -> Result<NakedDecoder<'a>> {
    Ok(NakedDecoder { rawloader, camera })
  }
}

impl<'a> Decoder for NakedDecoder<'a> {
  fn raw_image(&self, file: &RawSource, _params: &RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let data = file.as_vec()?;
    let buffer = &data;
    let width = self.camera.raw_width;
    let height = self.camera.raw_height;
    let size = self.camera.filesize;
    let bits = size * 8 / width / height;

    let image = if self.camera.find_hint("12le_16bitaligned") {
      decode_12le_16bitaligned(buffer, width, height, dummy)
    } else {
      match bits {
        10 => decode_10le_lsb16(buffer, width, height, dummy),
        12 => decode_12be_msb16(buffer, width, height, dummy),
        _ => return Err(RawlerError::unsupported(&self.camera, format!("Naked: Don't know about {} bps images", bits))),
      }
    };
    let cpp = 1;
    ok_cfa_image(self.camera.clone(), cpp, [f32::NAN, f32::NAN, f32::NAN, f32::NAN], image, dummy)
  }

  fn format_dump(&self) -> FormatDump {
    todo!()
  }

  fn raw_metadata(&self, _file: &RawSource, _params: &RawDecodeParams) -> Result<RawMetadata> {
    let exif = Exif::default();
    let mdata = RawMetadata::new(&self.camera, exif);
    Ok(mdata)
  }

  fn format_hint(&self) -> FormatHint {
    FormatHint::Unknown
  }
}
