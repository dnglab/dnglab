use decoders::*;
use decoders::tiff::*;
use decoders::basics::*;
use std::f32::NAN;

#[derive(Debug, Clone)]
pub struct SrwDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: TiffIFD<'a>,
}

impl<'a> SrwDecoder<'a> {
  pub fn new(buf: &'a [u8], tiff: TiffIFD<'a>, rawloader: &'a RawLoader) -> SrwDecoder<'a> {
    SrwDecoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for SrwDecoder<'a> {
  fn identify(&self) -> Result<&Camera, String> {
    let make = fetch_tag!(self.tiff, Tag::Make, "SRW: Couldn't find Make").get_str();
    let model = fetch_tag!(self.tiff, Tag::Model, "SRW: Couldn't find Model").get_str();
    self.rawloader.check_supported(make, model)
  }

  fn image(&self) -> Result<Image,String> {
    let camera = try!(self.identify());
    let data = self.tiff.find_ifds_with_tag(Tag::StripOffsets);
    let raw = data[0];
    let width = fetch_tag!(raw, Tag::ImageWidth, "SRW: Couldn't find width").get_u32(0);
    let height = fetch_tag!(raw, Tag::ImageLength, "SRW: Couldn't find height").get_u32(0);
    let offset = fetch_tag!(raw, Tag::StripOffsets, "SRW: Couldn't find offset").get_u32(0) as usize;
    let compression = fetch_tag!(raw, Tag::Compression, "SRW: Couldn't find compression").get_u32(0);
    let bits = fetch_tag!(raw, Tag::BitsPerSample, "SRW: Couldn't find bps").get_u32(0);
    let src = &self.buffer[offset .. self.buffer.len()];

    let image = match compression {
      32770 => {
        if self.tiff.find_entry(Tag::SrwSensorAreas).is_some() {
          match bits {
            12 => decode_12be(src, width as usize, height as usize),
            14 => decode_14le_unpacked(src, width as usize, height as usize),
             x => return Err(format!("SRW: Don't know how to handle bps {}", x).to_string()),
          }
        } else {
          panic!("compressed not implemented yet")
          //SrwDecoder::decode_srw1(src, width as usize, height as usize)
        }
      }
      x => return Err(format!("SRW: Don't know how to handle compression {}", x).to_string()),
    };

    ok_image(camera, width, height, try!(self.get_wb()), image)
  }
}

impl<'a> SrwDecoder<'a> {
//  pub fn decode_srw1(buf: &[u8], width: usize, height: usize) -> Vec<u16> {
//    let mut out: Vec<u16> = vec![0; (width*height) as usize];
//    out
//  }

  fn get_wb(&self) -> Result<[f32;4], String> {
    let rggb_levels = fetch_tag!(self.tiff, Tag::SrwRGGBLevels, "SRW: No RGGB Levels");
    let rggb_blacks = fetch_tag!(self.tiff, Tag::SrwRGGBBlacks, "SRW: No RGGB Blacks");
    if rggb_levels.count() != 4 || rggb_blacks.count() != 4 {
      Err("SRW: RGGB Levels and Blacks don't have 4 elements".to_string())
    } else {
      let nlevels = &rggb_levels.copy_offset_from_parent(&self.buffer);
      let nblacks = &rggb_blacks.copy_offset_from_parent(&self.buffer);
      Ok([nlevels.get_u32(0) as f32 - nblacks.get_u32(0) as f32,
          nlevels.get_u32(1) as f32 - nblacks.get_u32(1) as f32,
          nlevels.get_u32(3) as f32 - nblacks.get_u32(3) as f32,
          NAN])
    }
  }
}
