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
    let (width,height) = if raw.has_entry(Tag::RafImageWidth) {
      (fetch_tag!(raw, Tag::RafImageWidth).get_u32(0),
       fetch_tag!(raw, Tag::RafImageLength).get_u32(0))
    } else {
      let sizes = fetch_tag!(raw, Tag::ImageWidth);
      (sizes.get_u32(1), sizes.get_u32(0))
    };
    let offset = fetch_tag!(raw, Tag::RafOffsets).get_u32(0) as usize + raw.start_offset();
    let bps = match raw.find_entry(Tag::RafBitsPerSample) {
      Some(val) => val.get_u32(0),
      None      => 16,
    };
    let src = &self.buffer[offset..];

    let image = match bps {
      12 => decode_12le(src, width as usize, height as usize),
      14 => decode_14le_unpacked(src, width as usize, height as usize),
      16 => decode_16le(src, width as usize, height as usize),
      _ => {return Err(format!("RAF: Don't know how to decode bps {}", bps).to_string());},
    };
    ok_image(camera, width, height, try!(self.get_wb()), image)
  }
}

impl<'a> RafDecoder<'a> {
  fn get_wb(&self) -> Result<[f32;4], String> {
    match self.tiff.find_entry(Tag::RafWBGRB) {
      Some(levels) => Ok([levels.get_f32(1), levels.get_f32(0), levels.get_f32(2), NAN]),
      None => {
        let levels = fetch_tag!(self.tiff, Tag::RafOldWB);
        Ok([levels.get_f32(1), levels.get_f32(0), levels.get_f32(3), NAN])
      },
    }
  }
}
